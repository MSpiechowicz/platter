use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::find;
use crate::error::{ScanError, ScanLimit};
use crate::model::{NodeId, ScanWarning, Tree};
use crate::scan::{CancelToken, ScanConfig, scan};
use crate::testing::{FakeFs, NOW, scan_fake};

fn wide_fs(dirs: usize) -> FakeFs {
    let mut fs = FakeFs::new();
    for i in 0..dirs {
        fs.file(&format!("/d{i:03}/sub/f.txt"), 10 + i as u64);
        fs.file(&format!("/d{i:03}/g.rs"), 5);
    }
    fs
}

fn scan_with_threads(fs: &FakeFs, threads: usize) -> Tree {
    let config = ScanConfig {
        threads: Some(threads),
        now: Some(NOW),
        ..ScanConfig::default()
    };
    scan(fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).unwrap()
}

fn fingerprint(tree: &Tree) -> Vec<(Vec<u8>, u64, u64, u32)> {
    (0..tree.len() as u32)
        .map(|i| {
            let id = NodeId(i);
            (
                tree.name(id).to_vec(),
                tree.allocated(id),
                tree.apparent(id),
                tree.parent(id).map_or(u32::MAX, |p| p.0),
            )
        })
        .collect()
}

#[test]
fn result_does_not_depend_on_the_number_of_threads() {
    let mut fs = wide_fs(60);
    fs.hardlink("/d001/g.rs", "/d050/link.rs");

    let single = scan_with_threads(&fs, 1);
    let many = scan_with_threads(&fs, 12);

    assert_eq!(single.totals(), many.totals());
    assert_eq!(fingerprint(&single), fingerprint(&many));
}

#[test]
fn cancelling_mid_scan_returns_cancelled_and_stops_reading() {
    let mut fs = wide_fs(100);
    let cancel = CancelToken::new();
    let reads = std::sync::Arc::new(AtomicUsize::new(0));

    let (token, counter) = (cancel.clone(), reads.clone());
    fs.on_read_dir(move |_| {
        if counter.fetch_add(1, Ordering::SeqCst) == 10 {
            token.cancel();
        }
    });

    let config = ScanConfig {
        threads: Some(2),
        ..ScanConfig::default()
    };
    let result = scan(&fs, Path::new("/"), &config, &cancel, &|_| {});

    assert!(matches!(result, Err(ScanError::Cancelled)));
    assert!(
        fs.read_dirs().len() < 201,
        "stopped before reading all 201 directories"
    );
}

#[test]
fn a_pre_cancelled_scan_reads_nothing() {
    let fs = wide_fs(5);
    let cancel = CancelToken::new();
    cancel.cancel();

    let result = scan(&fs, Path::new("/"), &ScanConfig::default(), &cancel, &|_| {});

    assert!(matches!(result, Err(ScanError::Cancelled)));
    assert!(fs.read_dirs().is_empty());
}

#[test]
fn progress_is_throttled_and_reports_the_current_path() {
    let mut fs = wide_fs(40);
    fs.on_read_dir(|_| std::thread::sleep(Duration::from_millis(3)));

    let calls = AtomicUsize::new(0);
    let last_path = std::sync::Mutex::new(String::new());
    let config = ScanConfig {
        threads: Some(2),
        progress_interval: Duration::from_millis(20),
        ..ScanConfig::default()
    };

    let started = Instant::now();
    scan(&fs, Path::new("/"), &config, &CancelToken::new(), &|progress| {
        calls.fetch_add(1, Ordering::SeqCst);
        *last_path.lock().unwrap() = progress.current_path.clone();
    })
    .unwrap();

    let max_expected = started.elapsed().as_millis() as usize / 20 + 1;
    let calls = calls.load(Ordering::SeqCst);
    assert!(
        calls >= 1,
        "at least one snapshot for a scan lasting {:?}",
        started.elapsed()
    );
    assert!(
        calls <= max_expected,
        "{calls} snapshots exceed the throttle ({max_expected})"
    );
    assert!(last_path.lock().unwrap().starts_with('/'));
}

#[test]
fn a_missing_root_and_a_file_root_are_reported() {
    let mut fs = FakeFs::new();
    fs.file("/f.txt", 1);
    let run = |path: &str| {
        scan(
            &fs,
            Path::new(path),
            &ScanConfig::default(),
            &CancelToken::new(),
            &|_| {},
        )
    };

    assert!(matches!(run("/nope"), Err(ScanError::Root { .. })));
    assert!(matches!(run("/f.txt"), Err(ScanError::NotADirectory(_))));
}

#[test]
fn a_root_whose_identity_changed_since_it_was_resolved_is_refused() {
    let mut fs = FakeFs::new();
    fs.file("/data/f.txt", 1);
    let (dev, ino) = fs.identity("/data");
    let run = |identity| {
        let config = ScanConfig {
            root_identity: identity,
            ..ScanConfig::default()
        };
        scan(&fs, Path::new("/data"), &config, &CancelToken::new(), &|_| {})
    };

    assert!(run(Some((dev, ino))).is_ok());
    assert!(run(None).is_ok());
    assert!(matches!(run(Some((dev, ino + 1))), Err(ScanError::Root { .. })));
    assert!(matches!(run(Some((dev + 1, ino))), Err(ScanError::Root { .. })));
}

#[test]
fn deep_trees_are_scanned_without_overflowing_the_stack() {
    let mut fs = FakeFs::new();
    let path: String = (0..300).map(|i| format!("/d{i}")).collect();
    fs.file(&format!("{path}/leaf.bin"), 42);

    let tree = scan_fake(&fs, false);

    assert_eq!(tree.totals().allocated, 42);
    assert_eq!(tree.totals().dirs, 300);
    assert_eq!(tree.ancestry(find(&tree, &format!("{path}/leaf.bin"))).len(), 302);
}

#[test]
fn absurdly_deep_trees_are_cut_off_and_counted_as_errors() {
    let mut fs = FakeFs::new();
    let path: String = (0..1100).map(|i| format!("/d{i}")).collect();
    fs.file(&format!("{path}/leaf.bin"), 42);

    let tree = scan_fake(&fs, false);

    assert!(tree.totals().errors >= 1);
    assert_eq!(tree.totals().allocated, 0, "the leaf lies below the depth limit");

    let flagged: Vec<u32> = (0..tree.len() as u32)
        .filter(|&index| tree.is_unreadable(NodeId(index)))
        .collect();
    assert_eq!(
        flagged.len(),
        1,
        "only the first directory past the limit is flagged"
    );
    assert_eq!(
        tree.ancestry(NodeId(flagged[0])).len(),
        1026,
        "root + 1025 levels"
    );
}

#[test]
fn a_directory_that_is_its_own_ancestor_is_not_entered() {
    let mut fs = FakeFs::new();
    fs.file("/a/f.bin", 10)
        .file("/a/b/g.bin", 20)
        .file("/a/b/loop/never.bin", 99);
    // `/a/b/loop` has the identity of `/a`: a bind mount or hard-linked directory pointing upwards.
    let (dev, ino) = fs.identity("/a");
    fs.set_identity("/a/b/loop", dev, ino, 1);

    let tree = scan_fake(&fs, false);

    assert_eq!(tree.totals().allocated, 30, "the loop is not read again");
    assert_eq!(tree.totals().errors, 1);
    assert_eq!(tree.child_count(find(&tree, "a/b/loop")), 0);
    assert!(
        tree.is_unreadable(find(&tree, "a/b/loop")),
        "a loop cut-off counts as unreadable"
    );
    assert!(!tree.is_unreadable(find(&tree, "a/b")));
    assert_eq!(fs.read_dirs().iter().filter(|p| *p == "/a/b/loop").count(), 0);
}

#[test]
fn equal_identities_in_different_branches_are_not_loops() {
    let mut fs = FakeFs::new();
    fs.file("/a/x/f.bin", 1).file("/b/y/g.bin", 2);
    let (dev, ino) = fs.identity("/a/x");
    fs.set_identity("/b/y", dev, ino, 1);

    let tree = scan_fake(&fs, false);

    assert_eq!(tree.totals().errors, 0);
    assert_eq!(tree.totals().allocated, 3);
}

fn entry_cap(max_entries: Option<u64>) -> ScanConfig {
    ScanConfig {
        threads: Some(1),
        now: Some(NOW),
        max_entries,
        ..ScanConfig::default()
    }
}

#[test]
fn the_walk_aborts_once_the_entry_cap_would_be_exceeded() {
    let fs = wide_fs(50); // 50 * (dir + sub + f + g.rs) = 200 entries
    let run = |cap| scan(&fs, Path::new("/"), &entry_cap(cap), &CancelToken::new(), &|_| {});

    assert!(matches!(
        run(Some(100)),
        Err(ScanError::TooLarge {
            limit: ScanLimit::Entries(100)
        })
    ));
    assert!(run(Some(200)).is_ok(), "exactly at the cap is fine");
    assert!(run(None).is_ok());
    assert!(
        fs.read_dirs().len() < 3 * 101 + 1,
        "the first aborted scan did not read the whole tree"
    );
}

#[test]
fn the_default_entry_cap_is_thirty_million() {
    assert_eq!(ScanConfig::default().max_entries, Some(30_000_000));
    assert_eq!(crate::scan::DEFAULT_MAX_ENTRIES, 30_000_000);
}

#[test]
fn too_large_tells_the_user_what_happened_in_plain_words_and_names_the_limit_that_was_hit() {
    let entries = ScanError::TooLarge {
        limit: ScanLimit::Entries(30_000_000),
    }
    .to_string();
    assert_eq!(
        entries,
        "This folder has more entries than Platter can hold in memory (limit 30000000 entries). Scan a smaller folder."
    );

    let memory = ScanError::TooLarge {
        limit: ScanLimit::Memory(6 << 30),
    }
    .to_string();
    assert_eq!(
        memory,
        "This folder needs more memory than Platter may use for a scan (limit 6 GiB, half of the memory that is free). Scan a smaller folder."
    );

    // A machine with little free memory is told so: the folder is not necessarily big.
    let little = ScanError::TooLarge {
        limit: ScanLimit::Memory(100 << 20),
    }
    .to_string();
    assert_eq!(
        little,
        "There is very little free memory right now, so Platter may use only 100 MiB for a scan, which fits only a small folder. Close other programs or scan a smaller folder."
    );
}

#[test]
fn a_single_huge_directory_is_not_read_beyond_the_remaining_budget() {
    let mut fs = FakeFs::new();
    for i in 0..1_000 {
        fs.file(&format!("/big/f{i:04}"), 1);
    }

    let result = scan(
        &fs,
        Path::new("/"),
        &entry_cap(Some(100)),
        &CancelToken::new(),
        &|_| {},
    );

    assert!(matches!(
        result,
        Err(ScanError::TooLarge {
            limit: ScanLimit::Entries(100)
        })
    ));
    assert!(
        fs.largest_listing() <= 100,
        "the source was refused room at the cap and stopped, it returned {}",
        fs.largest_listing()
    );
}

#[test]
fn the_room_a_source_may_use_shrinks_with_what_was_already_collected() {
    let mut fs = FakeFs::new();
    for i in 0..30 {
        fs.file(&format!("/a/f{i:02}"), 1);
    }
    for i in 0..30 {
        fs.file(&format!("/b/g{i:02}"), 1);
    }

    // `/` has two entries, `/a` brings 30 (32 so far), so `/b` may only add 8 more.
    let result = scan(
        &fs,
        Path::new("/"),
        &entry_cap(Some(40)),
        &CancelToken::new(),
        &|_| {},
    );

    assert!(matches!(
        result,
        Err(ScanError::TooLarge {
            limit: ScanLimit::Entries(40)
        })
    ));
    let listed: usize = 2 + 30 + 8;
    assert!(
        fs.largest_listing() <= 30 && fs.read_dirs().len() == 3,
        "no listing grew past the room that was left (the whole scan held at most {listed}), largest {}",
        fs.largest_listing()
    );
}

fn name_cap(max_name_bytes: Option<u64>) -> ScanConfig {
    ScanConfig {
        max_name_bytes,
        ..entry_cap(None)
    }
}

#[test]
fn the_walk_aborts_as_soon_as_the_name_bytes_would_pass_the_limit() {
    let mut fs = FakeFs::new();
    for dir in 0..20 {
        for file in 0..10 {
            fs.file(&format!("/d{dir:02}/file-{file}.txt"), 1);
        }
    }
    // 20 directory names of 3 bytes, 200 file names of 10 bytes.
    let total = 20 * 3 + 200 * 10;
    let run = |limit| {
        scan(
            &fs,
            Path::new("/"),
            &name_cap(limit),
            &CancelToken::new(),
            &|_| {},
        )
    };

    assert!(matches!(run(Some(total / 4)), Err(ScanError::TooLarge { .. })));
    assert!(
        fs.read_dirs().len() < 10,
        "the scan stopped long before reading every directory: {}",
        fs.read_dirs().len()
    );
    assert!(matches!(run(Some(total - 1)), Err(ScanError::TooLarge { .. })));
    assert!(run(Some(total)).is_ok(), "exactly at the limit is fine");
    assert!(run(None).is_ok());
}

#[test]
fn a_source_warning_is_kept_on_the_tree_and_counted_as_an_error() {
    let mut fs = FakeFs::new();
    fs.file("/a/f", 5);
    fs.warn(ScanWarning::MountTableUnavailable);

    let tree = scan_fake(&fs, false);

    assert_eq!(tree.warnings(), [ScanWarning::MountTableUnavailable]);
    assert_eq!(tree.totals().errors, 1);
    assert_eq!(tree.totals().allocated, 5, "the scan itself is complete");

    let clean = scan_fake(&FakeFs::new(), false);
    assert!(clean.warnings().is_empty());
    assert_eq!(clean.totals().errors, 0);
}

#[test]
fn the_arena_is_sized_exactly_so_no_vector_keeps_slack_or_grows_by_doubling() {
    let mut fs = wide_fs(40);
    fs.hardlink("/d001/g.rs", "/d002/link.rs");
    fs.file("/d003/a-much-longer-file-name-than-the-others.bin", 1);

    for threads in [1, 6] {
        let tree = scan_with_threads(&fs, threads);

        assert_eq!(
            tree.names.capacity(),
            tree.names.len(),
            "names, {threads} threads"
        );
        assert_eq!(
            tree.nodes.capacity(),
            tree.nodes.len(),
            "nodes, {threads} threads"
        );
        assert_eq!(tree.dirs.capacity(), tree.dirs.len(), "dirs, {threads} threads");
    }
}
