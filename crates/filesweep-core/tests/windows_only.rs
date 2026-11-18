//! What only Windows can show: junctions, file attributes, a file another
//! program holds open, and a path longer than the old 260-character limit.
#![cfg(windows)]

mod common;

use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Tree, pattern, run};
use filesweep_core::{DuplicateGroup, ScanPolicy, Skip, SkipReason};

fn skip(path: impl AsRef<Path>, reason: SkipReason) -> Skip {
    Skip {
        path: path.as_ref().to_path_buf(),
        reason,
    }
}

fn everything() -> ScanPolicy {
    ScanPolicy {
        include_hidden: true,
        include_system: true,
    }
}

fn paths_of(group: &DuplicateGroup) -> Vec<PathBuf> {
    group
        .files
        .iter()
        .flat_map(|file| file.paths.iter().cloned())
        .collect()
}

/// Sets or clears a file attribute the way a user would, with `attrib`.
fn attrib(flag: &str, path: &Path) {
    let status = Command::new("attrib")
        .arg(flag)
        .arg(path)
        .status()
        .expect("run attrib");
    assert!(status.success(), "attrib {flag} {}", path.display());
}

/// A junction at `link` that leads to the folder `target`. Unlike a symlink
/// it needs no special rights.
fn junction(link: &Path, target: &Path) {
    let status = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(link)
        .arg(target)
        .status()
        .expect("run mklink");
    assert!(status.success(), "mklink /J {}", link.display());
}

#[test]
fn junction_is_not_followed() {
    let tree = Tree::new();
    tree.file("real/one.bin", &pattern(100));
    let link = tree.root().join("alias");
    junction(&link, &tree.root().join("real"));

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.skipped, [skip(&link, SkipReason::Link)]);
    assert_eq!(seen.summary.files_seen, 1);
}

#[test]
fn a_junction_root_to_a_folder_already_walked_is_covered() {
    let tree = Tree::new();
    tree.file("real/one.bin", &pattern(100));
    let real = tree.root().join("real");
    let link = tree.root().join("alias");
    junction(&link, &real);

    let seen = run(&[&real, &link], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 1);
    assert_eq!(seen.skipped, [skip(&link, SkipReason::AlreadyCovered)]);
}

#[test]
fn hidden_attribute_is_skipped_by_default() {
    let tree = Tree::new();
    tree.file("seen.bin", &pattern(10));
    let file = tree.file("secret.bin", &pattern(20));
    tree.file("cache/inside.bin", &pattern(30));
    let folder = tree.root().join("cache");
    attrib("+h", &file);
    attrib("+h", &folder);

    let hiding = run(&[tree.root()], ScanPolicy::default());
    let showing = run(&[tree.root()], everything());

    assert_eq!(
        hiding.skipped,
        [
            skip(&folder, SkipReason::Hidden),
            skip(&file, SkipReason::Hidden)
        ]
    );
    assert_eq!(hiding.summary.files_seen, 1);
    assert!(showing.skipped.is_empty(), "skipped: {:?}", showing.skipped);
    assert_eq!(showing.summary.files_seen, 3);
}

#[test]
fn system_attribute_is_skipped_unless_included() {
    let tree = Tree::new();
    tree.file("plain.bin", &pattern(10));
    let system = tree.file("protected.bin", &pattern(20));
    attrib("+s", &system);

    let hiding = run(&[tree.root()], ScanPolicy::default());
    let showing = run(&[tree.root()], everything());

    assert_eq!(hiding.skipped, [skip(&system, SkipReason::System)]);
    assert_eq!(hiding.summary.files_seen, 1);
    assert!(showing.skipped.is_empty(), "skipped: {:?}", showing.skipped);
    assert_eq!(showing.summary.files_seen, 2);
}

#[test]
fn exclusively_open_file_is_reported_in_use() {
    let tree = Tree::new();
    let a = tree.file("a.bin", &pattern(5000));
    let b = tree.file("b.bin", &pattern(5000));
    let c = tree.file("c.bin", &pattern(5000));
    // Share mode 0: nobody else may open the file while this handle lives.
    let held = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&b)
        .expect("open a fixture file exclusively");

    let seen = run(&[tree.root()], ScanPolicy::default());
    drop(held);

    assert_eq!(seen.skipped, [skip(&b, SkipReason::InUse)]);
    assert_eq!(seen.groups.len(), 1);
    assert_eq!(paths_of(&seen.groups[0]), [a, c]);
}

#[test]
fn path_longer_than_260_characters_is_scanned() {
    let tree = Tree::new();
    let mut deep = PathBuf::new();
    for _ in 0..8 {
        deep.push("a-folder-name-that-is-forty-characters-x");
    }
    let first = tree.file(deep.join("first.bin"), &pattern(3000));
    let second = tree.file(deep.join("second.bin"), &pattern(3000));
    assert!(first.as_os_str().len() > 260);

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert!(seen.skipped.is_empty(), "skipped: {:?}", seen.skipped);
    assert_eq!(seen.groups.len(), 1);
    assert_eq!(paths_of(&seen.groups[0]), [first, second]);
}
