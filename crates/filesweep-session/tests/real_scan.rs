//! The session with the real scanner behind it, on a temporary folder.

use std::fs;
use std::time::{Duration, Instant};

use filesweep_core::ScanPolicy;
use filesweep_session::{GroupQuery, GroupSort, Session, State, Status};
use tempfile::TempDir;

/// A folder with one pair of identical files, one file of the same size with
/// other content, and one file like no other.
fn tree() -> TempDir {
    let tree = tempfile::tempdir().expect("a temporary folder");
    let write = |name: &str, content: &[u8]| {
        fs::write(tree.path().join(name), content).expect("a file in the temporary folder");
    };

    write("a.bin", b"same bytes");
    write("b.bin", b"same bytes");
    write("c.bin", b"other byte");
    write("unique.txt", b"one of a kind");

    tree
}

/// Waits for the scan to end, and fails instead of hanging if it never does.
fn ended(session: &Session) -> Status {
    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        let status = session.status();
        if status.state != State::Scanning {
            return status;
        }

        assert!(
            Instant::now() < deadline,
            "the scan never ended: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_real_scan_finds_the_duplicates_in_a_temporary_folder() {
    let tree = tree();
    let session = Session::new();

    session
        .start(vec![tree.path().to_path_buf()], ScanPolicy::default())
        .expect("the scan starts");

    let status = ended(&session);
    assert_eq!(status.state, State::Completed);
    let scan = status.scan.expect("a finished scan is still shown");
    assert_eq!(
        (scan.groups, scan.reclaimable_bytes, scan.skipped),
        (1, 10, 0)
    );
    let summary = scan.summary.expect("a finished scan has a summary");
    assert_eq!(
        (summary.files_seen, summary.candidates, summary.files_hashed),
        (4, 3, 3)
    );
}

#[test]
fn the_groups_of_a_real_scan_name_both_copies() {
    let tree = tree();
    let session = Session::new();
    session
        .start(vec![tree.path().to_path_buf()], ScanPolicy::default())
        .expect("the scan starts");
    let scan = ended(&session)
        .scan
        .expect("a finished scan is still shown");
    let by_savings = GroupQuery {
        offset: 0,
        limit: 10,
        sort: GroupSort::Savings,
    };

    let page = session
        .groups(scan.id, &by_savings)
        .expect("the scan is the current one");

    assert_eq!(page.total, 1);
    let group = &page.groups[0];
    assert_eq!((group.size, group.reclaimable_bytes), (10, 10));
    assert_eq!(group.hash.len(), 64);
    let names: Vec<&str> = group.files.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(names, ["a.bin", "b.bin"]);
    for file in &group.files {
        assert_eq!(file.folder, tree.path().to_string_lossy());
        assert!(file.modified_ms.is_some(), "{} has a time", file.name);
        assert_eq!(
            session.file_path(scan.id, group.id, file.id),
            Ok(tree.path().join(&file.name))
        );
    }
}
