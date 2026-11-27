//! A scanner driven by hand, and samples of what a scanner reports: for the
//! tests of this crate and, behind the `testkit` feature, of crates built on
//! the session. Being test support, it fails loudly: a director whose engine
//! is gone panics instead of returning an error.

use std::any::Any;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use filesweep_core::{
    ContentHash, DuplicateFile, DuplicateGroup, FileStamp, FileTimes, Independence, IndexedFile,
    Outcome, RequestError, ScanControl, ScanEvent, ScanPolicy, ScanRequest, ScanSummary, Skip,
    SkipReason,
};

use crate::error::StartError;
use crate::session::{Engine, Session};
use crate::view::Status;

/// How long a test waits for something that has to happen.
pub(crate) const PATIENCE: Duration = Duration::from_secs(10);

enum Step {
    Emit(ScanEvent),
    AskCancelled,
    Finish(Result<ScanSummary, RequestError>),
    Panic(Box<dyn Any + Send>),
}

/// The test's hand on a scripted scan. The scan does nothing until told, and
/// ends as cancelled when its director is dropped.
pub struct Director {
    steps: Sender<Step>,
    taken: Receiver<()>,
    answers: Receiver<bool>,
}

impl Director {
    /// Returns once the session has taken the event in.
    pub fn emit(&self, event: ScanEvent) {
        self.tell(Step::Emit(event));
        self.taken
            .recv_timeout(PATIENCE)
            .expect("the engine hands the event over");
    }

    /// Whether a cancel has reached the scanner: asked of the control the
    /// scanner itself was given.
    pub fn is_cancelled(&self) -> bool {
        self.tell(Step::AskCancelled);
        self.answers
            .recv_timeout(PATIENCE)
            .expect("the engine answers")
    }

    pub fn finish(&self, result: Result<ScanSummary, RequestError>) {
        self.tell(Step::Finish(result));
    }

    pub fn panic(&self, payload: impl Any + Send) {
        self.tell(Step::Panic(Box::new(payload)));
    }

    fn tell(&self, step: Step) {
        self.steps.send(step).expect("the engine is running");
    }
}

pub fn start_scripted(
    session: &Session,
    roots: Vec<PathBuf>,
) -> Result<(Status, Director), StartError> {
    let (engine, director) = script();
    let status = session.start_with(roots, ScanPolicy::default(), engine)?;

    Ok((status, director))
}

/// Left without a director the engine ends as a cancelled scan, so a test
/// that fails midway leaves no thread behind.
pub(crate) fn script() -> (impl Engine, Director) {
    let (steps, inbox) = channel();
    let (hand_over, taken) = channel();
    let (answer, answers) = channel();

    let engine = move |_: &ScanRequest, control: &ScanControl, sink: &mut dyn FnMut(ScanEvent)| {
        loop {
            match inbox.recv() {
                Ok(Step::Emit(event)) => {
                    sink(event);
                    // A director that stopped listening has already failed
                    // its test; the engine has nothing to add.
                    let _ = hand_over.send(());
                }
                Ok(Step::AskCancelled) => {
                    let _ = answer.send(control.is_cancelled());
                }
                Ok(Step::Finish(result)) => return result,
                // resume_unwind carries the payload like a panic does but
                // skips the panic hook, which would print into the test run.
                Ok(Step::Panic(payload)) => std::panic::resume_unwind(payload),
                Err(_) => return Ok(summary(Outcome::Cancelled)),
            }
        }
    };
    let director = Director {
        steps,
        taken,
        answers,
    };

    (engine, director)
}

/// An absolute folder on every platform. A scripted scan never opens it.
pub fn root() -> PathBuf {
    std::env::temp_dir().join("filesweep-scripted-scan")
}

pub fn group(size: u64, reclaimable_bytes: u64) -> DuplicateGroup {
    let file = |name: &str| DuplicateFile {
        paths: vec![root().join(format!("{size}-{name}"))],
        stamp: FileStamp {
            identity: None,
            size,
            modified: None,
            link_count: None,
        },
    };

    DuplicateGroup {
        size,
        hash: ContentHash([0; 32]),
        files: vec![file("a"), file("b")],
        independence: Independence::Verified,
        reclaimable_bytes,
    }
}

/// A file the walk indexed under the scripted root, with the times its
/// listing gave.
pub fn indexed(name: &str, size: u64, times: FileTimes) -> IndexedFile {
    IndexedFile {
        path: root().join(name),
        size,
        times,
    }
}

pub fn skip(name: &str, reason: SkipReason) -> Skip {
    Skip {
        path: root().join(name),
        reason,
    }
}

pub fn summary(outcome: Outcome) -> ScanSummary {
    ScanSummary {
        outcome,
        files_seen: 0,
        empty_files: 0,
        candidates: 0,
        files_hashed: 0,
        bytes_hashed: 0,
        groups: 0,
        reclaimable_bytes: 0,
        skipped: 0,
    }
}
