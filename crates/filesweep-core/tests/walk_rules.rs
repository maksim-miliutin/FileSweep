//! What the walk leaves out below a root, and why.
//!
//! Unix only for now: the fixtures are symlinks, dot-files and a FIFO, and the
//! folder identity these tests lean on has no Windows source yet.
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};

use common::{Tree, run};
use filesweep_core::{ScanPolicy, Skip, SkipReason};

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

#[test]
fn symlink_to_directory_is_not_entered() {
    let tree = Tree::new();
    tree.file("real/one.txt", b"1");
    tree.file("real/two.txt", b"22");
    let link = tree.symlink("real", "alias");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.skipped, [skip(&link, SkipReason::Link)]);
    assert_eq!(seen.summary.files_seen, 2);
}

#[test]
fn symlink_to_file_is_skipped_not_indexed() {
    let tree = Tree::new();
    tree.file("real.txt", b"content");
    let link = tree.symlink("real.txt", "alias.txt");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.skipped, [skip(&link, SkipReason::Link)]);
    assert_eq!(seen.summary.files_seen, 1);
}

#[test]
fn symlink_loop_terminates() {
    let tree = Tree::new();
    tree.file("a/b/file.txt", b"x");
    let up = tree.symlink("a", "a/b/back-to-a");
    let itself = tree.symlink("a/b/self", "a/b/self");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(
        seen.skipped,
        [skip(&up, SkipReason::Link), skip(&itself, SkipReason::Link)]
    );
    assert_eq!(seen.summary.files_seen, 1);
}

#[test]
fn a_dangling_symlink_is_reported_as_a_link() {
    let tree = Tree::new();
    let link = tree.symlink("nowhere", "broken");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.skipped, [skip(&link, SkipReason::Link)]);
}

#[test]
fn explicit_root_that_is_a_symlink_is_walked() {
    let tree = Tree::new();
    tree.file("real/one.txt", b"1");
    let link = tree.symlink("real", "alias");

    let seen = run(&[&link], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 1);
    assert!(seen.skipped.is_empty(), "skipped: {:?}", seen.skipped);
}

#[test]
fn overlapping_roots_visit_each_file_once() {
    let tree = Tree::new();
    tree.file("top.txt", b"1");
    tree.file("sub/inner.txt", b"22");
    let sub = tree.root().join("sub");

    let seen = run(&[tree.root(), &sub], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 2);
    assert_eq!(seen.skipped, [skip(&sub, SkipReason::AlreadyCovered)]);
}

#[test]
fn a_parent_root_after_its_child_skips_only_the_child() {
    let tree = Tree::new();
    tree.file("top.txt", b"1");
    tree.file("sub/inner.txt", b"22");
    let sub = tree.root().join("sub");

    let seen = run(&[&sub, tree.root()], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 2);
    assert_eq!(seen.skipped, [skip(&sub, SkipReason::AlreadyCovered)]);
}

#[test]
fn same_root_twice_is_covered() {
    let tree = Tree::new();
    tree.file("one.txt", b"1");

    let seen = run(&[tree.root(), tree.root()], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 1);
    assert_eq!(
        seen.skipped,
        [skip(tree.root(), SkipReason::AlreadyCovered)]
    );
}

#[test]
fn root_spelled_many_ways_is_one_directory() {
    let tree = Tree::new();
    tree.file("dir/a.txt", b"1");
    tree.file("dir/b.txt", b"22");
    let plain = tree.root().join("dir");
    let dotted = plain.join("..").join("dir");
    let slashed = PathBuf::from(format!("{}/", plain.display()));

    let seen = run(&[&plain, &dotted, &slashed], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 2);
    let reasons: Vec<&SkipReason> = seen.skipped.iter().map(|skip| &skip.reason).collect();
    assert_eq!(
        reasons,
        [&SkipReason::AlreadyCovered, &SkipReason::AlreadyCovered]
    );
}

#[test]
fn a_link_root_to_a_folder_already_walked_is_covered() {
    let tree = Tree::new();
    tree.file("real/one.txt", b"1");
    let real = tree.root().join("real");
    let link = tree.symlink("real", "alias");

    let seen = run(&[&real, &link], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 1);
    assert_eq!(seen.skipped, [skip(&link, SkipReason::AlreadyCovered)]);
}

#[test]
fn hidden_entries_are_skipped_by_default() {
    let tree = Tree::new();
    tree.file("seen.txt", b"1");
    let file = tree.hidden_file("secret.txt", b"22");
    tree.file(".cache/inside.bin", b"333");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 1);
    assert_eq!(
        seen.skipped,
        [
            skip(tree.root().join(".cache"), SkipReason::Hidden),
            skip(&file, SkipReason::Hidden),
        ]
    );
}

#[test]
fn include_hidden_indexes_them() {
    let tree = Tree::new();
    tree.file("seen.txt", b"1");
    tree.hidden_file("secret.txt", b"22");
    tree.file(".cache/inside.bin", b"333");

    let seen = run(&[tree.root()], everything());

    assert_eq!(seen.summary.files_seen, 3);
    assert!(seen.skipped.is_empty(), "skipped: {:?}", seen.skipped);
}

#[test]
fn a_hidden_root_is_still_walked() {
    let tree = Tree::new();
    tree.file(".chosen/one.txt", b"1");
    let root = tree.root().join(".chosen");

    let seen = run(&[&root], ScanPolicy::default());

    assert_eq!(seen.summary.files_seen, 1);
    assert!(seen.skipped.is_empty(), "skipped: {:?}", seen.skipped);
}

#[test]
fn a_link_named_like_a_hidden_file_is_reported_as_link() {
    let tree = Tree::new();
    tree.file("real.txt", b"1");
    let link = tree.symlink("real.txt", ".alias");

    let hiding = run(&[tree.root()], ScanPolicy::default());
    let showing = run(&[tree.root()], everything());

    assert_eq!(hiding.skipped, [skip(&link, SkipReason::Link)]);
    assert_eq!(showing.skipped, [skip(&link, SkipReason::Link)]);
}

#[test]
fn fifo_is_skipped_as_special_and_never_opened() {
    let tree = Tree::new();
    tree.file("plain.txt", b"1");
    let fifo = tree.root().join("pipe");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("run mkfifo");
    assert!(made.success());

    // Opening a FIFO for reading blocks until something writes to it, so a
    // scan that opened this one would never come back.
    let seen = run(&[tree.root()], everything());

    assert_eq!(seen.skipped, [skip(&fifo, SkipReason::Special)]);
    assert_eq!(seen.summary.files_seen, 1);
}
