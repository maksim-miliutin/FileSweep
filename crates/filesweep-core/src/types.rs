use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRequest {
    /// Folders to scan, walked in this order. A root is scanned even when it
    /// is hidden or a link: the caller named it on purpose.
    pub roots: Vec<PathBuf>,
    pub policy: ScanPolicy,
}

/// Files whose data is not on this device are always left out and have no
/// switch: reading them downloads them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanPolicy {
    pub include_hidden: bool,
    pub include_system: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    Progress(Progress),
    Skipped(Skip),
    Group(DuplicateGroup),
    /// Once for every file the walk puts into the index, before any hashing.
    /// Empty and skipped files are not sent.
    Indexed(IndexedFile),
}

/// Counters never go down. Each phase reports when it starts and when it
/// ends; how often in between is not promised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Walking {
        files_seen: u64,
    },
    /// Candidates that were skipped or turned out to be one file count as
    /// done, so the phase ends with `bytes_done == bytes_total`. The bytes
    /// really read are in the summary.
    Hashing {
        files_done: u64,
        files_total: u64,
        bytes_done: u64,
        bytes_total: u64,
    },
}

/// A regular file as the listing of its folder describes it. Nothing about it
/// was read from the file itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedFile {
    pub path: PathBuf,
    /// Bytes, from the listing.
    pub size: u64,
    pub times: FileTimes,
}

/// When a file was last changed, last opened and made, as the listing reports
/// it. `None`: the file system gives no such time.
///
/// Good enough to sort by, not to act on. On Windows the listing gives the
/// copy NTFS keeps in the folder entry, which can lag behind the file, most of
/// all for a file with more than one name, and Windows often does not record
/// opening at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileTimes {
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub created: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    pub path: PathBuf,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Symlink, junction or mount point. Never followed.
    Link,
    OnlineOnly,
    System,
    Hidden,
    /// Device, pipe or socket.
    Special,
    NotADirectory,
    /// A directory already visited through another root or another name.
    AlreadyCovered,
    AccessDenied,
    InUse,
    Vanished,
    /// Differs from what was listed, or changed while it was read.
    Changed,
    Io(IoFailure),
}

/// Confirmed by a hash of the whole content, never by size alone. Holds at
/// least two different physical files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    /// Bytes of content in each file.
    pub size: u64,
    pub hash: ContentHash,
    /// In the order the walk met them.
    pub files: Vec<DuplicateFile>,
    pub independence: Independence,
    /// A cautious estimate of what removing the extra copies would free.
    /// Zero when independence is unverified.
    pub reclaimable_bytes: u64,
}

/// One physical file, under every name the scan found for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateFile {
    pub paths: Vec<PathBuf>,
    pub stamp: FileStamp,
}

/// The state of a file as read from its open handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    /// `None`: the file system gave no usable identifier.
    pub identity: Option<FileIdentity>,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// All names of the file on disk, including those outside the scan.
    pub link_count: Option<u64>,
}

/// Names one physical file on this machine for as long as the file exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    pub volume: u64,
    pub file: u128,
}

/// BLAKE3 of the whole content. Displays as lowercase hex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash(pub [u8; 32]);

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }

        Ok(())
    }
}

/// Whether the files of a group are known to be separate files on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Independence {
    Verified,
    /// At least one file had no identifier, so two entries may be one file.
    Unverified,
}

/// An OS error, kept as data so events stay comparable and cloneable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoFailure {
    pub kind: io::ErrorKind,
    pub os_code: Option<i32>,
    pub message: String,
}

impl IoFailure {
    pub(crate) fn from_error(err: &io::Error) -> Self {
        Self {
            kind: err.kind(),
            os_code: err.raw_os_error(),
            message: err.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSummary {
    pub outcome: Outcome,
    /// Regular files indexed by the walk. Empty files are counted apart.
    pub files_seen: u64,
    /// Zero-length files: counted, never grouped.
    pub empty_files: u64,
    /// Paths whose size matched another path.
    pub candidates: u64,
    /// Physical files read in full.
    pub files_hashed: u64,
    pub bytes_hashed: u64,
    pub groups: u64,
    pub reclaimable_bytes: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    /// Groups reported before the cancel stay valid: each was fully confirmed.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    NoRoots,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRoots => f.write_str("the scan request names no root folder"),
        }
    }
}

impl std::error::Error for RequestError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_displays_lowercase_hex() {
        assert_eq!(ContentHash([0xab; 32]).to_string(), "ab".repeat(32));

        let mut bytes = [0u8; 32];
        bytes[0] = 0x0f;
        let text = ContentHash(bytes).to_string();
        assert_eq!(text.len(), 64);
        assert!(
            text.starts_with("0f00"),
            "a leading zero was dropped: {text}"
        );
    }

    #[test]
    fn scan_policy_defaults_hide_hidden_and_system() {
        let policy = ScanPolicy::default();
        assert!(!policy.include_hidden);
        assert!(!policy.include_system);
    }

    #[test]
    fn io_failure_keeps_kind_os_code_and_message() {
        let plain = IoFailure::from_error(&io::Error::other("boom"));
        assert_eq!(plain.kind, io::ErrorKind::Other);
        assert_eq!(plain.os_code, None);
        assert_eq!(plain.message, "boom");

        let from_os = io::Error::from_raw_os_error(5);
        let kept = IoFailure::from_error(&from_os);
        assert_eq!(kept.os_code, Some(5));
        assert_eq!(kept.kind, from_os.kind());
        assert_eq!(kept.message, from_os.to_string());
    }

    #[test]
    fn request_error_says_what_is_missing() {
        let error: Box<dyn std::error::Error> = Box::new(RequestError::NoRoots);
        assert!(error.to_string().contains("root"));
    }
}
