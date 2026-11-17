use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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

#[derive(Clone, Default)]
struct Faults {
    flags: Option<EntryFlags>,
    no_identity: bool,
    list_error: Option<fn() -> io::Error>,
    entry_error: Option<fn() -> io::Error>,
    /// How many opens succeed before the error starts.
    open_error: Option<(usize, fn() -> io::Error)>,
    restamp: Option<fn(usize, FileStamp) -> FileStamp>,
    /// How many bytes are read before the error starts.
    read_error_after: Option<(u64, fn() -> io::Error)>,
    truncate_reads_at: Option<u64>,
    on_list: Option<Rc<dyn Fn()>>,
    on_read: Option<Rc<dyn Fn(usize)>>,
}

/// Wraps a real disk, changes what it reports for chosen paths, and counts
/// what the scanner asks for.
pub(crate) struct Faulty<D: Disk> {
    inner: D,
    faults: HashMap<PathBuf, Faults>,
    opened: RefCell<HashMap<PathBuf, usize>>,
    stamped: RefCell<HashMap<PathBuf, Rc<Cell<usize>>>>,
    open_now: Rc<Cell<usize>>,
    max_open: Cell<usize>,
    listed: RefCell<HashMap<PathBuf, usize>>,
    bytes_read: Rc<Cell<u64>>,
}

impl<D: Disk> Faulty<D> {
    pub(crate) fn new(inner: D) -> Self {
        Self {
            inner,
            faults: HashMap::new(),
            opened: RefCell::new(HashMap::new()),
            stamped: RefCell::new(HashMap::new()),
            open_now: Rc::new(Cell::new(0)),
            max_open: Cell::new(0),
            listed: RefCell::new(HashMap::new()),
            bytes_read: Rc::new(Cell::new(0)),
        }
    }

    fn at(&mut self, path: &Path) -> &mut Faults {
        self.faults.entry(path.to_path_buf()).or_default()
    }

    fn faults_of(&self, path: &Path) -> Faults {
        self.faults.get(path).cloned().unwrap_or_default()
    }

    pub(crate) fn flags(mut self, path: &Path, flags: EntryFlags) -> Self {
        self.at(path).flags = Some(flags);
        self
    }

    /// `path` reports no identity, both as a folder and as an open file.
    pub(crate) fn no_identity(mut self, path: &Path) -> Self {
        self.at(path).no_identity = true;
        self
    }

    pub(crate) fn list_error(mut self, dir: &Path, make: fn() -> io::Error) -> Self {
        self.at(dir).list_error = Some(make);
        self
    }

    /// The folder above `path` is listed, but the details of `path` are not.
    pub(crate) fn entry_error(mut self, path: &Path, make: fn() -> io::Error) -> Self {
        self.at(path).entry_error = Some(make);
        self
    }

    pub(crate) fn open_error(self, path: &Path, make: fn() -> io::Error) -> Self {
        self.open_error_after(path, 0, make)
    }

    /// The first `opens` opens of `path` succeed and the rest fail.
    pub(crate) fn open_error_after(
        mut self,
        path: &Path,
        opens: usize,
        make: fn() -> io::Error,
    ) -> Self {
        self.at(path).open_error = Some((opens, make));
        self
    }

    /// Every stamp of `path` passes through `change`. Its first argument
    /// counts the stamps of that path since the scan began, across opens:
    /// 0 is the first look, 1 is just before the read, 2 is just after it.
    pub(crate) fn restamp(
        mut self,
        path: &Path,
        change: fn(usize, FileStamp) -> FileStamp,
    ) -> Self {
        self.at(path).restamp = Some(change);
        self
    }

    pub(crate) fn read_error_after(
        mut self,
        path: &Path,
        bytes: u64,
        make: fn() -> io::Error,
    ) -> Self {
        self.at(path).read_error_after = Some((bytes, make));
        self
    }

    pub(crate) fn truncate_reads_at(mut self, path: &Path, bytes: u64) -> Self {
        self.at(path).truncate_reads_at = Some(bytes);
        self
    }

    /// `hook` runs each time the folder `dir` is about to be listed.
    pub(crate) fn on_list(mut self, dir: &Path, hook: impl Fn() + 'static) -> Self {
        self.at(dir).on_list = Some(Rc::new(hook));
        self
    }

    /// `hook` runs after each read of `path`, with the number of reads that
    /// came before it on the same handle.
    pub(crate) fn on_read(mut self, path: &Path, hook: impl Fn(usize) + 'static) -> Self {
        self.at(path).on_read = Some(Rc::new(hook));
        self
    }

    /// How many times `path` was opened, failed attempts included.
    pub(crate) fn opened(&self, path: &Path) -> usize {
        self.opened.borrow().get(path).copied().unwrap_or(0)
    }

    pub(crate) fn max_open(&self) -> usize {
        self.max_open.get()
    }

    /// How many times the folder `dir` was listed, failed attempts included.
    pub(crate) fn listed(&self, dir: &Path) -> usize {
        self.listed.borrow().get(dir).copied().unwrap_or(0)
    }

    pub(crate) fn bytes_read(&self) -> u64 {
        self.bytes_read.get()
    }
}

impl<D: Disk> Disk for Faulty<D> {
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<Listed>> {
        let faults = self.faults_of(dir);
        *self
            .listed
            .borrow_mut()
            .entry(dir.to_path_buf())
            .or_default() += 1;
        if let Some(hook) = &faults.on_list {
            hook();
        }
        if let Some(make) = faults.list_error {
            return Err(make());
        }

        let mut listed = self.inner.list_dir(dir)?;
        for entry in &mut listed {
            let faults = self.faults_of(&entry.path);
            if let Some(make) = faults.entry_error {
                entry.info = Err(make());
            }
            if let (Some(flags), Ok(info)) = (faults.flags, entry.info.as_mut()) {
                info.flags = flags;
            }
        }

        Ok(listed)
    }

    fn stat_root(&self, path: &Path) -> io::Result<EntryInfo> {
        self.inner.stat_root(path)
    }

    fn dir_identity(&self, dir: &Path) -> Option<FileIdentity> {
        if self.faults_of(dir).no_identity {
            return None;
        }

        self.inner.dir_identity(dir)
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn OpenFile>> {
        let faults = self.faults_of(path);
        let earlier_opens = {
            let mut opened = self.opened.borrow_mut();
            let count = opened.entry(path.to_path_buf()).or_default();
            *count += 1;
            *count - 1
        };
        if let Some((allowed, make)) = faults.open_error
            && earlier_opens >= allowed
        {
            return Err(make());
        }

        let inner = self.inner.open(path)?;
        self.open_now.set(self.open_now.get() + 1);
        self.max_open
            .set(self.max_open.get().max(self.open_now.get()));
        let stamp_calls = Rc::clone(
            self.stamped
                .borrow_mut()
                .entry(path.to_path_buf())
                .or_default(),
        );

        Ok(Box::new(FaultyFile {
            inner,
            faults,
            stamp_calls,
            consumed: 0,
            read_calls: 0,
            bytes_read: Rc::clone(&self.bytes_read),
            open_now: Rc::clone(&self.open_now),
        }))
    }
}

struct FaultyFile {
    inner: Box<dyn OpenFile>,
    faults: Faults,
    stamp_calls: Rc<Cell<usize>>,
    consumed: u64,
    read_calls: usize,
    bytes_read: Rc<Cell<u64>>,
    open_now: Rc<Cell<usize>>,
}

impl OpenFile for FaultyFile {
    fn stamp(&mut self) -> io::Result<FileStamp> {
        let call = self.stamp_calls.get();
        self.stamp_calls.set(call + 1);

        let mut stamp = self.inner.stamp()?;
        if self.faults.no_identity {
            stamp.identity = None;
        }
        if let Some(change) = self.faults.restamp {
            stamp = change(call, stamp);
        }

        Ok(stamp)
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut limit = buf.len();
        if let Some((after, make)) = self.faults.read_error_after {
            if self.consumed >= after {
                return Err(make());
            }
            limit = limit.min((after - self.consumed) as usize);
        }
        if let Some(end) = self.faults.truncate_reads_at {
            if self.consumed >= end {
                return Ok(0);
            }
            limit = limit.min((end - self.consumed) as usize);
        }

        let read = self.inner.read(&mut buf[..limit])?;
        self.consumed += read as u64;
        self.bytes_read.set(self.bytes_read.get() + read as u64);
        if let Some(hook) = &self.faults.on_read {
            hook(self.read_calls);
        }
        self.read_calls += 1;

        Ok(read)
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
    collect_with(&ScanControl::new(), disk, roots, policy)
}

pub(crate) fn collect_with(
    control: &ScanControl,
    disk: &dyn Disk,
    roots: &[&Path],
    policy: ScanPolicy,
) -> Seen {
    let request = ScanRequest {
        roots: roots.iter().map(|root| root.to_path_buf()).collect(),
        policy,
    };
    let mut groups = Vec::new();
    let mut skipped = Vec::new();
    let mut progress = Vec::new();

    let summary = scan_with_disk(
        &request,
        control,
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
