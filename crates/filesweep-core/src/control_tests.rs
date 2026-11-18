//! Cancel and pause, as a running scan obeys them.
#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::control::ScanControl;
use crate::disk::StdDisk;
use crate::hashing::BLOCK_SIZE;
use crate::scan::scan_with_disk;
use crate::testkit::{Faulty, collect, collect_with, pattern};
use crate::types::{Outcome, Progress, ScanEvent, ScanPolicy, ScanRequest};

/// Long enough for a scan that ignores a pause to say something, short
/// enough to wait for in a test. Waiting can only miss an event that comes
/// late; it cannot report one that never came.
const QUIET: Duration = Duration::from_millis(300);

/// Far longer than any step here takes, so a scan that hangs fails its test
/// instead of hanging the run.
const PATIENCE: Duration = Duration::from_secs(10);

/// Two files with the same content, `blocks` blocks long.
fn pair(dir: &Path, stem: &str, blocks: usize) -> (PathBuf, PathBuf) {
    let content = pattern(blocks * BLOCK_SIZE);
    let first = dir.join(format!("{stem}-1"));
    let second = dir.join(format!("{stem}-2"));
    fs::write(&first, &content).unwrap();
    fs::write(&second, &content).unwrap();

    (first, second)
}

/// Joins a scan thread that is expected to end soon. One that stays asleep
/// fails its test instead of hanging the run.
fn finish<T>(worker: thread::JoinHandle<T>) -> T {
    let deadline = Instant::now() + PATIENCE;
    while !worker.is_finished() {
        assert!(Instant::now() < deadline, "the scan did not finish");
        thread::sleep(Duration::from_millis(5));
    }

    worker.join().unwrap()
}

fn request(root: &Path) -> ScanRequest {
    ScanRequest {
        roots: vec![root.to_path_buf()],
        policy: ScanPolicy::default(),
    }
}

#[test]
fn cancel_before_start_reads_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = pair(dir.path(), "copy", 1);
    let disk = Faulty::new(StdDisk);
    let control = ScanControl::new();
    control.cancel();

    let seen = collect_with(&control, &disk, &[dir.path()], ScanPolicy::default());

    assert_eq!(seen.summary.outcome, Outcome::Cancelled);
    assert!(seen.progress.is_empty(), "progress: {:?}", seen.progress);
    assert!(seen.groups.is_empty() && seen.skipped.is_empty());
    assert_eq!(disk.listed(dir.path()), 0);
    assert_eq!(disk.opened(&first), 0);
}

#[test]
fn cancel_during_walk_stops_the_walk() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        fs::create_dir(dir.path().join(name)).unwrap();
        fs::write(dir.path().join(name).join("file"), pattern(100)).unwrap();
    }
    let control = ScanControl::new();
    let canceller = control.clone();
    let disk = Faulty::new(StdDisk).on_list(&dir.path().join("a"), move || canceller.cancel());

    let seen = collect_with(&control, &disk, &[dir.path()], ScanPolicy::default());

    assert_eq!(seen.summary.outcome, Outcome::Cancelled);
    assert_eq!(disk.listed(&dir.path().join("a")), 1);
    assert_eq!(disk.listed(&dir.path().join("b")), 0);
    assert_eq!(disk.listed(&dir.path().join("c")), 0);
    assert!(seen.summary.files_seen < 3);
    assert!(
        seen.progress
            .iter()
            .all(|step| matches!(step, Progress::Walking { .. })),
        "hashing began after the cancel: {:?}",
        seen.progress
    );
}

// Guards the promise that cancel stops the reading, not just the reporting.
#[test]
fn cancel_during_hashing_reads_at_most_one_more_block() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = pair(dir.path(), "copy", 5);
    let control = ScanControl::new();
    let canceller = control.clone();
    let disk = Faulty::new(StdDisk).on_read(&first, move |call| {
        if call == 0 {
            canceller.cancel();
        }
    });

    let seen = collect_with(&control, &disk, &[dir.path()], ScanPolicy::default());

    assert_eq!(seen.summary.outcome, Outcome::Cancelled);
    assert!(seen.groups.is_empty());
    assert!(
        disk.bytes_read() <= 2 * BLOCK_SIZE as u64,
        "read {} bytes after being told to stop",
        disk.bytes_read()
    );
    assert_eq!(disk.opened(&second), 1, "the next file was read anyway");
}

#[test]
fn groups_emitted_before_a_cancel_stay_valid() {
    let dir = tempfile::tempdir().unwrap();
    let (big, big_twin) = pair(dir.path(), "big", 3);
    let (small, _) = pair(dir.path(), "small", 2);
    let control = ScanControl::new();
    let canceller = control.clone();
    let disk = Faulty::new(StdDisk).on_read(&small, move |call| {
        if call == 0 {
            canceller.cancel();
        }
    });

    let seen = collect_with(&control, &disk, &[dir.path()], ScanPolicy::default());

    assert_eq!(seen.summary.outcome, Outcome::Cancelled);
    assert_eq!(seen.groups.len(), 1);
    assert_eq!(seen.groups[0].size, 3 * BLOCK_SIZE as u64);
    assert_eq!(seen.groups[0].files[0].paths, [big]);
    assert_eq!(seen.groups[0].files[1].paths, [big_twin]);
    assert_eq!(seen.summary.groups, 1);
}

#[test]
fn pause_holds_the_work_until_resume() {
    let dir = tempfile::tempdir().unwrap();
    pair(dir.path(), "copy", 1);
    let control = ScanControl::new();
    control.pause();
    let (events_tx, events_rx) = mpsc::channel();
    let worker = {
        let control = control.clone();
        let request = request(dir.path());
        thread::spawn(move || {
            scan_with_disk(
                &request,
                &control,
                &mut |event| {
                    let _ = events_tx.send(event);
                },
                &StdDisk,
            )
            .unwrap()
        })
    };

    assert_eq!(
        events_rx.recv_timeout(QUIET),
        Err(RecvTimeoutError::Timeout),
        "the scan reported while it was paused"
    );
    assert!(
        !worker.is_finished(),
        "the scan finished while it was paused"
    );

    control.resume();
    let summary = finish(worker);
    let groups: Vec<ScanEvent> = events_rx
        .try_iter()
        .filter(|event| matches!(event, ScanEvent::Group(_)))
        .collect();

    let straight = collect(&StdDisk, &[dir.path()], ScanPolicy::default());
    assert_eq!(summary, straight.summary);
    assert_eq!(summary.outcome, Outcome::Completed);
    assert_eq!(
        groups,
        straight
            .groups
            .into_iter()
            .map(ScanEvent::Group)
            .collect::<Vec<_>>()
    );
}

#[test]
fn pause_in_the_middle_of_a_file_holds_the_read() {
    let dir = tempfile::tempdir().unwrap();
    let (first, _) = pair(dir.path(), "copy", 4);
    let control = ScanControl::new();
    let (events_tx, events_rx) = mpsc::channel();
    let (paused_tx, paused_rx) = mpsc::channel();
    let worker = {
        let control = control.clone();
        let request = request(dir.path());
        thread::spawn(move || {
            let pauser = control.clone();
            let disk = Faulty::new(StdDisk).on_read(&first, move |call| {
                if call == 0 {
                    pauser.pause();
                    let _ = paused_tx.send(());
                }
            });
            let summary = scan_with_disk(
                &request,
                &control,
                &mut |event| {
                    let _ = events_tx.send(event);
                },
                &disk,
            )
            .unwrap();

            (summary, disk.bytes_read())
        })
    };
    paused_rx
        .recv_timeout(PATIENCE)
        .expect("the scan reached its first read");

    // What was already on its way when the pause was asked for may still
    // arrive. After that the stream goes quiet with the scan unfinished.
    loop {
        match events_rx.recv_timeout(QUIET) {
            Ok(_) => continue,
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => panic!("the scan finished while it was paused"),
        }
    }
    assert!(
        !worker.is_finished(),
        "the scan finished while it was paused"
    );

    control.resume();
    let (summary, bytes_read) = finish(worker);

    assert_eq!(summary.outcome, Outcome::Completed);
    assert_eq!(summary.groups, 1);
    assert_eq!(bytes_read, 8 * BLOCK_SIZE as u64);
}

#[test]
fn cancel_while_paused_returns_cancelled_instead_of_hanging() {
    let dir = tempfile::tempdir().unwrap();
    pair(dir.path(), "copy", 1);
    let control = ScanControl::new();
    control.pause();
    let (done_tx, done_rx) = mpsc::channel();
    {
        let control = control.clone();
        let request = request(dir.path());
        thread::spawn(move || {
            let summary = scan_with_disk(&request, &control, &mut |_| {}, &StdDisk).unwrap();
            let _ = done_tx.send(summary);
        });
    }
    // Give the scan time to reach the pause and go to sleep on it.
    thread::sleep(Duration::from_millis(100));

    control.cancel();

    let summary = done_rx
        .recv_timeout(PATIENCE)
        .expect("a cancelled scan must wake up from a pause");
    assert_eq!(summary.outcome, Outcome::Cancelled);
    assert_eq!(summary.files_hashed, 0);
}

#[test]
fn a_cancelled_control_cancels_the_next_scan_at_once() {
    let dir = tempfile::tempdir().unwrap();
    pair(dir.path(), "copy", 1);
    let disk = Faulty::new(StdDisk);
    let control = ScanControl::new();
    control.cancel();

    let first = collect_with(&control, &disk, &[dir.path()], ScanPolicy::default());
    control.resume();
    let second = collect_with(&control, &disk, &[dir.path()], ScanPolicy::default());

    assert_eq!(first.summary.outcome, Outcome::Cancelled);
    assert_eq!(second.summary.outcome, Outcome::Cancelled);
    assert!(second.progress.is_empty());
    assert_eq!(disk.listed(dir.path()), 0);
}

#[test]
fn pause_then_resume_before_start_is_harmless() {
    let dir = tempfile::tempdir().unwrap();
    pair(dir.path(), "copy", 1);
    let control = ScanControl::new();
    control.pause();
    control.resume();

    let seen = collect_with(&control, &StdDisk, &[dir.path()], ScanPolicy::default());

    assert_eq!(seen.summary.outcome, Outcome::Completed);
    assert_eq!(seen.groups.len(), 1);
}
