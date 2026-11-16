use crate::control::ScanControl;
use crate::disk::{Disk, StdDisk};
use crate::events::Events;
use crate::hashing::find_groups;
use crate::types::{Outcome, RequestError, ScanEvent, ScanRequest, ScanSummary};
use crate::walk::walk;

/// Blocks until the scan is over and calls `sink` on the calling thread; it
/// starts no threads of its own. It only reads the disk.
///
/// The only error is a request that names no roots. Whatever depends on the
/// state of the disk, a missing or unreadable root included, arrives as a
/// `Skipped` event and the scan goes on.
pub fn scan(
    request: &ScanRequest,
    control: &ScanControl,
    sink: &mut dyn FnMut(ScanEvent),
) -> Result<ScanSummary, RequestError> {
    scan_with_disk(request, control, sink, &StdDisk)
}

pub(crate) fn scan_with_disk(
    request: &ScanRequest,
    _control: &ScanControl,
    sink: &mut dyn FnMut(ScanEvent),
    disk: &dyn Disk,
) -> Result<ScanSummary, RequestError> {
    if request.roots.is_empty() {
        return Err(RequestError::NoRoots);
    }

    let mut events = Events::new(sink);
    let walked = walk(&request.roots, &request.policy, disk, &mut events);
    let files_seen = walked.index.len() as u64;
    let hashed = find_groups(walked.index, disk, &mut events);

    Ok(ScanSummary {
        outcome: Outcome::Completed,
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
