use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};

use crate::disk::{Disk, OpenFile};
use crate::events::Events;
use crate::platform::skip_reason;
use crate::types::{
    ContentHash, DuplicateFile, DuplicateGroup, FileStamp, Independence, Progress, SkipReason,
};
use crate::walk::Indexed;

/// Bytes read at a time.
pub(crate) const BLOCK_SIZE: usize = 1024 * 1024;

#[derive(Debug, Default)]
pub(crate) struct Hashed {
    pub candidates: u64,
    pub files_hashed: u64,
    pub bytes_hashed: u64,
    pub groups: u64,
    pub reclaimable_bytes: u64,
}

pub(crate) fn find_groups(index: Vec<Indexed>, disk: &dyn Disk, events: &mut Events<'_>) -> Hashed {
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
        hashing.bucket(size, paths);
    }
    hashing.report();

    hashing.hashed
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
    events: &'a mut Events<'e>,
    /// One buffer for the whole phase. A fresh block per file would cost more
    /// than reading the small files themselves.
    buffer: Vec<u8>,
    tally: Tally,
    hashed: Hashed,
}

impl Hashing<'_, '_> {
    /// All the candidates of one size.
    fn bucket(&mut self, size: u64, paths: Vec<PathBuf>) {
        // Files with equal content, in the order the first of each was met.
        let mut classes: Vec<(ContentHash, Vec<DuplicateFile>)> = Vec::new();
        let mut class_of: HashMap<ContentHash, usize> = HashMap::new();

        for path in paths {
            let done_after = self.tally.bytes_done.saturating_add(size);

            match self.read(&path, done_after) {
                Ok((hash, stamp)) => {
                    let class = *class_of.entry(hash).or_insert_with(|| {
                        classes.push((hash, Vec::new()));
                        classes.len() - 1
                    });
                    classes[class].1.push(DuplicateFile {
                        paths: vec![path],
                        stamp,
                    });
                }
                Err(reason) => self.events.skip(path, reason),
            }

            self.tally.files_done += 1;
            self.tally.bytes_done = done_after;
            self.report();
        }

        for (hash, files) in classes {
            if files.len() > 1 {
                self.group(size, hash, files);
            }
        }
    }

    /// `bytes_done` never passes `ceiling`, so a file that grew since it was
    /// listed cannot push the progress past its total.
    fn read(&mut self, path: &Path, ceiling: u64) -> Result<(ContentHash, FileStamp), SkipReason> {
        let mut file = self.disk.open(path).map_err(refused)?;
        let stamp = file.stamp().map_err(refused)?;

        let tally = &mut self.tally;
        let events = &mut *self.events;
        let bytes_hashed = &mut self.hashed.bytes_hashed;
        let (hash, _) = hash_reader(file.as_mut(), &mut self.buffer, &mut |block| {
            *bytes_hashed += block;
            tally.bytes_done = tally.bytes_done.saturating_add(block).min(ceiling);
            events.progress(tally.progress());
        })
        .map_err(refused)?;

        self.hashed.files_hashed += 1;

        Ok((hash, stamp))
    }

    fn group(&mut self, size: u64, hash: ContentHash, files: Vec<DuplicateFile>) {
        let extra_copies = (files.len() as u64).saturating_sub(1);
        let reclaimable_bytes = size.saturating_mul(extra_copies);

        self.hashed.groups += 1;
        self.hashed.reclaimable_bytes = self
            .hashed
            .reclaimable_bytes
            .saturating_add(reclaimable_bytes);
        self.events.group(DuplicateGroup {
            size,
            hash,
            files,
            independence: Independence::Verified,
            reclaimable_bytes,
        });
    }

    fn report(&mut self) {
        self.events.progress(self.tally.progress());
    }
}

fn refused(err: io::Error) -> SkipReason {
    skip_reason(&err)
}

/// Calls `on_block` with the size of each piece as it is read. Returns the
/// hash and the number of bytes read.
pub(crate) fn hash_reader(
    file: &mut dyn OpenFile,
    buffer: &mut [u8],
    on_block: &mut dyn FnMut(u64),
) -> io::Result<(ContentHash, u64)> {
    debug_assert!(!buffer.is_empty(), "an empty buffer reads nothing");
    let mut hasher = blake3::Hasher::new();
    let mut total = 0_u64;

    loop {
        let read = file.read(buffer)?;
        if read == 0 {
            break;
        }

        hasher.update(&buffer[..read]);
        total += read as u64;
        on_block(read as u64);
    }

    Ok((ContentHash(*hasher.finalize().as_bytes()), total))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::disk::StdDisk;
    use crate::testkit::{collect, pattern};
    use crate::types::{Progress, ScanPolicy};

    fn hash_file(path: &Path) -> (ContentHash, u64, Vec<u64>) {
        let mut file = StdDisk.open(path).unwrap();
        let mut buffer = vec![0; BLOCK_SIZE];
        let mut blocks = Vec::new();

        let (hash, read) =
            hash_reader(file.as_mut(), &mut buffer, &mut |block| blocks.push(block)).unwrap();

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
}
