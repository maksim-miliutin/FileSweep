//! What the session promises about one scan, checked with an engine the
//! tests drive by hand. No disk is read here.

use std::collections::BTreeMap;
use std::path::PathBuf;

use filesweep_core::{Outcome, Progress, RequestError, ScanEvent, ScanPolicy, ScanSummary};
use filesweep_core::{SkipReason, SkipReason::AccessDenied};

use crate::error::StartError;
use crate::session::{MAX_ROOTS, Session};
use crate::testkit::{
    Director, QUIET, duplicates, gated, group, root, script, skip, summary, wait_for,
};
use crate::view::{Phase, SkipCode, State, Status, SummaryView};

/// Starts a scripted scan of one root and hands back its director.
fn started(session: &Session) -> Director {
    let (engine, director) = script();
    session
        .start_with(vec![root()], ScanPolicy::default(), engine)
        .expect("the scan starts");

    director
}

fn ended(session: &Session) -> Status {
    wait_for(session, |status| {
        matches!(
            status.state,
            State::Completed | State::Cancelled | State::Failed
        )
    })
}

/// Starts the real scanner on `tree`, held at a gate until the test opens it.
fn started_at_a_gate(session: &Session, tree: &tempfile::TempDir) -> crate::testkit::Gate {
    let (engine, gate) = gated(filesweep_core::scan);
    session
        .start_with(
            vec![tree.path().to_path_buf()],
            ScanPolicy::default(),
            engine,
        )
        .expect("the scan starts");

    gate
}

#[test]
fn a_new_session_is_idle() {
    assert_eq!(Session::new().status(), Status::idle());
}

#[test]
fn start_reports_a_scanning_scan_with_its_roots() {
    let session = Session::new();
    let (engine, _director) = script();

    let status = session
        .start_with(vec![root()], ScanPolicy::default(), engine)
        .expect("the scan starts");

    assert_eq!(status.state, State::Scanning);
    let scan = status.scan.clone().expect("a started scan is shown");
    assert_eq!(scan.id, 1);
    assert_eq!(scan.roots, vec![root().to_string_lossy().into_owned()]);
    assert_eq!(scan.phase, Phase::Walking);
    assert_eq!((scan.files_seen, scan.groups, scan.skipped), (0, 0, 0));
    assert_eq!((scan.summary, scan.error), (None, None));
    // The engine has not moved yet, so asking again changes nothing.
    assert_eq!(session.status(), status);
}

#[test]
fn progress_and_findings_show_up_in_the_status() {
    let session = Session::new();
    let director = started(&session);

    director.emit(ScanEvent::Progress(Progress::Walking { files_seen: 5 }));
    let walking = session.status().scan.expect("the scan is shown");
    assert_eq!((walking.phase, walking.files_seen), (Phase::Walking, 5));

    director.emit(ScanEvent::Progress(Progress::Hashing {
        files_done: 1,
        files_total: 4,
        bytes_done: 10,
        bytes_total: 40,
    }));
    director.emit(ScanEvent::Skipped(skip("locked", AccessDenied)));
    director.emit(ScanEvent::Skipped(skip("gone", SkipReason::Vanished)));
    director.emit(ScanEvent::Skipped(skip("locked too", AccessDenied)));
    director.emit(ScanEvent::Group(group(10, 7)));
    director.emit(ScanEvent::Group(group(4, 4)));

    let scan = session.status().scan.expect("the scan is shown");
    assert_eq!(scan.phase, Phase::Hashing);
    assert_eq!(scan.files_seen, 5);
    assert_eq!((scan.files_done, scan.files_total), (1, 4));
    assert_eq!((scan.bytes_done, scan.bytes_total), (10, 40));
    assert_eq!((scan.groups, scan.reclaimable_bytes), (2, 11));
    assert_eq!(scan.skipped, 3);
    assert_eq!(
        scan.skipped_by_reason,
        BTreeMap::from([(SkipCode::AccessDenied, 2), (SkipCode::Vanished, 1)])
    );
}

#[test]
fn a_finished_scan_is_completed_and_keeps_its_summary() {
    let session = Session::new();
    let director = started(&session);
    director.emit(ScanEvent::Group(group(10, 7)));
    let reported = ScanSummary {
        files_seen: 9,
        groups: 1,
        ..summary(Outcome::Completed)
    };

    director.finish(Ok(reported.clone()));

    let status = ended(&session);
    assert_eq!(status.state, State::Completed);
    let scan = status.scan.expect("a finished scan is still shown");
    assert_eq!(scan.summary, Some(SummaryView::from(&reported)));
    assert_eq!((scan.groups, scan.error), (1, None));
}

#[test]
fn a_scan_the_engine_ends_as_cancelled_is_cancelled() {
    let session = Session::new();
    let director = started(&session);
    director.emit(ScanEvent::Group(group(10, 7)));

    director.finish(Ok(summary(Outcome::Cancelled)));

    let status = ended(&session);
    assert_eq!(status.state, State::Cancelled);
    let scan = status.scan.expect("a cancelled scan is still shown");
    assert!(scan.summary.is_some());
    assert_eq!(scan.groups, 1);
}

#[test]
fn an_engine_that_panics_leaves_a_failed_scan_and_a_usable_session() {
    let session = Session::new();
    let director = started(&session);
    director.emit(ScanEvent::Group(group(10, 7)));

    director.panic("boom");

    let status = ended(&session);
    assert_eq!(status.state, State::Failed);
    let scan = status.scan.expect("a failed scan is still shown");
    assert_eq!(
        scan.error.as_deref(),
        Some("the scanner stopped unexpectedly: boom")
    );
    assert_eq!((scan.summary, scan.groups), (None, 1));

    let _next = started(&session);
    let next = session.status();
    assert_eq!(next.state, State::Scanning);
    assert_eq!(next.scan.map(|scan| scan.id), Some(2));
}

#[test]
fn a_panic_is_described_as_far_as_its_payload_allows() {
    let error_after = |panic: fn(&Director)| {
        let session = Session::new();
        let director = started(&session);
        panic(&director);

        ended(&session).scan.and_then(|scan| scan.error)
    };

    // A formatted panic carries a String, a plain one a &str, and
    // panic_any can carry anything at all.
    assert_eq!(
        error_after(|director| director.panic(String::from("index 3 out of range"))).as_deref(),
        Some("the scanner stopped unexpectedly: index 3 out of range")
    );
    assert_eq!(
        error_after(|director| director.panic(42_u8)).as_deref(),
        Some("the scanner stopped unexpectedly")
    );
}

#[test]
fn an_engine_that_refuses_the_request_leaves_a_failed_scan() {
    let session = Session::new();
    let director = started(&session);

    director.finish(Err(RequestError::NoRoots));

    let status = ended(&session);
    assert_eq!(status.state, State::Failed);
    assert_eq!(
        status.scan.and_then(|scan| scan.error),
        Some(RequestError::NoRoots.to_string())
    );
}

#[test]
fn a_second_start_is_refused_while_a_scan_runs() {
    let session = Session::new();
    let director = started(&session);
    director.emit(ScanEvent::Progress(Progress::Walking { files_seen: 3 }));
    let before = session.status();

    let (engine, _unused) = script();
    let refused = session.start_with(vec![root()], ScanPolicy::default(), engine);

    assert_eq!(refused, Err(StartError::Busy));
    assert_eq!(session.status(), before);
}

#[test]
fn a_new_scan_replaces_a_finished_one() {
    let session = Session::new();
    let first = started(&session);
    first.emit(ScanEvent::Progress(Progress::Walking { files_seen: 3 }));
    first.emit(ScanEvent::Group(group(10, 7)));
    first.finish(Ok(summary(Outcome::Completed)));
    ended(&session);

    let _second = started(&session);

    let status = session.status();
    assert_eq!(status.state, State::Scanning);
    let scan = status.scan.expect("the new scan is shown");
    assert_eq!(scan.id, 2);
    assert_eq!(
        (scan.files_seen, scan.groups, scan.reclaimable_bytes),
        (0, 0, 0)
    );
    assert_eq!(scan.summary, None);
}

#[test]
fn start_refuses_a_request_without_roots() {
    let session = Session::new();
    let (engine, _unused) = script();

    let refused = session.start_with(Vec::new(), ScanPolicy::default(), engine);

    assert_eq!(refused, Err(StartError::NoRoots));
    assert_eq!(session.status(), Status::idle());

    // A refused start is not a scan and takes no id.
    let _director = started(&session);
    assert_eq!(session.status().scan.map(|scan| scan.id), Some(1));
}

#[test]
fn start_refuses_a_relative_root() {
    let session = Session::new();
    let (engine, _unused) = script();
    let roots = vec![root(), PathBuf::from("Downloads")];

    let refused = session.start_with(roots, ScanPolicy::default(), engine);

    assert_eq!(
        refused,
        Err(StartError::RelativeRoot {
            path: "Downloads".into()
        })
    );
    assert_eq!(session.status(), Status::idle());
}

#[test]
fn start_refuses_too_many_roots() {
    let session = Session::new();
    let roots =
        |count: usize| -> Vec<PathBuf> { (0..count).map(|i| root().join(i.to_string())).collect() };

    let (engine, _unused) = script();
    let refused = session.start_with(roots(MAX_ROOTS + 1), ScanPolicy::default(), engine);
    assert_eq!(refused, Err(StartError::TooManyRoots { max: MAX_ROOTS }));
    assert_eq!(session.status(), Status::idle());

    let (engine, _director) = script();
    let accepted = session.start_with(roots(MAX_ROOTS), ScanPolicy::default(), engine);
    assert_eq!(accepted.map(|status| status.state), Ok(State::Scanning));
}

#[test]
fn session_can_be_shared_between_threads() {
    // The desktop shell keeps one session and calls it from any thread.
    fn shared<T: Send + Sync + 'static>() {}

    shared::<Session>();
}

#[test]
fn pause_is_reported_at_once_and_resume_returns_to_scanning() {
    let session = Session::new();
    let _director = started(&session);

    assert_eq!(session.pause().state, State::Paused);
    assert_eq!(session.status().state, State::Paused);

    assert_eq!(session.resume().state, State::Scanning);
    assert_eq!(session.status().state, State::Scanning);
}

#[test]
fn a_paused_scan_does_no_work_until_resumed() {
    let tree = duplicates();
    let session = Session::new();
    let gate = started_at_a_gate(&session, &tree);

    session.pause();
    gate.open();

    // Held before its first step, the scanner has nothing to report.
    std::thread::sleep(QUIET);
    let held = session.status();
    assert_eq!(held.state, State::Paused);
    let scan = held.scan.expect("a paused scan is shown");
    assert_eq!((scan.files_seen, scan.groups), (0, 0));

    session.resume();

    let status = ended(&session);
    assert_eq!(status.state, State::Completed);
    assert_eq!(status.scan.map(|scan| scan.groups), Some(1));
}

#[test]
fn cancel_reports_stopping_until_the_engine_returns() {
    let session = Session::new();
    let director = started(&session);
    director.emit(ScanEvent::Group(group(10, 7)));
    assert!(!director.is_cancelled());

    assert_eq!(session.cancel().state, State::Stopping);

    assert!(director.is_cancelled());
    assert_eq!(session.status().state, State::Stopping);

    director.finish(Ok(summary(Outcome::Cancelled)));
    let status = ended(&session);
    assert_eq!(status.state, State::Cancelled);
    assert_eq!(status.scan.map(|scan| scan.groups), Some(1));
}

#[test]
fn a_cancelled_scan_stops_a_real_engine() {
    let tree = duplicates();
    let session = Session::new();
    let gate = started_at_a_gate(&session, &tree);

    session.cancel();
    gate.open();

    let status = ended(&session);
    assert_eq!(status.state, State::Cancelled);
    let scan = status.scan.expect("a cancelled scan is still shown");
    assert_eq!((scan.files_seen, scan.groups), (0, 0));
}

#[test]
fn a_scan_that_finishes_despite_a_cancel_is_completed() {
    let session = Session::new();
    let director = started(&session);
    session.cancel();

    // The cancel came too late for the engine to notice.
    director.finish(Ok(summary(Outcome::Completed)));

    assert_eq!(ended(&session).state, State::Completed);
}

#[test]
fn a_scan_that_ends_while_paused_is_not_left_paused() {
    let session = Session::new();
    let director = started(&session);
    session.pause();

    // The pause came too late for the engine to notice. A scan left paused
    // here would never end: nothing is running that a resume could wake.
    director.finish(Ok(summary(Outcome::Completed)));

    let status = ended(&session);
    assert_eq!(status.state, State::Completed);
    assert_eq!(session.resume(), status);
}

#[test]
fn cancel_stops_a_paused_scan() {
    let tree = duplicates();
    let session = Session::new();
    let gate = started_at_a_gate(&session, &tree);
    session.pause();
    gate.open();

    assert_eq!(session.cancel().state, State::Stopping);

    let status = ended(&session);
    assert_eq!(status.state, State::Cancelled);
    assert_eq!(status.scan.map(|scan| scan.groups), Some(0));
}

#[test]
fn a_stopping_scan_takes_no_more_orders() {
    let session = Session::new();
    let director = started(&session);
    session.cancel();

    assert_eq!(session.resume().state, State::Stopping);
    assert_eq!(session.pause().state, State::Stopping);
    assert!(director.is_cancelled());
}

#[test]
fn start_is_refused_while_a_scan_is_paused_or_stopping() {
    let session = Session::new();
    let _director = started(&session);
    let start_again = || {
        let (engine, _unused) = script();
        session.start_with(vec![root()], ScanPolicy::default(), engine)
    };

    session.pause();
    assert_eq!(start_again(), Err(StartError::Busy));

    session.cancel();
    assert_eq!(start_again(), Err(StartError::Busy));
}

#[test]
fn controls_do_nothing_without_a_running_scan() {
    let session = Session::new();
    assert_eq!(session.pause(), Status::idle());
    assert_eq!(session.resume(), Status::idle());
    assert_eq!(session.cancel(), Status::idle());

    let director = started(&session);
    director.finish(Ok(summary(Outcome::Completed)));
    let finished = ended(&session);

    assert_eq!(session.pause(), finished);
    assert_eq!(session.resume(), finished);
    assert_eq!(session.cancel(), finished);
}

#[test]
fn dropping_the_session_cancels_the_scan() {
    let session = Session::new();
    let director = started(&session);
    assert!(!director.is_cancelled());

    drop(session);

    assert!(director.is_cancelled());
}
