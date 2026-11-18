//! One file under several names is one file, not a set of copies.

mod common;

use common::{Tree, pattern, run};
use filesweep_core::{Independence, ScanPolicy};

// Guards the promise that a group holds at least two different physical
// files: offering one name of a file for removal as a "copy" of another name
// would free nothing, and removing all but one name the scan saw could remove
// the only one the user relies on.
#[test]
fn two_names_of_one_file_are_not_a_duplicate() {
    let tree = Tree::new();
    tree.file("original.bin", &pattern(4000));
    tree.hard_link("original.bin", "second-name.bin");
    tree.hard_link("original.bin", "sub/third-name.bin");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert!(seen.groups.is_empty(), "groups: {:?}", seen.groups);
    assert_eq!(seen.summary.candidates, 3);
    assert_eq!(seen.summary.files_hashed, 0);
    assert_eq!(seen.summary.bytes_hashed, 0);
    assert_eq!(seen.summary.reclaimable_bytes, 0);
}

#[test]
fn hard_linked_pair_plus_a_copy_is_two_physical_files() {
    let tree = Tree::new();
    let a = tree.file("a.bin", &pattern(4000));
    let b = tree.hard_link("a.bin", "b.bin");
    let c = tree.file("c.bin", &pattern(4000));

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.groups.len(), 1);
    let group = &seen.groups[0];
    assert_eq!(group.files.len(), 2);
    assert_eq!(group.files[0].paths, [a, b]);
    assert_eq!(group.files[1].paths, [c]);
    assert_eq!(group.independence, Independence::Verified);
    assert_eq!(group.reclaimable_bytes, 4000);
    assert_eq!(seen.summary.files_hashed, 2);
    assert_eq!(seen.summary.bytes_hashed, 8000);
}

#[test]
fn names_outside_the_scan_free_nothing() {
    let tree = Tree::new();
    tree.file("scanned/a.bin", &pattern(4000));
    tree.file("scanned/c.bin", &pattern(4000));
    tree.hard_link("scanned/a.bin", "elsewhere/a-kept.bin");
    tree.hard_link("scanned/c.bin", "elsewhere/c-kept.bin");
    let scanned = tree.root().join("scanned");

    let seen = run(&[&scanned], ScanPolicy::default());

    assert_eq!(seen.groups.len(), 1);
    assert_eq!(seen.groups[0].independence, Independence::Verified);
    assert_eq!(seen.groups[0].reclaimable_bytes, 0);
    assert_eq!(seen.summary.reclaimable_bytes, 0);
}

#[test]
fn only_copies_with_no_name_elsewhere_count_as_reclaimable() {
    let tree = Tree::new();
    tree.file("scanned/a.bin", &pattern(4000));
    tree.file("scanned/c.bin", &pattern(4000));
    tree.file("scanned/d.bin", &pattern(4000));
    tree.hard_link("scanned/a.bin", "elsewhere/a-kept.bin");
    tree.hard_link("scanned/c.bin", "elsewhere/c-kept.bin");
    let scanned = tree.root().join("scanned");

    let seen = run(&[&scanned], ScanPolicy::default());

    // Three files, two of them pinned by a name outside the scan: only d.bin
    // gives its bytes back.
    assert_eq!(seen.groups.len(), 1);
    assert_eq!(seen.groups[0].files.len(), 3);
    assert_eq!(seen.groups[0].reclaimable_bytes, 4000);
}

#[test]
fn stamp_carries_identity_size_modified_and_link_count() {
    let tree = Tree::new();
    let a = tree.file("a.bin", &pattern(4000));
    tree.hard_link("a.bin", "b.bin");
    let c = tree.file("c.bin", &pattern(4000));

    let seen = run(&[tree.root()], ScanPolicy::default());

    let first = &seen.groups[0].files[0].stamp;
    let second = &seen.groups[0].files[1].stamp;
    assert!(first.identity.is_some());
    assert!(second.identity.is_some());
    assert_ne!(first.identity, second.identity);
    assert_eq!((first.size, second.size), (4000, 4000));
    assert_eq!((first.link_count, second.link_count), (Some(2), Some(1)));
    assert_eq!(
        first.modified,
        std::fs::metadata(&a).unwrap().modified().ok()
    );
    assert_eq!(
        second.modified,
        std::fs::metadata(&c).unwrap().modified().ok()
    );
}
