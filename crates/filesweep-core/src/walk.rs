//! Phase one: find the regular files under the roots without opening any.

use std::path::{Path, PathBuf};

use crate::disk::{Disk, EntryKind, Listed};
use crate::events::Events;
use crate::platform::skip_reason;
use crate::types::{Progress, SkipReason};

// Read by the hashing phase once it lands.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Indexed {
    pub path: PathBuf,
    /// Bytes, from the listing. Checked again on the open handle before hashing.
    pub size: u64,
}

#[derive(Debug, Default)]
pub(crate) struct Walked {
    /// In walk order: roots as requested, the entries of a folder by name.
    pub index: Vec<Indexed>,
    pub empty_files: u64,
}

pub(crate) fn walk(roots: &[PathBuf], disk: &dyn Disk, events: &mut Events<'_>) -> Walked {
    let mut walker = Walker {
        disk,
        events,
        pending: Vec::new(),
        walked: Walked::default(),
    };

    walker.report();
    for root in roots {
        walker.root(root);
    }
    walker.report();

    walker.walked
}

struct Walker<'a, 'e> {
    disk: &'a dyn Disk,
    events: &'a mut Events<'e>,
    /// One sorted listing per folder on the way down. A loop over this stack
    /// stands in for recursion, so a deep tree cannot overflow the call stack.
    pending: Vec<std::vec::IntoIter<Listed>>,
    walked: Walked,
}

impl Walker<'_, '_> {
    fn root(&mut self, root: &Path) {
        // absolute(), not canonicalize(): results must show the path the user
        // chose, not a resolved link or a \\?\ form.
        let root = match std::path::absolute(root) {
            Ok(root) => root,
            Err(err) => {
                self.events.skip(root.to_path_buf(), skip_reason(&err));
                return;
            }
        };

        match self.disk.stat_root(&root) {
            Ok(info) if info.kind == EntryKind::Dir => {}
            Ok(_) => {
                self.events.skip(root, SkipReason::NotADirectory);
                return;
            }
            Err(err) => {
                self.events.skip(root, skip_reason(&err));
                return;
            }
        }

        self.enter(root);
        self.drain();
    }

    fn drain(&mut self) {
        loop {
            let Some(entries) = self.pending.last_mut() else {
                return;
            };
            let Some(entry) = entries.next() else {
                self.pending.pop();
                self.report();
                continue;
            };

            self.visit(entry);
        }
    }

    fn enter(&mut self, dir: PathBuf) {
        match self.disk.list_dir(&dir) {
            Ok(mut entries) => {
                // By name as the OS stores it: byte order, not an alphabet.
                // It makes the walk repeatable and promises nothing more.
                entries.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
                self.pending.push(entries.into_iter());
            }
            Err(err) => self.events.skip(dir, skip_reason(&err)),
        }
    }

    fn visit(&mut self, entry: Listed) {
        let Listed { path, info } = entry;
        let info = match info {
            Ok(info) => info,
            Err(err) => {
                self.events.skip(path, skip_reason(&err));
                return;
            }
        };

        match info.kind {
            EntryKind::Dir => self.enter(path),
            EntryKind::File if info.len == 0 => self.walked.empty_files += 1,
            EntryKind::File => self.walked.index.push(Indexed {
                path,
                size: info.len,
            }),
            // Never followed and never opened.
            EntryKind::Link | EntryKind::Special => {}
        }
    }

    fn report(&mut self) {
        self.events.progress(Progress::Walking {
            files_seen: self.walked.index.len() as u64,
        });
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::fs;

    use super::*;
    use crate::disk::StdDisk;
    use crate::types::ScanEvent;

    fn walk_quietly(roots: &[PathBuf]) -> Walked {
        let mut sink = |_event: ScanEvent| {};
        let mut events = Events::new(&mut sink);

        walk(roots, &StdDisk, &mut events)
    }

    #[test]
    fn roots_in_request_order_and_entries_sorted_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("b-root");
        let second = dir.path().join("a-root");
        fs::create_dir_all(second.join("nested")).unwrap();
        fs::create_dir(&first).unwrap();
        for path in [
            first.join("zeta"),
            first.join("alpha"),
            second.join("two"),
            second.join("one"),
            second.join("nested").join("inner"),
        ] {
            fs::write(&path, path.file_name().unwrap().as_encoded_bytes()).unwrap();
        }

        let walked = walk_quietly(&[first.clone(), second.clone()]);

        let paths: Vec<PathBuf> = walked.index.iter().map(|file| file.path.clone()).collect();
        assert_eq!(
            paths,
            [
                first.join("alpha"),
                first.join("zeta"),
                second.join("nested").join("inner"),
                second.join("one"),
                second.join("two"),
            ]
        );
        assert_eq!(walked.index[0].size, 5);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_names_survive() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(OsStr::from_bytes(b"caf\xFF.bin"));
        fs::write(&path, b"bytes").unwrap();

        let walked = walk_quietly(&[dir.path().to_path_buf()]);

        assert_eq!(walked.index, [Indexed { path, size: 5 }]);
    }
}
