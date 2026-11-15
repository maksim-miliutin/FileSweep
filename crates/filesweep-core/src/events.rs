//! The one place events leave the scanner.

use std::path::PathBuf;

use crate::types::{Progress, ScanEvent, Skip, SkipReason};

pub(crate) struct Events<'a> {
    sink: &'a mut dyn FnMut(ScanEvent),
    skipped: u64,
}

impl<'a> Events<'a> {
    pub(crate) fn new(sink: &'a mut dyn FnMut(ScanEvent)) -> Self {
        Self { sink, skipped: 0 }
    }

    pub(crate) fn progress(&mut self, progress: Progress) {
        (self.sink)(ScanEvent::Progress(progress));
    }

    pub(crate) fn skip(&mut self, path: PathBuf, reason: SkipReason) {
        self.skipped += 1;
        (self.sink)(ScanEvent::Skipped(Skip { path, reason }));
    }

    pub(crate) fn skipped(&self) -> u64 {
        self.skipped
    }
}
