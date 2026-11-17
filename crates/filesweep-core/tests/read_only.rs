//! The scanner's first promise, checked from the outside: a scan leaves
//! everything it is able to see exactly as it was.
#![allow(clippy::disallowed_methods)]

mod common;

use std::collections::BTreeMap;
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use common::{Tree, pattern, run};
use filesweep_core::ScanPolicy;

/// Everything about one entry that a scan could have damaged. The time of
/// last access is left out: reading a file may move it, and whether it does
/// is the operating system's choice.
#[derive(Debug, PartialEq, Eq)]
enum Entry {
    Dir {
        modified: Option<SystemTime>,
        facts: PlatformFacts,
    },
    File {
        content: Vec<u8>,
        modified: Option<SystemTime>,
        facts: PlatformFacts,
    },
    Link {
        target: PathBuf,
    },
    Other,
}

/// Permission bits, inode and number of names.
#[cfg(unix)]
type PlatformFacts = (u32, u64, u64);

#[cfg(unix)]
fn platform_facts(meta: &Metadata) -> PlatformFacts {
    use std::os::unix::fs::MetadataExt;

    (meta.mode(), meta.ino(), meta.nlink())
}

#[cfg(not(unix))]
type PlatformFacts = ();

#[cfg(not(unix))]
fn platform_facts(_meta: &Metadata) -> PlatformFacts {}

fn look(path: &Path) -> Entry {
    let meta = fs::symlink_metadata(path).unwrap();
    let kind = meta.file_type();

    if kind.is_symlink() {
        Entry::Link {
            target: fs::read_link(path).unwrap(),
        }
    } else if kind.is_dir() {
        Entry::Dir {
            modified: meta.modified().ok(),
            facts: platform_facts(&meta),
        }
    } else if kind.is_file() {
        Entry::File {
            content: fs::read(path).unwrap(),
            modified: meta.modified().ok(),
            facts: platform_facts(&meta),
        }
    } else {
        Entry::Other
    }
}

/// The root and everything below it, by path relative to the root.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Entry> {
    let mut entries = BTreeMap::new();
    entries.insert(PathBuf::new(), look(root));

    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let seen = look(&path);
            if matches!(seen, Entry::Dir { .. }) {
                pending.push(path.clone());
            }
            entries.insert(path.strip_prefix(root).unwrap().to_path_buf(), seen);
        }
    }

    entries
}

#[test]
fn scan_leaves_the_tree_unchanged() {
    let tree = Tree::new();
    tree.file("copies/one.bin", &pattern(70_000));
    tree.file("copies/two.bin", &pattern(70_000));
    tree.file("copies/deeper/three.bin", &pattern(70_000));
    tree.file("same-size/a.bin", &[1; 500]);
    tree.file("same-size/b.bin", &[2; 500]);
    tree.file("alone.txt", b"nothing like me");
    tree.file("empty-1", b"");
    tree.file("nested/empty-2", b"");
    tree.hard_link("copies/one.bin", "links/one-again.bin");
    tree.dir("an-empty-folder");
    #[cfg(unix)]
    {
        tree.hidden_file("copies/secret.bin", &pattern(70_000));
        tree.file(".hidden-folder/four.bin", &pattern(70_000));
        tree.symlink("copies", "link-to-folder");
        tree.symlink("alone.txt", "link-to-file");
        tree.symlink("nowhere", "broken-link");
    }
    let everything = ScanPolicy {
        include_hidden: true,
        include_system: true,
    };

    let before = snapshot(tree.root());
    let seen = run(&[tree.root()], everything);
    let after = snapshot(tree.root());

    // The scan has to have opened and read files for the comparison to mean
    // anything.
    assert!(!seen.groups.is_empty());
    assert!(
        seen.summary.files_hashed >= 5,
        "hashed: {}",
        seen.summary.files_hashed
    );
    assert_eq!(before, after);
}

#[test]
#[ignore = "reads 8 GiB; needs that much free disk where files cannot be sparse"]
fn file_over_4_gib_is_hashed() {
    let tree = Tree::new();
    let len = 4 * 1024 * 1024 * 1024 + 1_u64;
    for name in ["first.bin", "second.bin"] {
        let file = fs::File::create(tree.root().join(name)).unwrap();
        file.set_len(len).unwrap();
    }

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.groups.len(), 1);
    assert_eq!(seen.groups[0].size, len);
    assert_eq!(seen.groups[0].files.len(), 2);
    assert_eq!(seen.summary.bytes_hashed, 2 * len);
}
