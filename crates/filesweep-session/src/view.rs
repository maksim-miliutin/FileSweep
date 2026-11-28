//! Field names and codes are camelCase on the wire. Numbers travel as JSON
//! numbers, exact up to 2^53: enough for every size and count here. A path is
//! display text and may be lossy; an action names a file by ids instead.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use filesweep_core::{DuplicateFile, DuplicateGroup, Independence, ScanSummary, Skip, SkipReason};
use serde::{Deserialize, Serialize};

/// The most rows one page carries, whatever the query asks for.
pub const MAX_PAGE: u32 = 500;

/// The most edges one history is counted between. More are left out, the
/// latest first.
pub const MAX_EDGES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: State,
    /// The running scan or the last one. `None` only while idle.
    pub scan: Option<ScanView>,
}

impl Status {
    pub fn idle() -> Self {
        Self {
            state: State::Idle,
            scan: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// No scan was started yet.
    Idle,
    Scanning,
    Paused,
    /// A cancel was asked for and the scanner has not returned yet.
    Stopping,
    Cancelled,
    Completed,
    /// The scanner broke down. What it found before that is kept.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// The totals of the next phase are not known yet.
    Walking,
    Hashing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanView {
    /// Names this scan in every query about its results.
    pub id: u64,
    pub roots: Vec<String>,
    pub phase: Phase,
    pub files_seen: u64,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub groups: u64,
    pub reclaimable_bytes: u64,
    pub skipped: u64,
    pub skipped_by_reason: BTreeMap<SkipCode, u64>,
    /// Present once the scanner has returned.
    pub summary: Option<SummaryView>,
    /// Present in the failed state.
    pub error: Option<String>,
}

/// The scanner's own account of a scan that has ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryView {
    pub files_seen: u64,
    pub empty_files: u64,
    pub candidates: u64,
    pub files_hashed: u64,
    pub bytes_hashed: u64,
    pub groups: u64,
    pub reclaimable_bytes: u64,
    pub skipped: u64,
}

impl From<&ScanSummary> for SummaryView {
    fn from(summary: &ScanSummary) -> Self {
        Self {
            files_seen: summary.files_seen,
            empty_files: summary.empty_files,
            candidates: summary.candidates,
            files_hashed: summary.files_hashed,
            bytes_hashed: summary.bytes_hashed,
            groups: summary.groups,
            reclaimable_bytes: summary.reclaimable_bytes,
            skipped: summary.skipped,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipCode {
    Link,
    OnlineOnly,
    System,
    Hidden,
    Special,
    NotADirectory,
    AlreadyCovered,
    AccessDenied,
    InUse,
    Vanished,
    Changed,
    Io,
}

impl SkipCode {
    pub fn of(reason: &SkipReason) -> Self {
        match reason {
            SkipReason::Link => Self::Link,
            SkipReason::OnlineOnly => Self::OnlineOnly,
            SkipReason::System => Self::System,
            SkipReason::Hidden => Self::Hidden,
            SkipReason::Special => Self::Special,
            SkipReason::NotADirectory => Self::NotADirectory,
            SkipReason::AlreadyCovered => Self::AlreadyCovered,
            SkipReason::AccessDenied => Self::AccessDenied,
            SkipReason::InUse => Self::InUse,
            SkipReason::Vanished => Self::Vanished,
            SkipReason::Changed => Self::Changed,
            SkipReason::Io(_) => Self::Io,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkipView {
    pub path: String,
    pub reason: SkipCode,
    /// What the OS said. Only the `io` reason carries it.
    pub detail: Option<String>,
}

impl SkipView {
    pub fn new(skip: &Skip) -> Self {
        let detail = match &skip.reason {
            SkipReason::Io(failure) => Some(failure.message.clone()),
            _ => None,
        };

        Self {
            path: shown(&skip.path),
            reason: SkipCode::of(&skip.reason),
            detail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkipPage {
    /// All skipped entries of the scan, not only the ones on this page.
    pub total: u64,
    pub offset: u64,
    pub items: Vec<SkipView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    /// The place of the group in the order the scanner found it. Stays the
    /// same for the whole scan, whatever order a page is asked in.
    pub id: u64,
    /// Bytes of content in each file.
    pub size: u64,
    /// Lowercase hex.
    pub hash: String,
    pub independence: IndependenceView,
    pub reclaimable_bytes: u64,
    pub files: Vec<FileView>,
}

impl GroupView {
    pub fn new(id: u64, group: &DuplicateGroup) -> Self {
        let files = group
            .files
            .iter()
            .zip(0..)
            .map(|(file, id)| FileView::new(id, file))
            .collect();

        Self {
            id,
            size: group.size,
            hash: group.hash.to_string(),
            independence: match group.independence {
                Independence::Verified => IndependenceView::Verified,
                Independence::Unverified => IndependenceView::Unverified,
            },
            reclaimable_bytes: group.reclaimable_bytes,
            files,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IndependenceView {
    Verified,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileView {
    /// The place of the file in its group.
    pub id: u64,
    /// The first name the scan found for the file.
    pub path: String,
    pub name: String,
    pub folder: String,
    /// Milliseconds since 1970, negative before it. `None` when the file
    /// system keeps no such time.
    pub modified_ms: Option<i64>,
    /// More names of the same file inside the scan: hard links.
    pub other_paths: Vec<String>,
    /// The file has names outside the scanned folders too, so removing it
    /// here frees nothing.
    pub linked_elsewhere: bool,
}

impl FileView {
    fn new(id: u64, file: &DuplicateFile) -> Self {
        let mut names = file.paths.iter();
        // The scanner reports no file without a name. Should one arrive
        // anyway, an empty row is better than a panic in front of the user.
        let first = names.next().map_or(Path::new(""), PathBuf::as_path);
        let names_in_scan = file.paths.len() as u64;

        Self {
            id,
            path: shown(first),
            name: shown(first.file_name().map_or(first, Path::new)),
            folder: first.parent().map(shown).unwrap_or_default(),
            modified_ms: file.stamp.modified.map(unix_millis),
            other_paths: names.map(|path| shown(path)).collect(),
            linked_elsewhere: file
                .stamp
                .link_count
                .is_some_and(|names_on_disk| names_on_disk > names_in_scan),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupPage {
    /// All groups of the scan so far, not only the ones on this page.
    pub total: u64,
    pub offset: u64,
    pub groups: Vec<GroupView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupQuery {
    pub offset: u64,
    pub limit: u32,
    pub sort: GroupSort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupSort {
    /// The most reclaimable space first. Rows may move while a scan runs.
    Savings,
    /// As the scanner found them, largest files first. Rows never move.
    Found,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageQuery {
    pub offset: u64,
    pub limit: u32,
}

/// When a file was last used is the latest time the file system gives for
/// it: changed, opened or made. Windows often does not record opening, so
/// "last used" can be older than the truth, never newer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryQuery {
    /// Milliseconds since 1970, in any order. A repeated edge counts once.
    pub edges_ms: Vec<i64>,
}

/// How many files of a scan, and how many bytes, were last used between each
/// two edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistory {
    /// The edges counted between: ascending, each once.
    pub edges_ms: Vec<i64>,
    /// One more than edges: before the first edge, from each edge up to the
    /// next, and from the last edge on. A time on an edge is in the later span.
    pub spans: Vec<UsageSpan>,
    /// Files the file system gave no time for. They are in no span and are
    /// never listed.
    pub undated: UsageSpan,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSpan {
    pub files: u64,
    pub bytes: u64,
}

/// Files last used in one span of time, the largest first.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQuery {
    /// Last used at this moment or later. Absent: no lower bound.
    #[serde(default)]
    pub from_ms: Option<i64>,
    /// Last used before this moment. Absent: no upper bound.
    #[serde(default)]
    pub before_ms: Option<i64>,
    pub offset: u64,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePage {
    /// Files of the span asked for, not only the ones on this page.
    pub total: u64,
    /// Bytes of all of them.
    pub bytes: u64,
    pub offset: u64,
    pub files: Vec<UsageFileView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageFileView {
    /// The place of the file in the order the walk met it. Names the file
    /// for the whole scan.
    pub id: u64,
    pub path: String,
    pub name: String,
    pub folder: String,
    /// Bytes, as the listing gave them.
    pub size: u64,
    /// The latest of the times below. A file with none is never listed.
    pub last_used_ms: i64,
    pub modified_ms: Option<i64>,
    pub accessed_ms: Option<i64>,
    pub created_ms: Option<i64>,
}

impl UsageFileView {
    /// A file with its last use and none of the times it came from.
    pub(crate) fn new(id: u64, path: &Path, size: u64, last_used_ms: i64) -> Self {
        Self {
            id,
            path: shown(path),
            name: shown(path.file_name().map_or(path, Path::new)),
            folder: path.parent().map(shown).unwrap_or_default(),
            size,
            last_used_ms,
            modified_ms: None,
            accessed_ms: None,
            created_ms: None,
        }
    }
}

/// A path as text for the eye. Lossy for a name that is not Unicode, which is
/// why nothing is ever done to a file by this text.
pub(crate) fn shown(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn unix_millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_millis()).unwrap_or(i64::MAX),
        Err(before) => {
            i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |millis| -millis)
        }
    }
}
