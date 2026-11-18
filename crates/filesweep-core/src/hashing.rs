use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use crate::control::{Flow, ScanControl};
use crate::disk::{Disk, OpenFile};
use crate::events::Events;
use crate::platform::skip_reason;
use crate::types::{
    ContentHash, DuplicateFile, DuplicateGroup, FileIdentity, FileStamp, Independence, Progress,
    SkipReason,
};
use crate::walk::Indexed;

/// Bytes read at a time. Cancel and pause are obeyed between two blocks, so
/// this is also the most that is read after a cancel.
pub(crate) const BLOCK_SIZE: usize = 1024 * 1024;

#[derive(Debug, Default)]
pub(crate) struct Hashed {
    pub candidates: u64,
    pub files_hashed: u64,
    pub bytes_hashed: u64,
    pub groups: u64,
    pub reclaimable_bytes: u64,
    pub cancelled: bool,
}

pub(crate) fn find_groups(
    index: Vec<Indexed>,
    disk: &dyn Disk,
    control: &ScanControl,
    events: &mut Events<'_>,
) -> Hashed {
    // A size seen once cannot have a twin, so that file is never opened.
    let mut by_size: BTreeMap<u64, Vec<PathBuf>> = BTreeMap::new();
    for file in index {
        by_size.entry(file.size).or_default().push(file.path);
    }
    by_size.retain(|_, paths| paths.len() > 1);

    let mut tally = Tally::default();
    for (size, paths) in &by_size {
        let count = paths.len() as u64;
        tally.files_total += count;
        tally.bytes_total = tally.bytes_total.saturating_add(size.saturating_mul(count));
    }

    let mut hashing = Hashing {
        disk,
        control,
        events,
        buffer: vec![0; BLOCK_SIZE],
        tally,
        hashed: Hashed {
            candidates: tally.files_total,
            ..Hashed::default()
        },
    };

    hashing.report();
    // Largest first: the big duplicates are what the user came for.
    for (size, paths) in by_size.into_iter().rev() {
        if hashing.bucket(size, paths).is_err() {
            // Nothing more is reported after a cancel.
            hashing.hashed.cancelled = true;
            return hashing.hashed;
        }
    }
    hashing.report();

    hashing.hashed
}

struct Cancelled;

enum Unread {
    Skip(SkipReason),
    Cancelled,
}

impl From<SkipReason> for Unread {
    fn from(reason: SkipReason) -> Self {
        Self::Skip(reason)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Tally {
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
}

impl Tally {
    fn progress(self) -> Progress {
        Progress::Hashing {
            files_done: self.files_done,
            files_total: self.files_total,
            bytes_done: self.bytes_done,
            bytes_total: self.bytes_total,
        }
    }
}

struct Hashing<'a, 'e> {
    disk: &'a dyn Disk,
    control: &'a ScanControl,
    events: &'a mut Events<'e>,
    /// One buffer for the whole phase. A fresh block per file would cost more
    /// than reading the small files themselves.
    buffer: Vec<u8>,
    tally: Tally,
    hashed: Hashed,
}

impl Hashing<'_, '_> {
    /// All the candidates of one size. A cancel leaves the bucket without
    /// groups: a group is reported only for a bucket that was read to its end.
    fn bucket(&mut self, size: u64, paths: Vec<PathBuf>) -> Result<(), Cancelled> {
        let files = self.physical_files(size, paths)?;

        // One physical file has nothing to be compared with, however many
        // names led to it.
        if files.len() < 2 {
            for _ in &files {
                self.done(size);
            }
            return Ok(());
        }

        // Files with equal content, in the order the first of each was met.
        let mut classes: Vec<(ContentHash, Vec<DuplicateFile>)> = Vec::new();
        let mut class_of: HashMap<ContentHash, usize> = HashMap::new();

        for mut file in files {
            self.checkpoint()?;
            let start = self.tally.bytes_done;

            match self.read(&file.paths[0], &file.stamp, start.saturating_add(size)) {
                Ok((hash, after_read)) => {
                    file.stamp = after_read;
                    let class = *class_of.entry(hash).or_insert_with(|| {
                        classes.push((hash, Vec::new()));
                        classes.len() - 1
                    });
                    classes[class].1.push(file);
                }
                Err(Unread::Skip(reason)) => self.events.skip(file.paths[0].clone(), reason),
                Err(Unread::Cancelled) => return Err(Cancelled),
            }

            self.tally.bytes_done = start;
            self.done(size);
        }

        for (hash, files) in classes {
            if files.len() > 1 {
                self.group(size, hash, files);
            }
        }

        Ok(())
    }

    /// Pass one: learns which physical file each candidate is. The names of
    /// one file are folded together, and a path the index holds twice is taken
    /// once. Only one file is open at a time, however large the bucket.
    fn physical_files(
        &mut self,
        size: u64,
        paths: Vec<PathBuf>,
    ) -> Result<Vec<DuplicateFile>, Cancelled> {
        let first_mention: Vec<bool> = {
            let mut seen = HashSet::with_capacity(paths.len());
            paths
                .iter()
                .map(|path| seen.insert(path.as_path()))
                .collect()
        };

        let mut files: Vec<DuplicateFile> = Vec::new();
        let mut file_of: HashMap<FileIdentity, usize> = HashMap::new();

        for (path, first_mention) in paths.into_iter().zip(first_mention) {
            self.checkpoint()?;

            if !first_mention {
                self.done(size);
                continue;
            }

            let stamp = match self.stamp(&path, size) {
                Ok(stamp) => stamp,
                Err(reason) => {
                    self.events.skip(path, reason);
                    self.done(size);
                    continue;
                }
            };

            let known = stamp
                .identity
                .and_then(|identity| file_of.get(&identity).copied());
            match known {
                Some(file) => {
                    files[file].paths.push(path);
                    self.done(size);
                }
                None => {
                    if let Some(identity) = stamp.identity {
                        file_of.insert(identity, files.len());
                    }
                    files.push(DuplicateFile {
                        paths: vec![path],
                        stamp,
                    });
                }
            }
        }

        Ok(files)
    }

    fn stamp(&self, path: &Path, listed_size: u64) -> Result<FileStamp, SkipReason> {
        let mut file = self.disk.open(path).map_err(refused)?;
        let stamp = file.stamp().map_err(refused)?;

        // The listing can lag behind the file, and the path can lead to
        // another file by now. Either way this is not the file that was listed.
        if stamp.size != listed_size {
            return Err(SkipReason::Changed);
        }

        Ok(stamp)
    }

    /// Pass two: hashes one physical file. The handle is stamped before and
    /// after the read: the first stamp must agree with the first look at the
    /// file, the second with the first, and the bytes read with the size.
    /// Otherwise the hash may describe bytes that were never on disk together.
    ///
    /// `bytes_done` never passes `ceiling`, so a file that grew cannot push the
    /// progress past its total.
    fn read(
        &mut self,
        path: &Path,
        first_look: &FileStamp,
        ceiling: u64,
    ) -> Result<(ContentHash, FileStamp), Unread> {
        let mut file = self.disk.open(path).map_err(refused)?;

        let before = file.stamp().map_err(refused)?;
        if !same_content_state(first_look, &before) {
            return Err(SkipReason::Changed.into());
        }

        let tally = &mut self.tally;
        let events = &mut *self.events;
        let bytes_hashed = &mut self.hashed.bytes_hashed;
        let hashed = hash_reader(
            file.as_mut(),
            &mut self.buffer,
            self.control,
            &mut |block| {
                *bytes_hashed += block;
                tally.bytes_done = tally.bytes_done.saturating_add(block).min(ceiling);
                events.progress(tally.progress());
            },
        )
        .map_err(refused)?;
        let Some((hash, read)) = hashed else {
            return Err(Unread::Cancelled);
        };

        let after = file.stamp().map_err(refused)?;
        if read != before.size || !same_content_state(&before, &after) {
            return Err(SkipReason::Changed.into());
        }

        self.hashed.files_hashed += 1;

        Ok((hash, after))
    }

    fn group(&mut self, size: u64, hash: ContentHash, files: Vec<DuplicateFile>) {
        let (independence, reclaimable_bytes) = reclaimable(size, &files);

        self.hashed.groups += 1;
        self.hashed.reclaimable_bytes = self
            .hashed
            .reclaimable_bytes
            .saturating_add(reclaimable_bytes);
        self.events.group(DuplicateGroup {
            size,
            hash,
            files,
            independence,
            reclaimable_bytes,
        });
    }

    fn done(&mut self, size: u64) {
        self.tally.files_done += 1;
        self.tally.bytes_done = self.tally.bytes_done.saturating_add(size);
        self.report();
    }

    fn checkpoint(&self) -> Result<(), Cancelled> {
        match self.control.checkpoint() {
            Flow::Go => Ok(()),
            Flow::Stop => Err(Cancelled),
        }
    }

    fn report(&mut self) {
        self.events.progress(self.tally.progress());
    }
}

fn refused(err: io::Error) -> SkipReason {
    skip_reason(&err)
}

/// The number of names is left out: a new name for a file is not a change of
/// what is in it.
fn same_content_state(a: &FileStamp, b: &FileStamp) -> bool {
    a.identity == b.identity && a.size == b.size && a.modified == b.modified
}

/// The least that removing the extra copies would give back. One file always
/// stays. A file with a name outside the scan keeps its bytes on disk after
/// every name the scan knows is gone, so it gives nothing back. And unless
/// every file has an identity, two of them may be one file: then nothing is
/// promised at all.
fn reclaimable(size: u64, files: &[DuplicateFile]) -> (Independence, u64) {
    if files.iter().any(|file| file.stamp.identity.is_none()) {
        return (Independence::Unverified, 0);
    }

    let seen_in_full = files
        .iter()
        .filter(|file| file.stamp.link_count == Some(file.paths.len() as u64))
        .count() as u64;
    let removable = seen_in_full.min((files.len() as u64).saturating_sub(1));

    (Independence::Verified, size.saturating_mul(removable))
}

/// Calls `on_block` with the size of each piece as it is read. Returns the
/// hash and the number of bytes read, or `None` when the scan was cancelled
/// before the end of the file.
pub(crate) fn hash_reader(
    file: &mut dyn OpenFile,
    buffer: &mut [u8],
    control: &ScanControl,
    on_block: &mut dyn FnMut(u64),
) -> io::Result<Option<(ContentHash, u64)>> {
    debug_assert!(!buffer.is_empty(), "an empty buffer reads nothing");
    let mut hasher = blake3::Hasher::new();
    let mut total = 0_u64;

    loop {
        if control.checkpoint() == Flow::Stop {
            return Ok(None);
        }

        let read = file.read(buffer)?;
        if read == 0 {
            break;
        }

        hasher.update(&buffer[..read]);
        total += read as u64;
        on_block(read as u64);
    }

    Ok(Some((ContentHash(*hasher.finalize().as_bytes()), total)))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::disk::StdDisk;
    use crate::testkit::{Faulty, collect, pattern};
    use crate::types::{Progress, ScanPolicy};

    fn hash_file(path: &Path) -> (ContentHash, u64, Vec<u64>) {
        let mut file = StdDisk.open(path).unwrap();
        let mut buffer = vec![0; BLOCK_SIZE];
        let mut blocks = Vec::new();

        let (hash, read) = hash_reader(
            file.as_mut(),
            &mut buffer,
            &ScanControl::new(),
            &mut |block| blocks.push(block),
        )
        .unwrap()
        .expect("nothing cancelled this read");

        (hash, read, blocks)
    }

    #[test]
    fn blake3_of_empty_input_matches_published_vector() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        fs::write(&path, b"").unwrap();

        let (hash, read, blocks) = hash_file(&path);

        assert_eq!(
            hash.to_string(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(read, 0);
        assert!(blocks.is_empty());
    }

    #[test]
    fn reading_in_blocks_gives_the_hash_of_the_whole_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long");
        let content = pattern(2 * BLOCK_SIZE + 10);
        fs::write(&path, &content).unwrap();

        let (hash, read, blocks) = hash_file(&path);

        assert_eq!(hash.0, *blake3::hash(&content).as_bytes());
        assert_eq!(read, content.len() as u64);
        assert_eq!(blocks.iter().sum::<u64>(), content.len() as u64);
        assert!(blocks.len() >= 3, "blocks: {blocks:?}");
    }

    #[test]
    fn sizes_around_the_block_boundary_group_only_true_copies() {
        let dir = tempfile::tempdir().unwrap();
        let sizes = [
            2 * BLOCK_SIZE + 1,
            BLOCK_SIZE + 1,
            BLOCK_SIZE,
            BLOCK_SIZE - 1,
            1,
        ];
        for size in sizes {
            let content = pattern(size);
            let mut altered = content.clone();
            altered[size - 1] ^= 0xFF;
            fs::write(dir.path().join(format!("{size}-a")), &content).unwrap();
            fs::write(dir.path().join(format!("{size}-b")), &content).unwrap();
            fs::write(dir.path().join(format!("{size}-c")), &altered).unwrap();
        }

        let seen = collect(&StdDisk, &[dir.path()], ScanPolicy::default());

        let found: Vec<(u64, usize)> = seen
            .groups
            .iter()
            .map(|group| (group.size, group.files.len()))
            .collect();
        let expected: Vec<(u64, usize)> = sizes.iter().map(|size| (*size as u64, 2)).collect();
        assert_eq!(found, expected);
    }

    // Guards the promise that a group is a full match: a scanner that compared
    // only the first block would call these three files copies of each other.
    #[test]
    fn difference_in_the_second_block_makes_no_group() {
        let dir = tempfile::tempdir().unwrap();
        let content = pattern(2 * BLOCK_SIZE);
        let mut at_block_start = content.clone();
        at_block_start[BLOCK_SIZE] ^= 0xFF;
        let mut mid_block = content.clone();
        mid_block[BLOCK_SIZE + BLOCK_SIZE / 2] ^= 0xFF;
        fs::write(dir.path().join("original"), &content).unwrap();
        fs::write(dir.path().join("differs-at-block-start"), &at_block_start).unwrap();
        fs::write(dir.path().join("differs-mid-block"), &mid_block).unwrap();

        let seen = collect(&StdDisk, &[dir.path()], ScanPolicy::default());

        assert!(seen.groups.is_empty(), "groups: {:?}", seen.groups);
        assert_eq!(seen.summary.files_hashed, 3);
    }

    #[test]
    fn a_long_read_reports_progress_before_the_file_is_done() {
        let dir = tempfile::tempdir().unwrap();
        let content = pattern(3 * BLOCK_SIZE);
        fs::write(dir.path().join("one"), &content).unwrap();
        fs::write(dir.path().join("two"), &content).unwrap();

        let seen = collect(&StdDisk, &[dir.path()], ScanPolicy::default());

        let total = 2 * content.len() as u64;
        let mid_file = seen.progress.iter().any(|step| {
            matches!(step, Progress::Hashing { files_done: 0, bytes_done, .. } if *bytes_done > 0)
        });
        assert!(mid_file, "no progress while the first file was being read");
        assert!(seen.progress.iter().all(|step| match step {
            Progress::Hashing { bytes_done, .. } => *bytes_done <= total,
            Progress::Walking { .. } => true,
        }));
    }

    fn two_copies() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.bin"), dir.path().join("b.bin"));
        fs::write(&a, pattern(1000)).unwrap();
        fs::write(&b, pattern(1000)).unwrap();

        (dir, a, b)
    }

    #[test]
    fn unknown_identity_makes_the_group_unverified_with_zero_reclaimable() {
        let (dir, a, _) = two_copies();
        let disk = Faulty::new(StdDisk).no_identity(&a);

        let seen = collect(&disk, &[dir.path()], ScanPolicy::default());

        assert_eq!(seen.groups.len(), 1);
        let group = &seen.groups[0];
        assert_eq!(group.files.len(), 2);
        assert_eq!(group.files[0].stamp.identity, None);
        assert_eq!(group.independence, Independence::Unverified);
        assert_eq!(group.reclaimable_bytes, 0);
        assert_eq!(seen.summary.reclaimable_bytes, 0);
    }

    // Guards against a file becoming its own duplicate. With no folder
    // identity the walk cannot tell it entered one folder twice, and with no
    // file identity the two index entries of a path look like two files.
    #[test]
    fn a_path_indexed_twice_is_one_candidate() {
        let (dir, a, b) = two_copies();
        let disk = Faulty::new(StdDisk)
            .no_identity(dir.path())
            .no_identity(&a)
            .no_identity(&b);

        let seen = collect(&disk, &[dir.path(), dir.path()], ScanPolicy::default());

        assert_eq!(seen.summary.files_seen, 4);
        assert_eq!(seen.groups.len(), 1);
        let names: Vec<&[PathBuf]> = seen.groups[0]
            .files
            .iter()
            .map(|file| file.paths.as_slice())
            .collect();
        assert_eq!(names, [[a], [b]]);
    }

    #[test]
    fn a_file_listed_twice_with_no_other_twin_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let only = dir.path().join("only.bin");
        fs::write(&only, pattern(1000)).unwrap();
        let disk = Faulty::new(StdDisk)
            .no_identity(dir.path())
            .no_identity(&only);

        let seen = collect(&disk, &[dir.path(), dir.path()], ScanPolicy::default());

        assert!(seen.groups.is_empty(), "groups: {:?}", seen.groups);
        assert_eq!(seen.summary.files_hashed, 0);
    }

    #[test]
    fn large_bucket_keeps_at_most_two_files_open() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..300_u32 {
            let content = [(i % 3) as u8; 64];
            fs::write(dir.path().join(format!("{i:03}")), content).unwrap();
        }
        let disk = Faulty::new(StdDisk);

        let seen = collect(&disk, &[dir.path()], ScanPolicy::default());

        assert_eq!(seen.summary.files_hashed, 300);
        assert_eq!(seen.groups.len(), 3);
        assert!(
            (1..=2).contains(&disk.max_open()),
            "open at once: {}",
            disk.max_open()
        );
    }
}
