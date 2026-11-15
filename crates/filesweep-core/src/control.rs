use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

#[derive(Debug, Default)]
struct State {
    // Read by the scan once pausing lands.
    #[allow(dead_code)]
    paused: bool,
    cancelled: bool,
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

    pub fn cancel(&self) {
        self.lock().cancelled = true;
    }

    pub fn pause(&self) {
        self.lock().paused = true;
    }

    pub fn resume(&self) {
        self.lock().paused = false;
    }

    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
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
