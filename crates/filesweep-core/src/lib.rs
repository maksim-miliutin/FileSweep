//! The scanner behind FileSweep: it finds files with identical content.
//!
//! It only reads. Nothing on disk is created, changed, renamed or deleted, and
//! `clippy.toml` in this crate rejects the std calls that would do so.

#![forbid(unsafe_code)]

mod control;
mod disk;
mod events;
mod platform;
mod scan;
mod types;
mod walk;

pub use control::ScanControl;
pub use scan::scan;
pub use types::{
    ContentHash, DuplicateFile, DuplicateGroup, FileIdentity, FileStamp, Independence, IoFailure,
    Outcome, Progress, RequestError, ScanEvent, ScanPolicy, ScanRequest, ScanSummary, Skip,
    SkipReason,
};
