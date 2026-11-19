use std::any::Any;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use filesweep_core::{
    ContentHash, DuplicateFile, DuplicateGroup, FileStamp, Independence, Outcome, RequestError,
    ScanControl, ScanEvent, ScanRequest, ScanSummary, Skip, SkipReason,
};

use crate::session::{Engine, Session};
use crate::view::Status;

/// How long a test waits for something that has to happen.
pub(crate) const PATIENCE: Duration = Duration::from_secs(10);

/// How long a test watches for something that must not happen.
pub(crate) const QUIET: Duration = Duration::from_millis(300);

pub(crate) type EngineResult = Result<ScanSummary, RequestError>;

enum Step {
    Emit(ScanEvent),
    AskCancelled,
    Finish(EngineResult),
    Panic(Box<dyn Any + Send>),
}

/// The test's hand on a scripted engine.
pub(crate) struct Director {
    steps: Sender<Step>,
    taken: Receiver<()>,
    answers: Receiver<bool>,
}

impl Director {
    /// Returns once the session has taken the event in.
    pub(crate) fn emit(&self, event: ScanEvent) {
        self.tell(Step::Emit(event));
        self.taken
            .recv_timeout(PATIENCE)
            .expect("the engine hands the event over");
    }

    /// Whether a cancel has reached the engine: asked of the control the
    /// engine itself was given.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.tell(Step::AskCancelled);
        self.answers
            .recv_timeout(PATIENCE)
            .expect("the engine answers")
    }

    pub(crate) fn finish(&self, result: EngineResult) {
        self.tell(Step::Finish(result));
    }

    pub(crate) fn panic(&self, payload: impl Any + Send) {
        self.tell(Step::Panic(Box::new(payload)));
    }

    fn tell(&self, step: Step) {
        self.steps.send(step).expect("the engine is running");
    }
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
                Ok(Step::AskCancelled) => {
                    let _ = answer.send(control.is_cancelled());
                }
                Ok(Step::Emit(event)) => {
                    sink(event);
                    // A director that stopped listening has already failed
                    // its test; the engine has nothing to add.
                    let _ = hand_over.send(());
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

/// Lets a held engine go.
pub(crate) struct Gate {
    opening: Sender<()>,
}

impl Gate {
    pub(crate) fn open(self) {
        self.opening.send(()).expect("the engine waits at the gate");
    }
}

/// Holds `engine` back until its gate is opened, so a test can act on a scan
/// that is known not to have begun. A gate dropped unopened ends the scan as
/// cancelled without ever running the engine.
pub(crate) fn gated(engine: impl Engine) -> (impl Engine, Gate) {
    let (opening, opened) = channel();

    let held = move |request: &ScanRequest,
                     control: &ScanControl,
                     sink: &mut dyn FnMut(ScanEvent)| match opened.recv() {
        Ok(()) => engine(request, control, sink),
        Err(_) => Ok(summary(Outcome::Cancelled)),
    };

    (held, Gate { opening })
}

/// A temporary folder with one pair of identical files, for the real scanner.
pub(crate) fn duplicates() -> tempfile::TempDir {
    let tree = tempfile::tempdir().expect("a temporary folder");
    for name in ["one.bin", "two.bin"] {
        std::fs::write(tree.path().join(name), b"the same bytes twice")
            .expect("a file in the temporary folder");
    }

    tree
}

/// Polls the status until it is what the test waits for. A session that
/// never gets there fails the test with its last status instead of hanging.
pub(crate) fn wait_for(session: &Session, wanted: impl Fn(&Status) -> bool) -> Status {
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

/// An absolute folder on every platform. A scripted engine never opens it.
pub(crate) fn root() -> PathBuf {
    std::env::temp_dir().join("filesweep-scripted-scan")
}

pub(crate) fn group(size: u64, reclaimable_bytes: u64) -> DuplicateGroup {
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

pub(crate) fn skip(name: &str, reason: SkipReason) -> Skip {
    Skip {
        path: root().join(name),
        reason,
    }
}

pub(crate) fn summary(outcome: Outcome) -> ScanSummary {
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
