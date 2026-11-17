use crate::control::{Flow, ScanControl};
use crate::disk::{Disk, StdDisk};
use crate::events::Events;
use crate::hashing::{Hashed, find_groups};
use crate::types::{Outcome, RequestError, ScanEvent, ScanRequest, ScanSummary};
use crate::walk::{Walked, walk};

/// Blocks until the scan is over and calls `sink` on the calling thread; it
/// starts no threads of its own. It only reads the disk.
///
/// The only error is a request that names no roots. Whatever depends on the
/// state of the disk, a missing or unreadable root included, arrives as a
/// `Skipped` event and the scan goes on.
///
/// A cancelled scan also returns `Ok`, with a summary marked as cancelled.
pub fn scan(
    request: &ScanRequest,
    control: &ScanControl,
    sink: &mut dyn FnMut(ScanEvent),
) -> Result<ScanSummary, RequestError> {
    scan_with_disk(request, control, sink, &StdDisk)
}

pub(crate) fn scan_with_disk(
    request: &ScanRequest,
    control: &ScanControl,
    sink: &mut dyn FnMut(ScanEvent),
    disk: &dyn Disk,
) -> Result<ScanSummary, RequestError> {
    if request.roots.is_empty() {
        return Err(RequestError::NoRoots);
    }

    let mut events = Events::new(sink);
    // Asked before the first event: a control cancelled in advance must
    // leave no trace of a scan.
    let walked = match control.checkpoint() {
        Flow::Go => walk(request, disk, control, &mut events),
        Flow::Stop => Walked {
            cancelled: true,
            ..Walked::default()
        },
    };
    let files_seen = walked.index.len() as u64;
    let hashed = if walked.cancelled {
        Hashed {
            cancelled: true,
            ..Hashed::default()
        }
    } else {
        find_groups(walked.index, disk, control, &mut events)
    };
    let outcome = if hashed.cancelled {
        Outcome::Cancelled
    } else {
        Outcome::Completed
    };

    Ok(ScanSummary {
        outcome,
        files_seen,
        empty_files: walked.empty_files,
        candidates: hashed.candidates,
        files_hashed: hashed.files_hashed,
        bytes_hashed: hashed.bytes_hashed,
        groups: hashed.groups,
        reclaimable_bytes: hashed.reclaimable_bytes,
        skipped: events.skipped(),
    })
}
