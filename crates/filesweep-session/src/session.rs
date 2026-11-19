use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use filesweep_core::{
    Outcome, Progress, RequestError, ScanControl, ScanEvent, ScanPolicy, ScanRequest, ScanSummary,
};

use crate::error::{QueryError, StartError};
use crate::results::Results;
use crate::view::{
    GroupPage, GroupQuery, PageQuery, Phase, ScanView, SkipPage, State, Status, SummaryView, shown,
};

pub const MAX_ROOTS: usize = 64;

/// Runs scans in the background, one at a time, and answers for the running
/// scan or the last one. No method waits for the disk: the scanner works on a
/// thread of its own, and a call here only looks at what that thread has
/// reported so far. One session can be called from any number of threads.
#[derive(Default)]
pub struct Session {
    shared: Arc<Mutex<Inner>>,
}

/// Whatever can stand where the scanner stands: a function called the way
/// `filesweep_core::scan` is, fit to be moved to the scan thread.
pub(crate) trait Engine:
    FnOnce(
        &ScanRequest,
        &ScanControl,
        &mut dyn FnMut(ScanEvent),
    ) -> Result<ScanSummary, RequestError>
    + Send
    + 'static
{
}

impl<E> Engine for E where
    E: FnOnce(
            &ScanRequest,
            &ScanControl,
            &mut dyn FnMut(ScanEvent),
        ) -> Result<ScanSummary, RequestError>
        + Send
        + 'static
{
}

#[derive(Default)]
struct Inner {
    /// Ids start at 1 and are never given twice by one session.
    last_id: u64,
    scan: Option<Scan>,
}

struct Scan {
    id: u64,
    roots: Vec<PathBuf>,
    control: ScanControl,
    stage: Stage,
    files_seen: u64,
    /// `None` until the scanner starts reading files.
    hashing: Option<Hashing>,
    results: Results,
}

enum Stage {
    Scanning,
    Paused,
    Stopping,
    Completed(ScanSummary),
    Cancelled(ScanSummary),
    Failed(String),
}

#[derive(Clone, Copy, Default)]
struct Hashing {
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a scan of `roots` and returns at once with its first status.
    /// Refused while another scan runs. A scan that has ended is replaced, and
    /// its results are gone from then on.
    pub fn start(&self, roots: Vec<PathBuf>, policy: ScanPolicy) -> Result<Status, StartError> {
        self.start_with(roots, policy, filesweep_core::scan)
    }

    /// `start` with the scanner given by the caller, so tests can put a
    /// scripted one in its place.
    pub(crate) fn start_with(
        &self,
        roots: Vec<PathBuf>,
        policy: ScanPolicy,
        engine: impl Engine,
    ) -> Result<Status, StartError> {
        check(&roots)?;

        let mut inner = lock(&self.shared);
        if inner.scan.as_ref().is_some_and(Scan::is_running) {
            return Err(StartError::Busy);
        }

        let request = ScanRequest {
            roots: roots.clone(),
            policy,
        };
        let control = ScanControl::new();
        let shared = Arc::clone(&self.shared);

        let worker_control = control.clone();

        // The lock is held across the spawn on purpose: the new thread's
        // first event waits until the scan below is in place to receive it.
        thread::Builder::new()
            .name("filesweep-scan".to_owned())
            .spawn(move || run(&shared, &request, &worker_control, engine))
            .map_err(|err| StartError::Spawn {
                message: err.to_string(),
            })?;

        inner.last_id += 1;
        inner.scan = Some(Scan::new(inner.last_id, roots, control));

        Ok(inner.status())
    }

    /// Holds the running scan until `resume` or `cancel`. The status says
    /// paused at once; the scanner stops within one block of reading.
    pub fn pause(&self) -> Status {
        self.order(|scan| {
            if matches!(scan.stage, Stage::Scanning) {
                scan.control.pause();
                scan.stage = Stage::Paused;
            }
        })
    }

    pub fn resume(&self) -> Status {
        self.order(|scan| {
            if matches!(scan.stage, Stage::Paused) {
                scan.control.resume();
                scan.stage = Stage::Scanning;
            }
        })
    }

    /// Stops the running scan for good, paused or not. The status says
    /// stopping until the scanner has returned, and then how it ended: as a
    /// rule cancelled, but completed if it was already past its last step.
    pub fn cancel(&self) -> Status {
        self.order(|scan| {
            if matches!(scan.stage, Stage::Scanning | Stage::Paused) {
                scan.control.cancel();
                scan.stage = Stage::Stopping;
            }
        })
    }

    /// An order that does not fit the state of the scan is not an error: two
    /// interfaces, or two clicks, may race.
    fn order(&self, order: impl FnOnce(&mut Scan)) -> Status {
        let mut inner = lock(&self.shared);
        if let Some(scan) = inner.scan.as_mut() {
            order(scan);
        }

        inner.status()
    }

    pub fn status(&self) -> Status {
        lock(&self.shared).status()
    }

    /// One page of the groups of `scan`, which has to be the scan the
    /// session holds now. While it runs, more groups may arrive between two
    /// calls; `total` says how many there are at this moment.
    pub fn groups(&self, scan: u64, query: &GroupQuery) -> Result<GroupPage, QueryError> {
        let mut inner = lock(&self.shared);

        Ok(inner.scan_named(scan)?.results.group_page(query))
    }

    pub fn skipped(&self, scan: u64, query: &PageQuery) -> Result<SkipPage, QueryError> {
        let mut inner = lock(&self.shared);

        Ok(inner.scan_named(scan)?.results.skip_page(query))
    }

    /// The path of a file exactly as the scanner saw it, by the ids its view
    /// carries. This, and never the text shown for a path, is what an action
    /// on a file starts from.
    pub fn file_path(&self, scan: u64, group: u64, file: u64) -> Result<PathBuf, QueryError> {
        let mut inner = lock(&self.shared);

        inner
            .scan_named(scan)?
            .results
            .file_path(group, file)
            .map(Path::to_path_buf)
            .ok_or(QueryError::NoSuchFile)
    }
}

impl Drop for Session {
    /// A scan nobody can ask about any more is stopped. The thread is not
    /// waited for: it ends on its own within one block, and waiting here
    /// could hold up a closing application on a disk that does not answer.
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Inner {
    fn status(&self) -> Status {
        match &self.scan {
            Some(scan) => scan.status(),
            None => Status::idle(),
        }
    }

    /// Rows of one scan must never answer a question about another.
    fn scan_named(&mut self, id: u64) -> Result<&mut Scan, QueryError> {
        self.scan
            .as_mut()
            .filter(|scan| scan.id == id)
            .ok_or(QueryError::StaleScan)
    }
}

impl Scan {
    fn new(id: u64, roots: Vec<PathBuf>, control: ScanControl) -> Self {
        Self {
            id,
            roots,
            control,
            stage: Stage::Scanning,
            files_seen: 0,
            hashing: None,
            results: Results::default(),
        }
    }

    /// The scan thread has not returned yet.
    fn is_running(&self) -> bool {
        matches!(
            self.stage,
            Stage::Scanning | Stage::Paused | Stage::Stopping
        )
    }

    fn take(&mut self, event: ScanEvent) {
        match event {
            ScanEvent::Progress(Progress::Walking { files_seen }) => self.files_seen = files_seen,
            ScanEvent::Progress(Progress::Hashing {
                files_done,
                files_total,
                bytes_done,
                bytes_total,
            }) => {
                self.hashing = Some(Hashing {
                    files_done,
                    files_total,
                    bytes_done,
                    bytes_total,
                });
            }
            ScanEvent::Skipped(skip) => self.results.take_skip(skip),
            ScanEvent::Group(group) => self.results.take_group(group),
        }
    }

    fn status(&self) -> Status {
        let (state, summary, error) = match &self.stage {
            Stage::Scanning => (State::Scanning, None, None),
            Stage::Paused => (State::Paused, None, None),
            Stage::Stopping => (State::Stopping, None, None),
            Stage::Completed(summary) => (State::Completed, Some(summary), None),
            Stage::Cancelled(summary) => (State::Cancelled, Some(summary), None),
            Stage::Failed(error) => (State::Failed, None, Some(error)),
        };
        let hashing = self.hashing.unwrap_or_default();

        Status {
            state,
            scan: Some(ScanView {
                id: self.id,
                roots: self.roots.iter().map(|root| shown(root)).collect(),
                phase: match self.hashing {
                    Some(_) => Phase::Hashing,
                    None => Phase::Walking,
                },
                files_seen: self.files_seen,
                files_done: hashing.files_done,
                files_total: hashing.files_total,
                bytes_done: hashing.bytes_done,
                bytes_total: hashing.bytes_total,
                groups: self.results.group_count(),
                reclaimable_bytes: self.results.reclaimable_bytes(),
                skipped: self.results.skip_count(),
                skipped_by_reason: self.results.skipped_by_reason().clone(),
                summary: summary.map(SummaryView::from),
                error: error.cloned(),
            }),
        }
    }
}

fn check(roots: &[PathBuf]) -> Result<(), StartError> {
    if roots.is_empty() {
        return Err(StartError::NoRoots);
    }

    if roots.len() > MAX_ROOTS {
        return Err(StartError::TooManyRoots { max: MAX_ROOTS });
    }

    if let Some(relative) = roots.iter().find(|root| !root.is_absolute()) {
        return Err(StartError::RelativeRoot {
            path: shown(relative),
        });
    }

    Ok(())
}

fn run(shared: &Mutex<Inner>, request: &ScanRequest, control: &ScanControl, engine: impl Engine) {
    // Only one scan runs at a time and none is replaced while it runs, so
    // the scan in place is the one this thread was started for.
    let mut sink = |event: ScanEvent| {
        if let Some(scan) = lock(shared).scan.as_mut() {
            scan.take(event);
        }
    };

    // A panic in the scanner must not leave a scan that looks busy forever.
    let ending = panic::catch_unwind(AssertUnwindSafe(|| engine(request, control, &mut sink)));
    let stage = match ending {
        Ok(Ok(summary)) => match summary.outcome {
            Outcome::Completed => Stage::Completed(summary),
            Outcome::Cancelled => Stage::Cancelled(summary),
        },
        Ok(Err(refusal)) => Stage::Failed(refusal.to_string()),
        Err(payload) => Stage::Failed(breakdown(payload.as_ref())),
    };

    if let Some(scan) = lock(shared).scan.as_mut() {
        scan.stage = stage;
    }
}

/// Words for a panic. Its payload is a `&str` or a `String` when it came
/// from `panic!`, and anything at all otherwise.
fn breakdown(payload: &(dyn Any + Send)) -> String {
    let said = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str));

    match said {
        Some(said) => format!("the scanner stopped unexpectedly: {said}"),
        None => "the scanner stopped unexpectedly".to_owned(),
    }
}

// A poisoned lock means a thread panicked while it held the state. Every
// write here leaves the state usable, and the session has to keep answering.
fn lock(shared: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}
