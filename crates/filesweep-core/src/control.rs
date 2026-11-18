use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

#[derive(Debug, Default)]
struct State {
    paused: bool,
    cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Go,
    Stop,
}

/// Clones share one state, so another thread can cancel or pause the scan.
/// One control serves one scan: once cancelled it stays cancelled.
#[derive(Debug, Clone, Default)]
pub struct ScanControl {
    shared: Arc<(Mutex<State>, Condvar)>,
}

impl ScanControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stops the scan for good. It reads at most one more block, reports
    /// nothing further and returns a summary marked as cancelled. A paused
    /// scan is woken up to stop.
    pub fn cancel(&self) {
        self.lock().cancelled = true;
        self.shared.1.notify_all();
    }

    /// Holds the scan at its next block or entry until `resume` or `cancel`.
    /// A file being read stays open meanwhile.
    pub fn pause(&self) {
        self.lock().paused = true;
    }

    pub fn resume(&self) {
        self.lock().paused = false;
        self.shared.1.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// Blocks while the scan is paused. A cancel always wins over a pause.
    pub(crate) fn checkpoint(&self) -> Flow {
        let mut state = self.lock();
        while state.paused && !state.cancelled {
            state = self
                .shared
                .1
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }

        if state.cancelled {
            Flow::Stop
        } else {
            Flow::Go
        }
    }

    // A poisoned lock only means a thread panicked between two flag writes.
    // The flags are still valid, and a cancel must keep working after it.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_control_is_neither_paused_nor_cancelled() {
        let control = ScanControl::new();
        assert!(!control.is_cancelled());
        assert!(!control.lock().paused);
    }

    #[test]
    fn control_clones_share_state() {
        let control = ScanControl::new();
        let clone = control.clone();

        clone.cancel();
        assert!(control.is_cancelled());

        clone.pause();
        assert!(control.lock().paused);
    }

    #[test]
    fn resume_does_not_undo_cancel() {
        let control = ScanControl::new();
        control.pause();
        control.cancel();
        control.resume();

        assert!(control.is_cancelled());
        assert!(!control.lock().paused);
    }
}
