use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::control::ScanControl;
use crate::disk::{Disk, EntryInfo, Listed, OpenFile};
use crate::platform::EntryFlags;
use crate::scan::scan_with_disk;
use crate::types::{
    DuplicateGroup, FileIdentity, Progress, ScanEvent, ScanPolicy, ScanRequest, ScanSummary, Skip,
};

/// Wraps a real disk, changes what it reports for chosen paths, and counts
/// what the scanner asks for.
pub(crate) struct Faulty<D: Disk> {
    inner: D,
    flags: HashMap<PathBuf, EntryFlags>,
    opened: RefCell<HashMap<PathBuf, usize>>,
}

impl<D: Disk> Faulty<D> {
    pub(crate) fn new(inner: D) -> Self {
        Self {
            inner,
            flags: HashMap::new(),
            opened: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn flags(mut self, path: &Path, flags: EntryFlags) -> Self {
        self.flags.insert(path.to_path_buf(), flags);
        self
    }

    pub(crate) fn opened(&self, path: &Path) -> usize {
        self.opened.borrow().get(path).copied().unwrap_or(0)
    }
}

impl<D: Disk> Disk for Faulty<D> {
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<Listed>> {
        let mut listed = self.inner.list_dir(dir)?;

        for entry in &mut listed {
            if let (Some(flags), Ok(info)) = (self.flags.get(&entry.path), entry.info.as_mut()) {
                info.flags = *flags;
            }
        }

        Ok(listed)
    }

    fn stat_root(&self, path: &Path) -> io::Result<EntryInfo> {
        self.inner.stat_root(path)
    }

    fn dir_identity(&self, dir: &Path) -> Option<FileIdentity> {
        self.inner.dir_identity(dir)
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn OpenFile>> {
        *self
            .opened
            .borrow_mut()
            .entry(path.to_path_buf())
            .or_default() += 1;

        self.inner.open(path)
    }
}

pub(crate) struct Seen {
    pub groups: Vec<DuplicateGroup>,
    pub skipped: Vec<Skip>,
    pub progress: Vec<Progress>,
    pub summary: ScanSummary,
}

pub(crate) fn collect(disk: &dyn Disk, roots: &[&Path], policy: ScanPolicy) -> Seen {
    let request = ScanRequest {
        roots: roots.iter().map(|root| root.to_path_buf()).collect(),
        policy,
    };
    let mut groups = Vec::new();
    let mut skipped = Vec::new();
    let mut progress = Vec::new();

    let summary = scan_with_disk(
        &request,
        &ScanControl::new(),
        &mut |event| match event {
            ScanEvent::Group(group) => groups.push(group),
            ScanEvent::Skipped(skip) => skipped.push(skip),
            ScanEvent::Progress(step) => progress.push(step),
        },
        disk,
    )
    .expect("the request names at least one root");

    Seen {
        groups,
        skipped,
        progress,
        summary,
    }
}

/// Bytes that differ from one position to the next, so a shifted or cut-off
/// read cannot pass for the original.
pub(crate) fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}
