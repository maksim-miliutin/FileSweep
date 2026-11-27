//! What the session promises about one scan, checked with an engine the
//! tests drive by hand. No disk is read here.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use filesweep_core::{
    FileTimes, Outcome, Progress, RequestError, ScanEvent, ScanPolicy, ScanSummary,
};
use filesweep_core::{SkipReason, SkipReason::AccessDenied};

use crate::error::{QueryError, StartError};
use crate::session::{Engine, MAX_ROOTS, Session};
use crate::testkit::{Director, PATIENCE, group, indexed, root, script, skip, summary};
use crate::view::{
    GroupPage, GroupQuery, GroupSort, HistoryQuery, MAX_EDGES, MAX_PAGE, PageQuery, Phase,
    SkipCode, State, Status, SummaryView, UsageQuery, UsageSpan,
};

/// How long a test watches for something that must not happen.
const QUIET: Duration = Duration::from_millis(300);

/// Polls the status until it is what the test waits for. A session that
/// never gets there fails the test with its last status instead of hanging.
fn wait_for(session: &Session, wanted: impl Fn(&Status) -> bool) -> Status {
    let deadline = Instant::now() + PATIENCE;

    loop {
        let status = session.status();
        if wanted(&status) {
            return status;
        }

        assert!(
            Instant::now() < deadline,
            "the session never got there; its last status: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Lets a held engine go.
struct Gate {
    opening: Sender<()>,
}

impl Gate {
    fn open(self) {
        self.opening.send(()).expect("the engine waits at the gate");
    }
}

/// Holds `engine` back until its gate is opened, so a test can act on a scan
/// that is known not to have begun. A gate dropped unopened ends the scan as
/// cancelled without ever running the engine.
fn gated(engine: impl Engine) -> (impl Engine, Gate) {
    let (opening, opened) = channel();

    let held = move |request: &filesweep_core::ScanRequest,
                     control: &filesweep_core::ScanControl,
                     sink: &mut dyn FnMut(ScanEvent)| match opened.recv() {
        Ok(()) => engine(request, control, sink),
        Err(_) => Ok(summary(Outcome::Cancelled)),
    };

    (held, Gate { opening })
}

/// A temporary folder with one pair of identical files, for the real scanner.
fn duplicates() -> tempfile::TempDir {
    let tree = tempfile::tempdir().expect("a temporary folder");
    for name in ["one.bin", "two.bin"] {
        std::fs::write(tree.path().join(name), b"the same bytes twice")
            .expect("a file in the temporary folder");
    }

    tree
}

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
fn started_at_a_gate(session: &Session, tree: &tempfile::TempDir) -> Gate {
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

/// A scripted scan that has reported one group for each `(size, reclaimable)`.
/// Hands back its director and the id to query it by.
fn with_groups(session: &Session, groups: &[(u64, u64)]) -> (Director, u64) {
    let director = started(session);
    for &(size, reclaimable_bytes) in groups {
        director.emit(ScanEvent::Group(group(size, reclaimable_bytes)));
    }
    let scan = session.status().scan.expect("the scan is shown").id;

    (director, scan)
}

fn page(offset: u64, limit: u32, sort: GroupSort) -> GroupQuery {
    GroupQuery {
        offset,
        limit,
        sort,
    }
}

fn ids(page: &GroupPage) -> Vec<u64> {
    page.groups.iter().map(|group| group.id).collect()
}

#[test]
fn groups_come_in_pages_with_the_total() {
    let session = Session::new();
    let sizes = [(50, 50), (40, 40), (30, 30), (20, 20), (10, 10)];
    let (_director, scan) = with_groups(&session, &sizes);

    let second_and_third = session
        .groups(scan, &page(1, 2, GroupSort::Found))
        .expect("the scan is the current one");

    assert_eq!((second_and_third.total, second_and_third.offset), (5, 1));
    assert_eq!(ids(&second_and_third), [1, 2]);
    let first_row = &second_and_third.groups[0];
    assert_eq!((first_row.size, first_row.reclaimable_bytes), (40, 40));
    assert_eq!(first_row.files.len(), 2);
    assert_eq!(first_row.files[1].name, "40-b");
}

#[test]
fn a_page_past_the_end_is_empty() {
    let session = Session::new();
    let (_director, scan) = with_groups(&session, &[(30, 30), (20, 20), (10, 10)]);

    for sort in [GroupSort::Found, GroupSort::Savings] {
        let ask = |offset: u64, limit: u32| {
            session
                .groups(scan, &page(offset, limit, sort))
                .expect("the scan is the current one")
        };

        for offset in [3, 4, u64::MAX] {
            let past = ask(offset, 10);
            assert_eq!((past.total, past.groups.len()), (3, 0), "offset {offset}");
        }
        // A page that only reaches past the end is cut at the end.
        assert_eq!(ids(&ask(2, 10)), [2]);
        // Nothing is a fair thing to ask for: the total alone is an answer.
        assert_eq!((ask(0, 0).total, ask(0, 0).groups.len()), (3, 0));
    }
}

#[test]
fn a_page_is_never_longer_than_the_cap() {
    let session = Session::new();
    let director = started(&session);
    let rows = u64::from(MAX_PAGE) + 100;
    for size in (1..=rows).rev() {
        director.emit(ScanEvent::Group(group(size, size)));
        director.emit(ScanEvent::Skipped(skip(&size.to_string(), AccessDenied)));
    }
    let scan = session.status().scan.expect("the scan is shown").id;

    for sort in [GroupSort::Found, GroupSort::Savings] {
        let groups = session
            .groups(scan, &page(0, u32::MAX, sort))
            .expect("the scan is the current one");
        assert_eq!(groups.total, rows);
        assert_eq!(groups.groups.len(), MAX_PAGE as usize);
    }
    let everything = PageQuery {
        offset: 0,
        limit: u32::MAX,
    };
    let skipped = session
        .skipped(scan, &everything)
        .expect("the scan is the current one");
    assert_eq!(skipped.total, rows);
    assert_eq!(skipped.items.len(), MAX_PAGE as usize);
}

#[test]
fn savings_order_puts_the_most_reclaimable_first() {
    let session = Session::new();
    // Sizes fall as the scanner reports them; savings do not have to.
    let found = [(90, 30), (80, 0), (70, 70), (60, 30), (50, 70)];
    let (_director, scan) = with_groups(&session, &found);

    let by_savings = session
        .groups(scan, &page(0, 10, GroupSort::Savings))
        .expect("the scan is the current one");

    // Equal savings stay in the order they were found.
    assert_eq!(ids(&by_savings), [2, 4, 0, 3, 1]);
    let second_page = session
        .groups(scan, &page(3, 10, GroupSort::Savings))
        .expect("the scan is the current one");
    assert_eq!(ids(&second_page), [3, 1]);
}

#[test]
fn found_order_is_the_order_of_arrival() {
    let session = Session::new();
    let found = [(90, 30), (80, 0), (70, 70), (60, 30), (50, 70)];
    let (_director, scan) = with_groups(&session, &found);

    let as_found = session
        .groups(scan, &page(0, 10, GroupSort::Found))
        .expect("the scan is the current one");

    assert_eq!(ids(&as_found), [0, 1, 2, 3, 4]);
}

#[test]
fn savings_order_follows_groups_that_arrive_later() {
    let session = Session::new();
    let (director, scan) = with_groups(&session, &[(10, 10), (5, 5)]);
    let by_savings = || {
        session
            .groups(scan, &page(0, 10, GroupSort::Savings))
            .expect("the scan is the current one")
    };
    assert_eq!(ids(&by_savings()), [0, 1]);

    // Smaller files, but many copies of them.
    director.emit(ScanEvent::Group(group(4, 40)));

    assert_eq!(ids(&by_savings()), [2, 0, 1]);
}

#[test]
fn skipped_entries_come_in_pages() {
    let session = Session::new();
    let director = started(&session);
    for name in ["a", "b", "c", "d", "e"] {
        director.emit(ScanEvent::Skipped(skip(name, SkipReason::Hidden)));
    }
    let scan = session.status().scan.expect("the scan is shown").id;
    let ask = |offset: u64, limit: u32| {
        session
            .skipped(scan, &PageQuery { offset, limit })
            .expect("the scan is the current one")
    };

    let second_and_third = ask(1, 2);
    assert_eq!((second_and_third.total, second_and_third.offset), (5, 1));
    let paths: Vec<&str> = second_and_third
        .items
        .iter()
        .map(|skip| skip.path.as_str())
        .collect();
    let expected = [root().join("b"), root().join("c")];
    assert_eq!(
        paths,
        [expected[0].to_string_lossy(), expected[1].to_string_lossy()]
    );
    assert_eq!(second_and_third.items[0].reason, SkipCode::Hidden);

    assert_eq!((ask(5, 10).total, ask(5, 10).items.len()), (5, 0));
    assert_eq!(ask(u64::MAX, 10).items.len(), 0);
}

#[test]
fn a_query_for_another_scan_is_refused() {
    let session = Session::new();
    let found = page(0, 10, GroupSort::Found);
    let all = PageQuery {
        offset: 0,
        limit: 10,
    };
    // Nothing was ever scanned, so no id names anything.
    assert_eq!(session.groups(1, &found), Err(QueryError::StaleScan));

    let (first, first_scan) = with_groups(&session, &[(10, 10)]);
    let other = first_scan + 1;
    assert_eq!(session.groups(other, &found), Err(QueryError::StaleScan));
    assert_eq!(session.skipped(other, &all), Err(QueryError::StaleScan));
    assert_eq!(session.file_path(other, 0, 0), Err(QueryError::StaleScan));

    first.finish(Ok(summary(Outcome::Completed)));
    ended(&session);
    let (_second, second_scan) = with_groups(&session, &[]);

    // The first scan was replaced, and its rows went with it.
    assert_eq!(
        session.groups(first_scan, &found),
        Err(QueryError::StaleScan)
    );
    assert_eq!(
        session.skipped(first_scan, &all),
        Err(QueryError::StaleScan)
    );
    assert_eq!(
        session.file_path(first_scan, 0, 0),
        Err(QueryError::StaleScan)
    );
    assert_eq!(
        session.groups(second_scan, &found).map(|page| page.total),
        Ok(0)
    );
}

#[test]
fn file_path_names_the_file_the_ids_point_at() {
    let session = Session::new();
    let (_director, scan) = with_groups(&session, &[(10, 10), (5, 5)]);

    assert_eq!(session.file_path(scan, 0, 1), Ok(root().join("10-b")));
    assert_eq!(session.file_path(scan, 1, 0), Ok(root().join("5-a")));
}

#[cfg(unix)]
#[test]
fn file_path_gives_back_the_exact_path() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    // The guard this is: text shown for a name that is not Unicode cannot be
    // turned back into the path, so nothing may ever act on that text.
    let odd = root().join(OsStr::from_bytes(b"caf\xe9.txt"));
    let mut pair = group(10, 10);
    pair.files[1].paths = vec![odd.clone()];
    let session = Session::new();
    let director = started(&session);
    director.emit(ScanEvent::Group(pair));
    let scan = session.status().scan.expect("the scan is shown").id;

    let shown = session
        .groups(scan, &page(0, 10, GroupSort::Found))
        .expect("the scan is the current one");

    assert_eq!(shown.groups[0].files[1].name, "caf\u{fffd}.txt");
    assert_eq!(session.file_path(scan, 0, 1), Ok(odd));
}

#[test]
fn file_path_refuses_an_unknown_group_or_file() {
    let session = Session::new();
    let (_director, scan) = with_groups(&session, &[(10, 10)]);

    assert_eq!(session.file_path(scan, 1, 0), Err(QueryError::NoSuchFile));
    assert_eq!(session.file_path(scan, 0, 2), Err(QueryError::NoSuchFile));
    assert_eq!(
        session.file_path(scan, u64::MAX, u64::MAX),
        Err(QueryError::NoSuchFile)
    );
}

#[test]
fn results_stay_available_after_the_scan_ends() {
    let session = Session::new();
    let (director, scan) = with_groups(&session, &[(10, 10)]);
    director.emit(ScanEvent::Skipped(skip("locked", AccessDenied)));
    director.finish(Ok(summary(Outcome::Completed)));
    ended(&session);

    let groups = session.groups(scan, &page(0, 10, GroupSort::Savings));
    let skipped = session.skipped(
        scan,
        &PageQuery {
            offset: 0,
            limit: 10,
        },
    );

    assert_eq!(groups.map(|page| page.total), Ok(1));
    assert_eq!(skipped.map(|page| page.total), Ok(1));
    assert_eq!(session.file_path(scan, 0, 0), Ok(root().join("10-a")));
}

fn at(millis: u64) -> Option<SystemTime> {
    Some(UNIX_EPOCH + Duration::from_millis(millis))
}

/// Times of a file that was changed at `millis` and never opened or made
/// later, as far as the file system says.
fn changed(millis: u64) -> FileTimes {
    FileTimes {
        modified: at(millis),
        ..FileTimes::default()
    }
}

fn span(files: u64, bytes: u64) -> UsageSpan {
    UsageSpan { files, bytes }
}

fn history(session: &Session, edges_ms: &[i64]) -> crate::view::UsageHistory {
    session
        .history(
            1,
            &HistoryQuery {
                edges_ms: edges_ms.to_vec(),
            },
        )
        .expect("the scan is the one the session holds")
}

fn used(from_ms: Option<i64>, before_ms: Option<i64>, offset: u64, limit: u32) -> UsageQuery {
    UsageQuery {
        from_ms,
        before_ms,
        offset,
        limit,
    }
}

fn names(session: &Session, query: &UsageQuery) -> Vec<String> {
    session
        .files_used(1, query)
        .expect("the scan is the one the session holds")
        .files
        .into_iter()
        .map(|file| file.name)
        .collect()
}

#[test]
fn history_counts_each_file_by_the_latest_of_its_times() {
    let session = Session::new();
    let director = started(&session);
    // Changed long ago but opened late: last used when it was opened.
    director.emit(ScanEvent::Indexed(indexed(
        "opened",
        10,
        FileTimes {
            modified: at(500),
            accessed: at(2500),
            created: at(400),
        },
    )));
    director.emit(ScanEvent::Indexed(indexed("old", 20, changed(500))));
    // A copy made here after its content last changed.
    director.emit(ScanEvent::Indexed(indexed(
        "copied",
        40,
        FileTimes {
            modified: at(500),
            created: at(1500),
            ..FileTimes::default()
        },
    )));
    director.emit(ScanEvent::Indexed(indexed(
        "timeless",
        80,
        FileTimes::default(),
    )));

    // Out of order and repeated, as a careless caller might send them.
    let counted = history(&session, &[2000, 1000, 2000]);

    assert_eq!(counted.edges_ms, vec![1000, 2000]);
    assert_eq!(counted.spans, vec![span(1, 20), span(1, 40), span(1, 10),]);
    assert_eq!(counted.undated, span(1, 80));
}

#[test]
fn a_file_last_used_on_an_edge_is_in_the_later_span() {
    let session = Session::new();
    let director = started(&session);
    director.emit(ScanEvent::Indexed(indexed("on the edge", 7, changed(1000))));

    let counted = history(&session, &[1000]);

    assert_eq!(counted.spans, vec![span(0, 0), span(1, 7)]);
    assert!(names(&session, &used(None, Some(1000), 0, 10)).is_empty());
    assert_eq!(
        names(&session, &used(Some(1000), None, 0, 10)),
        vec!["on the edge"]
    );
}

#[test]
fn files_of_a_span_come_largest_first_and_alike_sizes_as_walked() {
    let session = Session::new();
    let director = started(&session);
    for (name, size, millis) in [
        ("small", 10, 100),
        ("big", 30, 200),
        ("twin a", 20, 300),
        ("twin b", 20, 400),
        ("recent", 50, 5000),
    ] {
        director.emit(ScanEvent::Indexed(indexed(name, size, changed(millis))));
    }
    director.emit(ScanEvent::Indexed(indexed(
        "timeless",
        70,
        FileTimes::default(),
    )));

    let old = session
        .files_used(1, &used(None, Some(1000), 0, 10))
        .expect("the scan is the one the session holds");

    assert_eq!((old.total, old.bytes, old.offset), (4, 80, 0));
    let rows: Vec<_> = old
        .files
        .iter()
        .map(|file| (file.name.as_str(), file.id, file.size, file.last_used_ms))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("big", 1, 30, 200),
            ("twin a", 2, 20, 300),
            ("twin b", 3, 20, 400),
            ("small", 0, 10, 100),
        ]
    );
    assert_eq!(
        names(&session, &used(None, Some(1000), 1, 2)),
        vec!["twin a", "twin b"]
    );
    // A file with no time at all is in no span, however wide.
    assert_eq!(
        names(&session, &used(None, None, 0, 10)),
        vec!["recent", "big", "twin a", "twin b", "small"]
    );
}

#[test]
fn a_usage_page_is_never_longer_than_the_most_a_page_carries() {
    let session = Session::new();
    let director = started(&session);
    for place in 0..=u64::from(MAX_PAGE) {
        director.emit(ScanEvent::Indexed(indexed(
            &format!("file {place}"),
            1,
            changed(1),
        )));
    }

    let page = session
        .files_used(1, &used(None, None, 0, u32::MAX))
        .expect("the scan is the one the session holds");

    assert_eq!(page.files.len(), MAX_PAGE as usize);
    assert_eq!(page.total, u64::from(MAX_PAGE) + 1);
}

#[test]
fn edges_past_the_most_a_history_takes_are_left_out_the_latest_first() {
    let session = Session::new();
    let _director = started(&session);
    let many = MAX_EDGES as i64 + 10;

    let counted = history(&session, &(0..many).rev().collect::<Vec<_>>());

    assert_eq!(counted.edges_ms, (0..MAX_EDGES as i64).collect::<Vec<_>>());
    assert_eq!(counted.spans.len(), MAX_EDGES + 1);
}

#[test]
fn an_indexed_file_is_found_again_by_its_id_and_only_in_its_own_scan() {
    let session = Session::new();
    let director = started(&session);
    director.emit(ScanEvent::Indexed(indexed("kept.bin", 1, changed(1))));

    assert_eq!(session.indexed_path(1, 0), Ok(root().join("kept.bin")));
    assert_eq!(session.indexed_path(1, 1), Err(QueryError::NoSuchFile));
    assert_eq!(session.indexed_path(2, 0), Err(QueryError::StaleScan));
    assert_eq!(
        session.history(2, &HistoryQuery { edges_ms: vec![] }),
        Err(QueryError::StaleScan)
    );
    assert_eq!(
        session.files_used(2, &used(None, None, 0, 10)),
        Err(QueryError::StaleScan)
    );
}
