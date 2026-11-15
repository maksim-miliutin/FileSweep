//! Phase one: find the regular files under the roots without opening any.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::disk::{Disk, EntryInfo, EntryKind, Listed};
use crate::events::Events;
use crate::platform::skip_reason;
use crate::types::{FileIdentity, Progress, ScanPolicy, SkipReason};

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

pub(crate) fn walk(
    roots: &[PathBuf],
    policy: &ScanPolicy,
    disk: &dyn Disk,
    events: &mut Events<'_>,
) -> Walked {
    let mut walker = Walker {
        disk,
        policy,
        events,
        visited: HashSet::new(),
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
    policy: &'a ScanPolicy,
    events: &'a mut Events<'e>,
    /// Folders already entered. A second root, or a second name for the same
    /// folder, must not put its files into the index twice.
    visited: HashSet<FileIdentity>,
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
        // Compared by identity, not by path text: one folder can be spelled
        // many ways. A folder that reports no identity is entered unguarded.
        if let Some(identity) = self.disk.dir_identity(&dir)
            && !self.visited.insert(identity)
        {
            self.events.skip(dir, SkipReason::AlreadyCovered);
            return;
        }

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

        match judge(&info, self.policy) {
            Verdict::Skip(reason) => self.events.skip(path, reason),
            Verdict::Enter => self.enter(path),
            Verdict::Empty => self.walked.empty_files += 1,
            Verdict::Index => self.walked.index.push(Indexed {
                path,
                size: info.len,
            }),
        }
    }

    fn report(&mut self) {
        self.events.progress(Progress::Walking {
            files_seen: self.walked.index.len() as u64,
        });
    }
}

enum Verdict {
    Skip(SkipReason),
    Enter,
    Empty,
    Index,
}

fn judge(info: &EntryInfo, policy: &ScanPolicy) -> Verdict {
    let flagged = if info.flags.online_only {
        Some(SkipReason::OnlineOnly)
    } else if info.flags.system && !policy.include_system {
        Some(SkipReason::System)
    } else if info.flags.hidden && !policy.include_hidden {
        Some(SkipReason::Hidden)
    } else {
        None
    };

    // The order of the arms is the rule: a link is never looked at any
    // further, and no policy lets an online-only entry through.
    match (info.kind, flagged) {
        (EntryKind::Link, _) => Verdict::Skip(SkipReason::Link),
        (_, Some(reason)) => Verdict::Skip(reason),
        (EntryKind::Special, None) => Verdict::Skip(SkipReason::Special),
        (EntryKind::Dir, None) => Verdict::Enter,
        (EntryKind::File, None) if info.len == 0 => Verdict::Empty,
        (EntryKind::File, None) => Verdict::Index,
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::fs;

    use super::*;
    use crate::disk::StdDisk;
    use crate::platform::EntryFlags;
    use crate::testkit::{Faulty, Seen, collect};
    use crate::types::{ScanEvent, Skip};

    fn walk_quietly(roots: &[PathBuf]) -> Walked {
        let mut sink = |_event: ScanEvent| {};
        let mut events = Events::new(&mut sink);

        walk(roots, &ScanPolicy::default(), &StdDisk, &mut events)
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

    fn dir_with(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, bytes) in files {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }

        dir
    }

    fn reasons(seen: &Seen) -> Vec<SkipReason> {
        seen.skipped
            .iter()
            .map(|skip| skip.reason.clone())
            .collect()
    }

    const EVERYTHING: ScanPolicy = ScanPolicy {
        include_hidden: true,
        include_system: true,
    };

    #[test]
    fn system_entries_are_skipped_unless_included() {
        let dir = dir_with(&[("pagefile.sys", b"12345"), ("plain.txt", b"1")]);
        let system = dir.path().join("pagefile.sys");
        let flags = EntryFlags {
            system: true,
            ..EntryFlags::default()
        };
        let disk = Faulty::new(StdDisk).flags(&system, flags);

        let hiding = collect(&disk, &[dir.path()], ScanPolicy::default());
        let showing = collect(&disk, &[dir.path()], EVERYTHING);

        assert_eq!(
            hiding.skipped,
            [Skip {
                path: system,
                reason: SkipReason::System
            }]
        );
        assert_eq!(hiding.summary.files_seen, 1);
        assert!(showing.skipped.is_empty());
        assert_eq!(showing.summary.files_seen, 2);
    }

    #[test]
    fn online_only_file_is_skipped_and_never_opened() {
        // Two files of one size are what the hashing phase would open.
        let dir = dir_with(&[("a.bin", b"same bytes"), ("b.bin", b"same bytes")]);
        let (a, b) = (dir.path().join("a.bin"), dir.path().join("b.bin"));
        let cloud = EntryFlags {
            online_only: true,
            ..EntryFlags::default()
        };
        let disk = Faulty::new(StdDisk).flags(&a, cloud).flags(&b, cloud);

        let seen = collect(&disk, &[dir.path()], EVERYTHING);

        assert_eq!(
            reasons(&seen),
            [SkipReason::OnlineOnly, SkipReason::OnlineOnly]
        );
        assert_eq!(seen.skipped[0].path, a);
        assert_eq!(seen.summary.files_seen, 0);
        assert_eq!((disk.opened(&a), disk.opened(&b)), (0, 0));
    }

    #[test]
    fn online_only_directory_is_not_entered() {
        let dir = dir_with(&[("cloud/inside.bin", b"12345"), ("local.bin", b"1")]);
        let cloud = dir.path().join("cloud");
        let flags = EntryFlags {
            online_only: true,
            ..EntryFlags::default()
        };
        let disk = Faulty::new(StdDisk).flags(&cloud, flags);

        let seen = collect(&disk, &[dir.path()], EVERYTHING);

        assert_eq!(
            seen.skipped,
            [Skip {
                path: cloud,
                reason: SkipReason::OnlineOnly
            }]
        );
        assert_eq!(seen.summary.files_seen, 1);
    }

    #[test]
    fn the_first_rule_that_applies_names_the_reason() {
        let dir = dir_with(&[("all.bin", b"1"), ("both.bin", b"22")]);
        let all = EntryFlags {
            hidden: true,
            system: true,
            online_only: true,
        };
        let both = EntryFlags {
            online_only: false,
            ..all
        };
        let disk = Faulty::new(StdDisk)
            .flags(&dir.path().join("all.bin"), all)
            .flags(&dir.path().join("both.bin"), both);
        let with_system = ScanPolicy {
            include_system: true,
            include_hidden: false,
        };

        let strict = collect(&disk, &[dir.path()], ScanPolicy::default());
        let system_shown = collect(&disk, &[dir.path()], with_system);
        let all_shown = collect(&disk, &[dir.path()], EVERYTHING);

        assert_eq!(
            reasons(&strict),
            [SkipReason::OnlineOnly, SkipReason::System]
        );
        assert_eq!(
            reasons(&system_shown),
            [SkipReason::OnlineOnly, SkipReason::Hidden]
        );
        assert_eq!(reasons(&all_shown), [SkipReason::OnlineOnly]);
        assert_eq!(all_shown.summary.files_seen, 1);
    }
}
