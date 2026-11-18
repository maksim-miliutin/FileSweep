//! The session with the real scanner behind it, on a temporary folder.

use std::fs;
use std::time::{Duration, Instant};

use filesweep_core::ScanPolicy;
use filesweep_session::{Session, State, Status};
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
