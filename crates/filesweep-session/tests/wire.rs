//! The shape of every message, checked against the examples in `contract/`.
//! The interface is tested against the same files, so the two sides cannot
//! drift apart without a test going red.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use filesweep_core::{
    ContentHash, DuplicateFile, DuplicateGroup, FileIdentity, FileStamp, Independence, IoFailure,
    Outcome, ScanSummary, Skip, SkipReason,
};
use filesweep_session::{
    FileView, GroupPage, GroupQuery, GroupSort, GroupView, PageQuery, Phase, QueryError, ScanView,
    SkipCode, SkipPage, SkipView, StartError, State, Status, SummaryView,
};
use serde::Serialize;
use serde_json::Value;

fn contract(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contract")
        .join(name);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));

    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn wire<T: Serialize>(message: &T) -> Value {
    serde_json::to_value(message).expect("a view always serializes")
}

fn at(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}

/// A stamp as the scanner gives it for a file the file system can name.
fn stamp(modified: Option<SystemTime>, link_count: u64) -> FileStamp {
    FileStamp {
        identity: Some(FileIdentity {
            volume: 7,
            file: 42,
        }),
        size: 1,
        modified,
        link_count: Some(link_count),
    }
}

/// A stamp from a file system that names nothing and keeps no times.
fn bare_stamp() -> FileStamp {
    FileStamp {
        identity: None,
        size: 1,
        modified: None,
        link_count: None,
    }
}

fn file(paths: &[&str], stamp: FileStamp) -> DuplicateFile {
    DuplicateFile {
        paths: paths.iter().map(PathBuf::from).collect(),
        stamp,
    }
}

fn group(files: Vec<DuplicateFile>) -> DuplicateGroup {
    DuplicateGroup {
        size: 1,
        hash: ContentHash([0; 32]),
        files,
        independence: Independence::Verified,
        reclaimable_bytes: 0,
    }
}

/// How the first file of a group is shown when its first name is `path`.
fn shown(path: PathBuf) -> FileView {
    let subject = DuplicateFile {
        paths: vec![path],
        stamp: stamp(None, 1),
    };
    let twin = file(&["/t/twin"], stamp(None, 1));

    GroupView::new(0, &group(vec![subject, twin]))
        .files
        .swap_remove(0)
}

fn scanning() -> ScanView {
    ScanView {
        id: 1,
        roots: vec!["/home/me/Downloads".into(), "/home/me/Pictures".into()],
        phase: Phase::Hashing,
        files_seen: 1200,
        files_done: 40,
        files_total: 300,
        bytes_done: 1_048_576,
        bytes_total: 734_003_200,
        groups: 3,
        reclaimable_bytes: 2_097_152,
        skipped: 2,
        skipped_by_reason: BTreeMap::from([(SkipCode::AccessDenied, 1), (SkipCode::Hidden, 1)]),
        summary: None,
        error: None,
    }
}

/// One skip for every reason the scanner has, in the order the contract
/// lists them.
fn every_skip() -> Vec<Skip> {
    let not_ready = IoFailure {
        kind: io::ErrorKind::Other,
        os_code: Some(21),
        message: "the device is not ready (os error 21)".into(),
    };

    [
        ("/t/link", SkipReason::Link),
        ("/t/cloud.docx", SkipReason::OnlineOnly),
        ("/t/sys", SkipReason::System),
        ("/t/.hidden", SkipReason::Hidden),
        ("/t/pipe", SkipReason::Special),
        ("/t/file-as-root", SkipReason::NotADirectory),
        ("/t/again", SkipReason::AlreadyCovered),
        ("/t/locked", SkipReason::AccessDenied),
        ("/t/busy.db", SkipReason::InUse),
        ("/t/gone", SkipReason::Vanished),
        ("/t/growing.log", SkipReason::Changed),
        ("/t/broken", SkipReason::Io(not_ready)),
    ]
    .into_iter()
    .map(|(path, reason)| Skip {
        path: PathBuf::from(path),
        reason,
    })
    .collect()
}

#[test]
fn idle_status_matches_the_contract() {
    assert_eq!(wire(&Status::idle()), contract("status-idle.json"));
}

#[test]
fn scanning_status_matches_the_contract() {
    let status = Status {
        state: State::Scanning,
        scan: Some(scanning()),
    };

    assert_eq!(wire(&status), contract("status-scanning.json"));
}

#[test]
fn completed_status_matches_the_contract() {
    let summary = ScanSummary {
        outcome: Outcome::Completed,
        files_seen: 1200,
        empty_files: 4,
        candidates: 300,
        files_hashed: 297,
        bytes_hashed: 733_000_000,
        groups: 3,
        reclaimable_bytes: 2_097_152,
        skipped: 2,
    };
    let status = Status {
        state: State::Completed,
        scan: Some(ScanView {
            id: 2,
            roots: vec!["/home/me/Downloads".into()],
            files_done: 300,
            bytes_done: 734_003_200,
            summary: Some(SummaryView::from(&summary)),
            ..scanning()
        }),
    };

    assert_eq!(wire(&status), contract("status-completed.json"));
}

#[test]
fn failed_status_matches_the_contract() {
    let status = Status {
        state: State::Failed,
        scan: Some(ScanView {
            id: 3,
            roots: vec!["/home/me/Downloads".into()],
            phase: Phase::Walking,
            files_seen: 17,
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            groups: 0,
            reclaimable_bytes: 0,
            skipped: 0,
            skipped_by_reason: BTreeMap::new(),
            summary: None,
            error: Some("the scanner stopped unexpectedly: boom".into()),
        }),
    };

    assert_eq!(wire(&status), contract("status-failed.json"));
}

#[test]
fn group_page_matches_the_contract() {
    let verified = DuplicateGroup {
        size: 1_048_576,
        hash: ContentHash(std::array::from_fn(|i| i as u8)),
        files: vec![
            file(&["/t/a/one.bin"], stamp(Some(at(1_731_400_000_123)), 1)),
            // Two names inside the scan and a third one somewhere else.
            file(
                &["/t/b/copy.bin", "/t/b/link.bin"],
                stamp(Some(at(1_731_400_060_000)), 3),
            ),
        ],
        independence: Independence::Verified,
        reclaimable_bytes: 1_048_576,
    };
    let unverified = DuplicateGroup {
        size: 12,
        hash: ContentHash([0xab; 32]),
        files: vec![
            file(&["/t/c/notes.txt"], bare_stamp()),
            file(&["/t/d/notes (1).txt"], bare_stamp()),
        ],
        independence: Independence::Unverified,
        reclaimable_bytes: 0,
    };
    let page = GroupPage {
        total: 7,
        offset: 5,
        groups: vec![GroupView::new(5, &verified), GroupView::new(6, &unverified)],
    };

    assert_eq!(wire(&page), contract("groups-page.json"));
}

#[test]
fn skip_page_matches_the_contract() {
    let page = SkipPage {
        total: 12,
        offset: 0,
        items: every_skip().iter().map(SkipView::new).collect(),
    };

    assert_eq!(wire(&page), contract("skipped-page.json"));
}

#[test]
fn every_skip_reason_has_its_own_code() {
    let codes: BTreeSet<SkipCode> = every_skip()
        .iter()
        .map(|skip| SkipCode::of(&skip.reason))
        .collect();

    assert_eq!(codes.len(), 12);
}

#[test]
fn errors_match_the_contract() {
    let examples = contract("errors.json");
    let messages = [
        ("busy", wire(&StartError::Busy)),
        ("noRoots", wire(&StartError::NoRoots)),
        ("tooManyRoots", wire(&StartError::TooManyRoots { max: 64 })),
        (
            "relativeRoot",
            wire(&StartError::RelativeRoot {
                path: "Downloads".into(),
            }),
        ),
        (
            "spawn",
            wire(&StartError::Spawn {
                message: "out of threads".into(),
            }),
        ),
        ("staleScan", wire(&QueryError::StaleScan)),
        ("noSuchFile", wire(&QueryError::NoSuchFile)),
    ];

    for (name, message) in &messages {
        assert_eq!(message, &examples[*name], "{name}");
    }
    // An example nobody produces would be a promise nobody keeps.
    let listed = examples.as_object().map(serde_json::Map::len);
    assert_eq!(listed, Some(messages.len()));
}

#[test]
fn a_refused_call_says_why() {
    let relative = StartError::RelativeRoot {
        path: "Downloads".into(),
    };
    let crowded = StartError::TooManyRoots { max: 64 };
    let no_thread = StartError::Spawn {
        message: "out of threads".into(),
    };

    assert!(relative.to_string().contains("Downloads"));
    assert!(crowded.to_string().contains("64"));
    assert!(no_thread.to_string().contains("out of threads"));
    assert_ne!(
        StartError::Busy.to_string(),
        StartError::NoRoots.to_string()
    );
    assert_ne!(
        QueryError::StaleScan.to_string(),
        QueryError::NoSuchFile.to_string()
    );
}

#[test]
fn queries_are_read_from_the_contract() {
    let examples = contract("queries.json");
    let read_groups = |name: &str| -> GroupQuery {
        serde_json::from_value(examples[name].clone()).unwrap_or_else(|err| panic!("{name}: {err}"))
    };
    let skipped: PageQuery =
        serde_json::from_value(examples["skipped"].clone()).expect("the skipped query reads");

    assert_eq!(
        read_groups("groupsBySavings"),
        GroupQuery {
            offset: 0,
            limit: 50,
            sort: GroupSort::Savings,
        }
    );
    assert_eq!(
        read_groups("groupsAsFound"),
        GroupQuery {
            offset: 100,
            limit: 50,
            sort: GroupSort::Found,
        }
    );
    assert_eq!(
        skipped,
        PageQuery {
            offset: 0,
            limit: 200,
        }
    );
}

#[test]
fn a_path_is_split_into_folder_and_name() {
    let nested = shown(PathBuf::from("/t/a/one.bin"));
    assert_eq!(
        (nested.folder.as_str(), nested.name.as_str()),
        ("/t/a", "one.bin")
    );

    let bare = shown(PathBuf::from("one.bin"));
    assert_eq!((bare.folder.as_str(), bare.name.as_str()), ("", "one.bin"));

    // A path with no last component still has to show something.
    let root = shown(PathBuf::from("/"));
    assert_eq!((root.folder.as_str(), root.name.as_str()), ("", "/"));
}

#[cfg(unix)]
#[test]
fn a_name_that_is_not_unicode_is_shown_with_a_replacement() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let view = shown(PathBuf::from(OsStr::from_bytes(b"/t/caf\xe9.txt")));

    assert_eq!(view.path, "/t/caf\u{fffd}.txt");
    assert_eq!(view.name, "caf\u{fffd}.txt");
    assert_eq!(view.folder, "/t");
}

#[test]
fn modified_time_is_milliseconds_since_1970() {
    let modified_ms = |time: SystemTime| {
        let subject = file(&["/t/a"], stamp(Some(time), 1));
        let twin = file(&["/t/b"], stamp(None, 1));

        GroupView::new(0, &group(vec![subject, twin])).files[0].modified_ms
    };

    assert_eq!(modified_ms(at(1_731_400_000_123)), Some(1_731_400_000_123));
    assert_eq!(
        modified_ms(UNIX_EPOCH - Duration::from_secs(1)),
        Some(-1000)
    );
}

#[test]
fn a_file_with_names_outside_the_scan_is_marked() {
    let linked_elsewhere = |paths: &[&str], stamp: FileStamp| {
        let twin = file(&["/t/twin"], bare_stamp());

        GroupView::new(0, &group(vec![file(paths, stamp), twin])).files[0].linked_elsewhere
    };

    assert!(linked_elsewhere(&["/t/a", "/t/b"], stamp(None, 3)));
    assert!(!linked_elsewhere(&["/t/a", "/t/b"], stamp(None, 2)));
    assert!(!linked_elsewhere(&["/t/a"], bare_stamp()));
}
