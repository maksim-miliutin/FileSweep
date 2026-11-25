//! Fixtures shared by the public-contract tests. Everything they create lives
//! in a temporary directory that is removed when the `Tree` is dropped.
#![allow(clippy::disallowed_methods)]
// Every test binary compiles this file and uses its own part of it.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use filesweep_core::{
    DuplicateGroup, IndexedFile, Progress, ScanControl, ScanEvent, ScanPolicy, ScanRequest,
    ScanSummary, Skip, scan,
};

pub struct Tree {
    dir: tempfile::TempDir,
}

impl Tree {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("filesweep-test-")
            .tempdir()
            .expect("create a temporary directory");

        Self { dir }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Creates the file and any missing folders above it.
    pub fn file(&self, rel: impl AsRef<Path>, bytes: &[u8]) -> PathBuf {
        let path = self.root().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the folders above a fixture file");
        }
        fs::write(&path, bytes).expect("write a fixture file");

        path
    }

    pub fn dir(&self, rel: impl AsRef<Path>) -> PathBuf {
        let path = self.root().join(rel);
        fs::create_dir_all(&path).expect("create a fixture folder");

        path
    }

    /// A second name for the file at `from`. Both are inside the tree.
    pub fn hard_link(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> PathBuf {
        let path = self.root().join(to);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the folders above a fixture link");
        }
        fs::hard_link(self.root().join(from), &path).expect("create a fixture hard link");

        path
    }

    /// A symlink at `rel` that points to `target`. Both are inside the tree,
    /// and the target does not have to exist.
    #[cfg(unix)]
    pub fn symlink(&self, target: impl AsRef<Path>, rel: impl AsRef<Path>) -> PathBuf {
        let path = self.root().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the folders above a fixture link");
        }
        std::os::unix::fs::symlink(self.root().join(target), &path)
            .expect("create a fixture symlink");

        path
    }

    /// A file the platform treats as hidden. On Unix that is a leading dot,
    /// so the returned path differs from `rel`.
    #[cfg(unix)]
    pub fn hidden_file(&self, rel: impl AsRef<Path>, bytes: &[u8]) -> PathBuf {
        let rel = rel.as_ref();
        let mut dotted = std::ffi::OsString::from(".");
        dotted.push(rel.file_name().expect("a hidden file needs a name"));

        self.file(rel.with_file_name(dotted), bytes)
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

/// Bytes that differ from one position to the next, so a shifted or cut-off
/// read cannot pass for the original. The published BLAKE3 test vectors use
/// the same sequence.
pub fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

/// Everything one scan reported.
pub struct Run {
    pub groups: Vec<DuplicateGroup>,
    pub skipped: Vec<Skip>,
    pub progress: Vec<Progress>,
    pub indexed: Vec<IndexedFile>,
    pub summary: ScanSummary,
}

pub fn run(roots: &[&Path], policy: ScanPolicy) -> Run {
    run_with(&ScanControl::new(), roots, policy)
}

pub fn run_with(control: &ScanControl, roots: &[&Path], policy: ScanPolicy) -> Run {
    let request = ScanRequest {
        roots: roots.iter().map(|root| root.to_path_buf()).collect(),
        policy,
    };
    let mut groups = Vec::new();
    let mut skipped = Vec::new();
    let mut progress = Vec::new();
    let mut indexed = Vec::new();

    let summary = scan(&request, control, &mut |event| match event {
        ScanEvent::Group(group) => groups.push(group),
        ScanEvent::Skipped(skip) => skipped.push(skip),
        ScanEvent::Progress(step) => progress.push(step),
        ScanEvent::Indexed(file) => indexed.push(file),
    })
    .expect("the request names at least one root");

    Run {
        groups,
        skipped,
        progress,
        indexed,
        summary,
    }
}
