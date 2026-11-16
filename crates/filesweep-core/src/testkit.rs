use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::control::ScanControl;
use crate::disk::{Disk, EntryInfo, Listed, OpenFile};
use crate::platform::EntryFlags;
use crate::scan::scan_with_disk;
use crate::types::{
    DuplicateGroup, FileIdentity, FileStamp, Progress, ScanEvent, ScanPolicy, ScanRequest,
    ScanSummary, Skip,
};

/// Wraps a real disk, changes what it reports for chosen paths, and counts
/// what the scanner asks for.
pub(crate) struct Faulty<D: Disk> {
    inner: D,
    flags: HashMap<PathBuf, EntryFlags>,
    no_identity: HashSet<PathBuf>,
    opened: RefCell<HashMap<PathBuf, usize>>,
    open_now: Rc<Cell<usize>>,
    max_open: Cell<usize>,
}

impl<D: Disk> Faulty<D> {
    pub(crate) fn new(inner: D) -> Self {
        Self {
            inner,
            flags: HashMap::new(),
            no_identity: HashSet::new(),
            opened: RefCell::new(HashMap::new()),
            open_now: Rc::new(Cell::new(0)),
            max_open: Cell::new(0),
        }
    }

    pub(crate) fn flags(mut self, path: &Path, flags: EntryFlags) -> Self {
        self.flags.insert(path.to_path_buf(), flags);
        self
    }

    /// `path` reports no identity, both as a folder and as an open file.
    pub(crate) fn no_identity(mut self, path: &Path) -> Self {
        self.no_identity.insert(path.to_path_buf());
        self
    }

    pub(crate) fn opened(&self, path: &Path) -> usize {
        self.opened.borrow().get(path).copied().unwrap_or(0)
    }

    pub(crate) fn max_open(&self) -> usize {
        self.max_open.get()
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
        if self.no_identity.contains(dir) {
            return None;
        }

        self.inner.dir_identity(dir)
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn OpenFile>> {
        *self
            .opened
            .borrow_mut()
            .entry(path.to_path_buf())
            .or_default() += 1;

        let inner = self.inner.open(path)?;
        self.open_now.set(self.open_now.get() + 1);
        self.max_open
            .set(self.max_open.get().max(self.open_now.get()));

        Ok(Box::new(FaultyFile {
            inner,
            no_identity: self.no_identity.contains(path),
            open_now: Rc::clone(&self.open_now),
        }))
    }
}

struct FaultyFile {
    inner: Box<dyn OpenFile>,
    no_identity: bool,
    open_now: Rc<Cell<usize>>,
}

impl OpenFile for FaultyFile {
    fn stamp(&mut self) -> io::Result<FileStamp> {
        let mut stamp = self.inner.stamp()?;
        if self.no_identity {
            stamp.identity = None;
        }

        Ok(stamp)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Drop for FaultyFile {
    fn drop(&mut self) {
        self.open_now.set(self.open_now.get() - 1);
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
