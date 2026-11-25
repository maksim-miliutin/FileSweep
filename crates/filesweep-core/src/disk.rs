//! The only door from the scanner to the file system. A test can stand in
//! front of the real disk and produce what a temporary directory cannot: a
//! refused folder, a file that changes while it is read, a cloud placeholder.

use std::ffi::OsStr;
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::platform::EntryFlags;
use crate::types::{FileIdentity, FileStamp, FileTimes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    File,
    Dir,
    Link,
    Special,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntryInfo {
    pub kind: EntryKind,
    /// Bytes, as the listing reports them. May lag behind the file itself.
    pub len: u64,
    pub flags: EntryFlags,
    pub times: FileTimes,
}

#[derive(Debug)]
pub(crate) struct Listed {
    pub path: PathBuf,
    /// An entry whose details could not be read keeps its own error, and the
    /// rest of the directory is still listed.
    pub info: io::Result<EntryInfo>,
}

pub(crate) trait Disk {
    /// The entries of `dir`, in no particular order. A link is reported as a
    /// link and never followed.
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<Listed>>;

    /// A root as the user named it. Follows links: naming a link is choosing
    /// what it points to.
    fn stat_root(&self, path: &Path) -> io::Result<EntryInfo>;

    /// Identifies the directory `dir` leads to, following links. `None` when
    /// the file system gives no usable identifier.
    fn dir_identity(&self, dir: &Path) -> Option<FileIdentity>;

    /// Opens for reading only.
    fn open(&self, path: &Path) -> io::Result<Box<dyn OpenFile>>;
}

pub(crate) trait OpenFile {
    fn stamp(&mut self) -> io::Result<FileStamp>;

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
}

pub(crate) struct StdDisk;

impl Disk for StdDisk {
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<Listed>> {
        let mut listed = Vec::new();

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            // DirEntry::metadata never follows a link, and on Windows it comes
            // from the listing itself, with no extra open per entry.
            let info = entry
                .metadata()
                .map(|meta| entry_info(&meta, &entry.file_name()));

            listed.push(Listed {
                path: entry.path(),
                info,
            });
        }

        Ok(listed)
    }

    fn stat_root(&self, path: &Path) -> io::Result<EntryInfo> {
        let meta = fs::metadata(path)?;

        Ok(EntryInfo {
            kind: kind_of(&meta),
            len: meta.len(),
            flags: EntryFlags::default(),
            times: times_of(&meta),
        })
    }

    fn dir_identity(&self, dir: &Path) -> Option<FileIdentity> {
        dir_identity_of(dir)
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn OpenFile>> {
        Ok(Box::new(StdFile(File::open(path)?)))
    }
}

struct StdFile(File);

impl OpenFile for StdFile {
    fn stamp(&mut self) -> io::Result<FileStamp> {
        let meta = self.0.metadata()?;
        let (identity, link_count) = identity_and_links(&self.0, &meta);

        Ok(FileStamp {
            identity,
            size: meta.len(),
            modified: meta.modified().ok(),
            link_count,
        })
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

fn entry_info(meta: &Metadata, name: &OsStr) -> EntryInfo {
    EntryInfo {
        kind: kind_of(meta),
        len: meta.len(),
        flags: flags_of(meta, name),
        times: times_of(meta),
    }
}

fn times_of(meta: &Metadata) -> FileTimes {
    FileTimes {
        modified: real_time(meta.modified()),
        accessed: real_time(meta.accessed()),
        created: real_time(meta.created()),
    }
}

// A time at or before 1970 stands for no time at all: Windows reads a zero
// FILETIME as 1601, and build tools stamp the files they make with zero.
fn real_time(time: io::Result<SystemTime>) -> Option<SystemTime> {
    time.ok().filter(|time| *time > SystemTime::UNIX_EPOCH)
}

fn kind_of(meta: &Metadata) -> EntryKind {
    let file_type = meta.file_type();

    if file_type.is_symlink() {
        EntryKind::Link
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Special
    }
}

#[cfg(unix)]
fn flags_of(_meta: &Metadata, name: &OsStr) -> EntryFlags {
    use std::os::unix::ffi::OsStrExt;

    EntryFlags {
        hidden: name.as_bytes().starts_with(b"."),
        ..EntryFlags::default()
    }
}

#[cfg(windows)]
fn flags_of(meta: &Metadata, _name: &OsStr) -> EntryFlags {
    use std::os::windows::fs::MetadataExt;

    crate::platform::flags_from_windows_attributes(meta.file_attributes())
}

#[cfg(unix)]
fn identity_and_links(_file: &File, meta: &Metadata) -> (Option<FileIdentity>, Option<u64>) {
    use std::os::unix::fs::MetadataExt;

    (
        crate::platform::identity_from_raw(meta.dev(), u128::from(meta.ino())),
        Some(meta.nlink()),
    )
}

// std keeps the volume serial number, the file index and the link count
// behind an unstable feature, so they come from winapi-util. A file system
// that cannot answer leaves the file without an identity, and the scan then
// calls its groups unverified instead of failing.
#[cfg(windows)]
fn identity_and_links(file: &File, _meta: &Metadata) -> (Option<FileIdentity>, Option<u64>) {
    match winapi_util::file::information(file) {
        Ok(info) => (
            crate::platform::identity_from_raw(
                info.volume_serial_number(),
                u128::from(info.file_index()),
            ),
            Some(info.number_of_links()),
        ),
        Err(_) => (None, None),
    }
}

#[cfg(unix)]
fn dir_identity_of(dir: &Path) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let meta = fs::metadata(dir).ok()?;

    crate::platform::identity_from_raw(meta.dev(), u128::from(meta.ino()))
}

// Costs one extra open per folder: a directory handle is the only way to its
// identity. The open follows links, so the answer is for what `dir` leads to.
#[cfg(windows)]
fn dir_identity_of(dir: &Path) -> Option<FileIdentity> {
    let handle = winapi_util::Handle::from_path_any(dir).ok()?;
    let info = winapi_util::file::information(&handle).ok()?;

    crate::platform::identity_from_raw(info.volume_serial_number(), u128::from(info.file_index()))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use super::*;

    fn info_of(listed: &[Listed], name: &str) -> EntryInfo {
        let entry = listed
            .iter()
            .find(|entry| entry.path.file_name().is_some_and(|found| found == name))
            .unwrap_or_else(|| panic!("{name} is missing from the listing"));

        *entry
            .info
            .as_ref()
            .expect("the listing has details for the entry")
    }

    #[test]
    fn std_disk_reports_kind_and_length() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("five.bin"), b"12345").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("five.bin", dir.path().join("link")).unwrap();

        let listed = StdDisk.list_dir(dir.path()).unwrap();

        let file = info_of(&listed, "five.bin");
        assert_eq!((file.kind, file.len), (EntryKind::File, 5));
        assert_eq!(info_of(&listed, "sub").kind, EntryKind::Dir);
        #[cfg(unix)]
        assert_eq!(info_of(&listed, "link").kind, EntryKind::Link);

        let five = dir.path().join("five.bin");
        assert!(listed.iter().any(|entry| entry.path == five));
    }

    #[test]
    fn std_disk_reports_the_times_a_file_carries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.txt");
        fs::write(&path, b"old").unwrap();
        // 1 January 2020 and 1 June 2021, whole seconds, so every file
        // system keeps them exactly.
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_577_836_800);
        let accessed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_622_505_600);
        let file = fs::File::options().write(true).open(&path).unwrap();
        file.set_times(
            fs::FileTimes::new()
                .set_modified(modified)
                .set_accessed(accessed),
        )
        .unwrap();
        drop(file);

        let listed = StdDisk.list_dir(dir.path()).unwrap();
        let times = info_of(&listed, "old.txt").times;

        assert_eq!(times.modified, Some(modified));
        assert_eq!(times.accessed, Some(accessed));
    }

    #[test]
    fn a_time_at_or_before_1970_is_no_time() {
        let epoch = SystemTime::UNIX_EPOCH;
        let second = Duration::from_secs(1);

        assert_eq!(real_time(Ok(epoch)), None);
        assert_eq!(real_time(Ok(epoch - second)), None);
        assert_eq!(real_time(Err(io::Error::other("no such time"))), None);
        assert_eq!(real_time(Ok(epoch + second)), Some(epoch + second));
    }

    #[test]
    fn stat_root_tells_a_folder_from_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file.bin");
        fs::write(&file, b"x").unwrap();

        assert_eq!(StdDisk.stat_root(dir.path()).unwrap().kind, EntryKind::Dir);
        assert_eq!(StdDisk.stat_root(&file).unwrap().kind, EntryKind::File);

        let missing = StdDisk.stat_root(&dir.path().join("missing"));
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn stat_root_follows_a_link_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("alias")).unwrap();

        let alias = StdDisk.stat_root(&dir.path().join("alias")).unwrap();
        assert_eq!(alias.kind, EntryKind::Dir);
    }

    #[cfg(unix)]
    #[test]
    fn std_disk_marks_dotfiles_hidden_on_unix() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".secret"), b"x").unwrap();
        fs::write(dir.path().join("plain"), b"x").unwrap();

        let listed = StdDisk.list_dir(dir.path()).unwrap();

        assert!(info_of(&listed, ".secret").flags.hidden);
        assert!(!info_of(&listed, "plain").flags.hidden);
    }

    #[test]
    fn hard_links_share_identity_and_report_link_count() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), b"same").unwrap();
        fs::hard_link(dir.path().join("a"), dir.path().join("b")).unwrap();
        fs::write(dir.path().join("c"), b"same").unwrap();

        let stamp = |name: &str| {
            let mut file = StdDisk.open(&dir.path().join(name)).unwrap();
            file.stamp().unwrap()
        };
        let (a, b, c) = (stamp("a"), stamp("b"), stamp("c"));

        assert!(a.identity.is_some());
        assert_eq!(a.identity, b.identity);
        assert_ne!(a.identity, c.identity);
        assert_eq!(
            (a.link_count, b.link_count, c.link_count),
            (Some(2), Some(2), Some(1))
        );
    }

    #[cfg(unix)]
    #[test]
    fn dir_identity_follows_links() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        let alias = dir.path().join("alias");
        let other = dir.path().join("other");
        fs::create_dir(&real).unwrap();
        fs::create_dir(&other).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();

        let identity = StdDisk.dir_identity(&real);

        assert!(identity.is_some());
        assert_eq!(StdDisk.dir_identity(&alias), identity);
        assert_ne!(StdDisk.dir_identity(&other), identity);
        assert_eq!(StdDisk.dir_identity(&dir.path().join("missing")), None);
    }

    #[test]
    fn dir_identity_tells_folders_apart() {
        let dir = tempfile::tempdir().unwrap();
        let (one, other) = (dir.path().join("one"), dir.path().join("other"));
        fs::create_dir(&one).unwrap();
        fs::create_dir(&other).unwrap();

        let identity = StdDisk.dir_identity(&one);

        assert!(identity.is_some());
        assert_eq!(StdDisk.dir_identity(&one), identity);
        assert_ne!(StdDisk.dir_identity(&other), identity);
        assert_eq!(StdDisk.dir_identity(&dir.path().join("missing")), None);
    }

    #[test]
    fn read_returns_the_bytes_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.bin");
        let content: Vec<u8> = (0..10_000_u32).map(|i| (i % 251) as u8).collect();
        fs::write(&path, &content).unwrap();

        let mut file = StdDisk.open(&path).unwrap();
        let mut read_back = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = file.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            read_back.extend_from_slice(&buffer[..read]);
        }

        assert_eq!(read_back, content);
        assert_eq!(file.stamp().unwrap().size, 10_000);
    }
}
