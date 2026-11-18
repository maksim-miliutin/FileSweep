mod common;

use common::{Tree, run};
use filesweep_core::{
    Outcome, Progress, RequestError, ScanControl, ScanPolicy, ScanRequest, ScanSummary, SkipReason,
    scan,
};

#[test]
fn no_roots_is_a_request_error() {
    let request = ScanRequest {
        roots: Vec::new(),
        policy: ScanPolicy::default(),
    };
    let mut events = 0;

    let result = scan(&request, &ScanControl::new(), &mut |_| events += 1);

    assert_eq!(result, Err(RequestError::NoRoots));
    assert_eq!(events, 0);
}

#[test]
fn missing_root_is_skipped_not_fatal() {
    let tree = Tree::new();
    let missing = tree.root().join("no-such-folder");

    let seen = run(&[&missing], ScanPolicy::default());

    assert_eq!(seen.summary.outcome, Outcome::Completed);
    assert_eq!(seen.skipped.len(), 1);
    assert_eq!(seen.skipped[0].path, missing);
    assert_eq!(seen.skipped[0].reason, SkipReason::Vanished);
    assert_eq!(seen.summary.skipped, 1);
}

#[test]
fn a_missing_root_does_not_stop_the_next_one() {
    let tree = Tree::new();
    let missing = tree.root().join("no-such-folder");
    let real = tree.dir("real");
    tree.file("real/kept.txt", b"still scanned");

    let seen = run(&[&missing, &real], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 1);
    assert_eq!(seen.summary.skipped, 1);
}

#[test]
fn file_root_is_not_a_directory() {
    let tree = Tree::new();
    let file = tree.file("just-a-file.txt", b"content");

    let seen = run(&[&file], ScanPolicy::default());

    assert_eq!(seen.skipped.len(), 1);
    assert_eq!(seen.skipped[0].path, file);
    assert_eq!(seen.skipped[0].reason, SkipReason::NotADirectory);
    assert_eq!(seen.summary.files_seen, 0);
}

#[test]
fn empty_directory_completes_with_zeros() {
    let tree = Tree::new();

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(
        seen.summary,
        ScanSummary {
            outcome: Outcome::Completed,
            files_seen: 0,
            empty_files: 0,
            candidates: 0,
            files_hashed: 0,
            bytes_hashed: 0,
            groups: 0,
            reclaimable_bytes: 0,
            skipped: 0,
        }
    );
    assert!(seen.skipped.is_empty());
    assert!(!seen.progress.is_empty());
}

#[test]
fn files_seen_and_empty_files_are_counted_apart() {
    let tree = Tree::new();
    tree.file("a.txt", b"one");
    tree.file("deep/b.txt", b"three");
    tree.file("deep/er/c.txt", b"seven");
    tree.file("empty-1", b"");
    tree.file("deep/empty-2", b"");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 3);
    assert_eq!(seen.summary.empty_files, 2);
    assert_eq!(seen.summary.skipped, 0);
}

#[test]
fn walking_progress_starts_and_ends_and_never_shrinks() {
    let tree = Tree::new();
    for name in ["a", "b/c", "b/d", "b/e/f", "g/h"] {
        tree.file(name, name.as_bytes());
    }

    let seen = run(&[tree.root()], ScanPolicy::default());

    let walking: Vec<u64> = seen
        .progress
        .iter()
        .filter_map(|step| match step {
            Progress::Walking { files_seen } => Some(*files_seen),
            Progress::Hashing { .. } => None,
        })
        .collect();
    assert_eq!(
        seen.progress.first(),
        Some(&Progress::Walking { files_seen: 0 })
    );
    assert_eq!(seen.summary.files_seen, 5);
    assert_eq!(walking.last(), Some(&5));
    assert!(walking.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn unicode_and_extensionless_names_are_indexed() {
    let tree = Tree::new();
    tree.file("фото из отпуска.jpg", b"1");
    tree.file("日本語のファイル.txt", b"22");
    tree.file("noext", b"333");
    tree.file("папка/ёжик", b"4444");
    tree.file("emoji 🙂.bin", b"55555");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 5);
    assert!(seen.skipped.is_empty(), "skipped: {:?}", seen.skipped);
}

#[cfg(unix)]
#[test]
fn a_name_that_is_not_utf8_is_still_walked() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let tree = Tree::new();
    tree.file(OsStr::from_bytes(b"caf\xFF.bin"), b"bytes");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 1);
    assert!(seen.skipped.is_empty(), "skipped: {:?}", seen.skipped);
}
