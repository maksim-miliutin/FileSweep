mod common;

use std::path::PathBuf;

use common::{Tree, pattern, run};
use filesweep_core::{
    DuplicateGroup, Outcome, Progress, ScanControl, ScanPolicy, ScanRequest, scan,
};

fn paths_of(group: &DuplicateGroup) -> Vec<PathBuf> {
    group
        .files
        .iter()
        .flat_map(|file| file.paths.iter().cloned())
        .collect()
}

#[test]
fn identical_bytes_with_different_names_form_one_group() {
    let tree = Tree::new();
    let content = pattern(3000);
    let copies = ["a.txt", "b.jpg", "noext", "ё.bin"].map(|name| tree.file(name, &content));
    tree.file("other.txt", b"something else");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.groups.len(), 1);
    let group = &seen.groups[0];
    assert_eq!(group.size, 3000);
    assert_eq!(group.files.len(), 4);
    assert_eq!(paths_of(group), copies);
    assert!(group.files.iter().all(|file| file.stamp.size == 3000));
}

#[test]
fn published_blake3_vectors() {
    // Lengths and digests from the official BLAKE3 test vectors. Their input
    // is the repeating 251-byte sequence that `pattern` produces.
    let vectors = [
        (
            1,
            "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
        ),
        (
            3,
            "e1be4d7a8ab5560aa4199eea339849ba8e293d55ca0a81006726d184519e647f",
        ),
        (
            1025,
            "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444",
        ),
    ];
    let tree = Tree::new();
    for (len, _) in vectors {
        tree.file(format!("{len}/first"), &pattern(len));
        tree.file(format!("{len}/second"), &pattern(len));
    }

    let seen = run(&[tree.root()], ScanPolicy::default());

    let mut found: Vec<(u64, String)> = seen
        .groups
        .iter()
        .map(|group| (group.size, group.hash.to_string()))
        .collect();
    found.sort();
    let expected: Vec<(u64, String)> = vectors
        .iter()
        .map(|(len, digest)| (*len as u64, digest.to_string()))
        .collect();
    assert_eq!(found, expected);
}

#[test]
fn same_size_different_last_byte_makes_no_group() {
    let tree = Tree::new();
    let mut altered = pattern(5000);
    altered[4999] ^= 0xFF;
    tree.file("one", &pattern(5000));
    tree.file("two", &altered);

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert!(seen.groups.is_empty());
    assert_eq!(seen.summary.candidates, 2);
    assert_eq!(seen.summary.files_hashed, 2);
    assert_eq!(seen.summary.groups, 0);
}

#[test]
fn unique_sizes_are_never_read() {
    let tree = Tree::new();
    tree.file("ten", &pattern(10));
    tree.file("twenty", &pattern(20));
    tree.file("thirty", &pattern(30));

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert!(seen.groups.is_empty());
    assert_eq!(seen.summary.files_seen, 3);
    assert_eq!(seen.summary.candidates, 0);
    assert_eq!(seen.summary.files_hashed, 0);
    assert_eq!(seen.summary.bytes_hashed, 0);
}

#[test]
fn duplicates_across_folders_and_roots_are_found() {
    let first = Tree::new();
    let second = Tree::new();
    let a = first.file("deep/down/copy.bin", &pattern(777));
    let b = first.file("other/place/renamed.dat", &pattern(777));
    let c = second.file("elsewhere.bin", &pattern(777));

    let seen = run(&[first.root(), second.root()], ScanPolicy::default());

    assert_eq!(seen.groups.len(), 1);
    assert_eq!(paths_of(&seen.groups[0]), [a, b, c]);
}

#[test]
fn empty_files_are_counted_not_grouped() {
    let tree = Tree::new();
    tree.file("empty-1", b"");
    tree.file("empty-2", b"");
    tree.file("sub/empty-3", b"");

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert!(seen.groups.is_empty());
    assert_eq!(seen.summary.empty_files, 3);
    assert_eq!(seen.summary.files_seen, 0);
    assert_eq!(seen.summary.candidates, 0);
}

#[test]
fn groups_come_largest_first_and_files_in_walk_order() {
    let tree = Tree::new();
    let small = ["a-small/1", "a-small/2"].map(|name| tree.file(name, &pattern(10)));
    let medium = ["m/1", "m/2", "m/3"].map(|name| tree.file(name, &pattern(300)));
    let large = ["z-large/1", "z-large/2"].map(|name| tree.file(name, &pattern(5000)));

    let seen = run(&[tree.root()], ScanPolicy::default());

    let sizes: Vec<u64> = seen.groups.iter().map(|group| group.size).collect();
    assert_eq!(sizes, [5000, 300, 10]);
    assert_eq!(paths_of(&seen.groups[0]), large);
    assert_eq!(paths_of(&seen.groups[1]), medium);
    assert_eq!(paths_of(&seen.groups[2]), small);
}

#[test]
fn two_contents_of_one_size_form_two_groups() {
    let tree = Tree::new();
    let this = vec![b'a'; 100];
    let that = vec![b'b'; 100];
    let first = ["1", "3"].map(|name| tree.file(name, &this));
    let second = ["2", "4"].map(|name| tree.file(name, &that));

    let seen = run(&[tree.root()], ScanPolicy::default());

    assert_eq!(seen.groups.len(), 2);
    assert_eq!(paths_of(&seen.groups[0]), first);
    assert_eq!(paths_of(&seen.groups[1]), second);
    assert_ne!(seen.groups[0].hash, seen.groups[1].hash);
}

#[test]
fn hashing_progress_starts_ends_and_reaches_totals() {
    let tree = Tree::new();
    for name in ["ten-1", "ten-2", "ten-3"] {
        tree.file(name, &pattern(10));
    }
    for name in ["five-1", "five-2"] {
        tree.file(name, &pattern(5));
    }
    tree.file("alone", &pattern(7));

    let seen = run(&[tree.root()], ScanPolicy::default());

    let hashing: Vec<[u64; 4]> = seen
        .progress
        .iter()
        .filter_map(|step| match step {
            Progress::Hashing {
                files_done,
                files_total,
                bytes_done,
                bytes_total,
            } => Some([*files_done, *files_total, *bytes_done, *bytes_total]),
            Progress::Walking { .. } => None,
        })
        .collect();
    assert_eq!(hashing.first(), Some(&[0, 5, 0, 40]));
    assert_eq!(hashing.last(), Some(&[5, 5, 40, 40]));
    assert!(hashing.iter().all(|step| step[1] == 5 && step[3] == 40));
    assert!(
        hashing
            .windows(2)
            .all(|pair| pair[0][0] <= pair[1][0] && pair[0][2] <= pair[1][2])
    );

    let first_hashing = seen
        .progress
        .iter()
        .position(|step| matches!(step, Progress::Hashing { .. }))
        .expect("the hashing phase reports progress");
    assert!(
        seen.progress[first_hashing..]
            .iter()
            .all(|step| matches!(step, Progress::Hashing { .. })),
        "the walk reported after hashing began"
    );
}

#[test]
fn summary_counts_add_up() {
    let tree = Tree::new();
    for name in ["big-1", "big-2", "big-3"] {
        tree.file(name, &pattern(1000));
    }
    for name in ["mid-1", "mid-2"] {
        tree.file(name, &pattern(200));
    }
    tree.file("same-size-1", &[1; 50]);
    tree.file("same-size-2", &[2; 50]);
    tree.file("alone", &pattern(7));
    tree.file("empty", b"");

    let seen = run(&[tree.root()], ScanPolicy::default());

    let summary = &seen.summary;
    assert_eq!(summary.outcome, Outcome::Completed);
    assert_eq!(summary.files_seen, 8);
    assert_eq!(summary.empty_files, 1);
    assert_eq!(summary.candidates, 7);
    assert_eq!(summary.files_hashed, 7);
    assert_eq!(summary.bytes_hashed, 3 * 1000 + 2 * 200 + 2 * 50);
    assert_eq!(summary.groups, 2);
    assert_eq!(seen.groups.len(), 2);
    assert_eq!(summary.skipped, 0);
    assert_eq!(
        summary.reclaimable_bytes,
        seen.groups
            .iter()
            .map(|group| group.reclaimable_bytes)
            .sum::<u64>()
    );
    // One copy of each group stays: two of the big ones and one of the mid ones can go.
    #[cfg(unix)]
    assert_eq!(summary.reclaimable_bytes, 2 * 1000 + 200);
}

#[test]
fn sink_runs_on_the_calling_thread() {
    let tree = Tree::new();
    tree.file("one", &pattern(64));
    tree.file("two", &pattern(64));
    let request = ScanRequest {
        roots: vec![tree.root().to_path_buf()],
        policy: ScanPolicy::default(),
    };
    let caller = std::thread::current().id();
    let mut events = 0;

    scan(&request, &ScanControl::new(), &mut |_| {
        assert_eq!(std::thread::current().id(), caller);
        events += 1;
    })
    .expect("the request names a root");

    assert!(events > 0);
}
