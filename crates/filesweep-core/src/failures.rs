//! What the scan does when the disk refuses, lies, or changes under it.
//!
//! Every case goes through the seam: none of them can be staged in a
//! temporary directory, least of all by a test that runs as root.
#![allow(clippy::disallowed_methods)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::disk::{Disk, StdDisk};
use crate::testkit::{Faulty, Seen, collect, pattern};
use crate::types::{FileStamp, IoFailure, Progress, ScanPolicy, SkipReason};

/// Files with the same 3000 bytes, in a fresh temporary directory.
fn copies(names: &[&str]) -> (tempfile::TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = names
        .iter()
        .map(|name| {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, pattern(3000)).unwrap();
            path
        })
        .collect();

    (dir, paths)
}

fn scan(disk: &dyn Disk, root: &Path) -> Seen {
    collect(disk, &[root], ScanPolicy::default())
}

fn skips(seen: &Seen) -> Vec<(PathBuf, SkipReason)> {
    seen.skipped
        .iter()
        .map(|skip| (skip.path.clone(), skip.reason.clone()))
        .collect()
}

/// The first path of every file in the only group.
fn grouped(seen: &Seen) -> Vec<PathBuf> {
    assert_eq!(seen.groups.len(), 1, "groups: {:?}", seen.groups);

    seen.groups[0]
        .files
        .iter()
        .map(|file| file.paths[0].clone())
        .collect()
}

fn denied() -> io::Error {
    io::Error::from(io::ErrorKind::PermissionDenied)
}

fn gone() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

fn broken() -> io::Error {
    io::Error::other("the disk is on fire")
}

fn broken_reason() -> SkipReason {
    SkipReason::Io(IoFailure {
        kind: io::ErrorKind::Other,
        os_code: None,
        message: "the disk is on fire".to_string(),
    })
}

fn a_second_later(stamp: FileStamp) -> FileStamp {
    FileStamp {
        modified: stamp.modified.map(|time| time + Duration::from_secs(1)),
        ..stamp
    }
}

#[test]
fn unreadable_directory_is_skipped_and_the_rest_scanned() {
    let (dir, paths) = copies(&["locked/inside.bin", "open/a.bin", "open/b.bin"]);
    let locked = dir.path().join("locked");
    let disk = Faulty::new(StdDisk).list_error(&locked, denied);

    let seen = scan(&disk, dir.path());

    assert_eq!(skips(&seen), [(locked, SkipReason::AccessDenied)]);
    assert_eq!(grouped(&seen), paths[1..]);
}

#[test]
fn an_entry_whose_details_cannot_be_read_is_skipped_alone() {
    let (dir, paths) = copies(&["a.bin", "b.bin", "c.bin"]);
    let disk = Faulty::new(StdDisk).entry_error(&paths[1], denied);

    let seen = scan(&disk, dir.path());

    assert_eq!(skips(&seen), [(paths[1].clone(), SkipReason::AccessDenied)]);
    assert_eq!(grouped(&seen), [paths[0].clone(), paths[2].clone()]);
}

#[test]
fn open_errors_are_reported_per_file() {
    let (dir, paths) = copies(&["a.bin", "b.bin", "c.bin", "d.bin", "e.bin"]);
    let disk = Faulty::new(StdDisk)
        .open_error(&paths[0], denied)
        .open_error(&paths[1], gone)
        .open_error(&paths[2], broken);

    let seen = scan(&disk, dir.path());

    assert_eq!(
        skips(&seen),
        [
            (paths[0].clone(), SkipReason::AccessDenied),
            (paths[1].clone(), SkipReason::Vanished),
            (paths[2].clone(), broken_reason()),
        ]
    );
    assert_eq!(grouped(&seen), paths[3..]);
}

#[test]
fn a_file_that_fails_to_open_does_not_spoil_its_twins() {
    let (dir, paths) = copies(&["a.bin", "b.bin", "c.bin"]);
    let disk = Faulty::new(StdDisk).open_error(&paths[1], denied);

    let seen = scan(&disk, dir.path());

    assert_eq!(grouped(&seen), [paths[0].clone(), paths[2].clone()]);
    assert_eq!(seen.summary.skipped, 1);
    assert_eq!(seen.summary.groups, 1);
}

#[test]
fn a_file_that_vanishes_between_the_passes_is_skipped_once() {
    let (dir, paths) = copies(&["a.bin", "b.bin", "c.bin"]);
    let disk = Faulty::new(StdDisk).open_error_after(&paths[1], 1, gone);

    let seen = scan(&disk, dir.path());

    assert_eq!(skips(&seen), [(paths[1].clone(), SkipReason::Vanished)]);
    assert_eq!(grouped(&seen), [paths[0].clone(), paths[2].clone()]);
    assert_eq!(disk.opened(&paths[1]), 2);
}

#[test]
fn size_differing_from_the_listing_is_changed() {
    let (dir, paths) = copies(&["a.bin", "b.bin", "c.bin"]);
    let disk = Faulty::new(StdDisk).restamp(&paths[1], |_, stamp| FileStamp {
        size: stamp.size + 1,
        ..stamp
    });

    let seen = scan(&disk, dir.path());

    assert_eq!(skips(&seen), [(paths[1].clone(), SkipReason::Changed)]);
    assert_eq!(grouped(&seen), [paths[0].clone(), paths[2].clone()]);
    assert_eq!(disk.opened(&paths[1]), 1, "a changed file was read anyway");
}

// Guards the promise that a file did not change while it was read. A hash
// taken across a change describes bytes that were never on disk together,
// and a group built on it could offer a file that is no copy for removal.
#[test]
fn modification_during_read_is_changed() {
    let (dir, paths) = copies(&["a.bin", "b.bin", "c.bin"]);
    let disk = Faulty::new(StdDisk).restamp(&paths[1], |call, stamp| {
        if call == 2 {
            a_second_later(stamp)
        } else {
            stamp
        }
    });

    let seen = scan(&disk, dir.path());

    assert_eq!(skips(&seen), [(paths[1].clone(), SkipReason::Changed)]);
    assert_eq!(grouped(&seen), [paths[0].clone(), paths[2].clone()]);
}

#[test]
fn a_file_replaced_between_the_passes_is_changed() {
    let (dir, paths) = copies(&["a.bin", "b.bin", "c.bin"]);
    let disk = Faulty::new(StdDisk).restamp(&paths[1], |call, stamp| {
        if call >= 1 {
            a_second_later(stamp)
        } else {
            stamp
        }
    });

    let seen = scan(&disk, dir.path());

    assert_eq!(skips(&seen), [(paths[1].clone(), SkipReason::Changed)]);
    assert_eq!(grouped(&seen), [paths[0].clone(), paths[2].clone()]);
    assert_eq!(
        seen.summary.bytes_hashed, 6000,
        "the replaced file was read"
    );
}

#[test]
fn short_read_is_changed() {
    let (dir, paths) = copies(&["a.bin", "b.bin", "c.bin"]);
    let disk = Faulty::new(StdDisk).truncate_reads_at(&paths[1], 1000);

    let seen = scan(&disk, dir.path());

    assert_eq!(skips(&seen), [(paths[1].clone(), SkipReason::Changed)]);
    assert_eq!(grouped(&seen), [paths[0].clone(), paths[2].clone()]);
}

#[test]
fn read_error_midway_is_reported_and_the_scan_continues() {
    let (dir, paths) = copies(&["a.bin", "b.bin", "c.bin"]);
    let disk = Faulty::new(StdDisk).read_error_after(&paths[1], 1000, broken);

    let seen = scan(&disk, dir.path());

    assert_eq!(skips(&seen), [(paths[1].clone(), broken_reason())]);
    assert_eq!(grouped(&seen), [paths[0].clone(), paths[2].clone()]);
}

/// One refused folder, one refused file and one changed file among good copies.
fn mixed_failures() -> (tempfile::TempDir, Faulty<StdDisk>) {
    let (dir, paths) = copies(&["locked/x.bin", "a.bin", "b.bin", "c.bin", "d.bin"]);
    let disk = Faulty::new(StdDisk)
        .list_error(&dir.path().join("locked"), denied)
        .open_error(&paths[2], gone)
        .truncate_reads_at(&paths[3], 10);

    (dir, disk)
}

#[test]
fn skipped_in_summary_equals_skipped_events() {
    let (dir, disk) = mixed_failures();

    let seen = scan(&disk, dir.path());

    assert_eq!(seen.skipped.len(), 3);
    assert_eq!(seen.summary.skipped, 3);
}

#[test]
fn progress_reaches_its_totals_when_files_are_skipped() {
    let (dir, disk) = mixed_failures();

    let seen = scan(&disk, dir.path());

    let hashing: Vec<[u64; 4]> = seen
        .progress
        .iter()
        .filter_map(|step| match step {
            Progress::Hashing {
                files_done,
                files_total,
                bytes_done,
                bytes_total,
            } => Some([*files_done, *files_total, *bytes_done, *bytes_total]),
            Progress::Walking { .. } => None,
        })
        .collect();
    assert_eq!(hashing.last(), Some(&[4, 4, 12000, 12000]));
    assert!(
        hashing
            .windows(2)
            .all(|pair| pair[0][0] <= pair[1][0] && pair[0][2] <= pair[1][2])
    );
}

#[test]
fn the_stamp_in_a_group_is_the_one_taken_after_the_read() {
    let (dir, paths) = copies(&["a.bin", "b.bin"]);
    // A new name for the file is not a change of its content.
    let disk = Faulty::new(StdDisk).restamp(&paths[0], |call, stamp| {
        if call == 2 {
            FileStamp {
                link_count: Some(7),
                ..stamp
            }
        } else {
            stamp
        }
    });

    let seen = scan(&disk, dir.path());

    assert_eq!(grouped(&seen), paths);
    assert_eq!(seen.groups[0].files[0].stamp.link_count, Some(7));
    assert!(seen.skipped.is_empty(), "skipped: {:?}", seen.skipped);
}

#[cfg(windows)]
#[test]
fn in_use_code_32_is_reported_as_in_use() {
    let (dir, paths) = copies(&["a.bin", "b.bin", "c.bin"]);
    let disk = Faulty::new(StdDisk).open_error(&paths[1], || io::Error::from_raw_os_error(32));

    let seen = scan(&disk, dir.path());

    assert_eq!(skips(&seen), [(paths[1].clone(), SkipReason::InUse)]);
}
