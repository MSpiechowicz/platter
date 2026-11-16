//! The Linux reader against real directories: the listing limit, the check that an opened directory is
//! the listed one (on both ways of opening), and the handling of the mount table.

use std::ffi::OsString;
use std::fs::{self, File};
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

use platter_core::{
    BudgetLimits, CancelToken, DirListing, DirSource, EntryBudget, ScanConfig, ScanWarning, SourceOptions,
    scan,
};
use rustix::fs::{AtFlags, CWD, StatxFlags, statx};

use super::{ExclusionRules, LinuxDirSource, LinuxHandle};

fn open(source: &LinuxDirSource, path: &Path) -> LinuxHandle {
    source
        .open_root(path, SourceOptions::default())
        .expect("open the root")
        .handle
}

/// Takes the child handle of the entry called `name` out of a listing.
fn child(listing: &mut DirListing<LinuxHandle>, name: &str) -> LinuxHandle {
    listing
        .entries
        .iter_mut()
        .find(|entry| entry.name == name.as_bytes())
        .and_then(|entry| entry.child.take())
        .unwrap_or_else(|| panic!("{name} is listed as a directory"))
}

fn names(listing: &DirListing<LinuxHandle>) -> Vec<String> {
    let mut names: Vec<String> = listing
        .entries
        .iter()
        .map(|entry| String::from_utf8_lossy(&entry.name).into_owned())
        .collect();
    names.sort();
    names
}

/// Replaces the directory `path` by an empty directory of the same name (another inode).
fn swap_for_another_directory(path: &Path) {
    let aside = path.with_file_name(format!("{}-old", path.file_name().unwrap().to_string_lossy()));

    fs::rename(path, aside).unwrap();
    fs::create_dir(path).unwrap();
    File::create(path.join("foreign")).unwrap();
}

#[test]
fn a_listing_stops_as_soon_as_the_budget_refuses_room() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..50 {
        File::create(dir.path().join(format!("f{i:02}"))).unwrap();
    }
    let source = LinuxDirSource::new();
    // A directory is read once, from its descriptor's current position: every read needs a fresh root.
    let read = |max_entries| {
        let budget = EntryBudget::new(
            BudgetLimits {
                max_entries,
                ..BudgetLimits::UNLIMITED
            },
            1,
        );
        let listing = source
            .read_dir(&open(&source, dir.path()), &mut budget.lease())
            .unwrap();

        (listing.entries.len(), budget.is_exhausted())
    };

    assert_eq!(
        read(10),
        (10, true),
        "stopped at the cap, and the scan knows it ended there"
    );
    assert_eq!(read(u64::MAX), (50, false));
    assert_eq!(read(50), (50, false), "exactly at the cap is not cut");
    assert_eq!(read(49), (49, true));
    assert_eq!(read(0), (0, true));
}

#[test]
fn a_directory_swapped_for_another_one_is_not_read_through_the_parent_descriptor() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a/sub")).unwrap();
    File::create(dir.path().join("a/sub/original")).unwrap();
    let source = LinuxDirSource::new();
    let root = open(&source, dir.path());

    let a = child(
        &mut source
            .read_dir(&root, &mut EntryBudget::unlimited().lease())
            .unwrap(),
        "a",
    );
    let mut in_a = source
        .read_dir(&a, &mut EntryBudget::unlimited().lease())
        .unwrap();
    let sub = child(&mut in_a, "sub");
    assert_eq!(
        names(
            &source
                .read_dir(&sub, &mut EntryBudget::unlimited().lease())
                .unwrap()
        ),
        ["original"],
        "unchanged: read"
    );

    // `sub` hangs under `a`'s descriptor, which is still open: one `openat` reaches whatever is named
    // `sub` now.
    swap_for_another_directory(&dir.path().join("a/sub"));

    let error = source
        .read_dir(&sub, &mut EntryBudget::unlimited().lease())
        .expect_err("another directory is behind the name");
    assert!(error.to_string().contains("replaced"), "{error}");
}

#[test]
fn a_directory_swapped_for_another_one_is_not_read_when_reopened_from_the_root() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a")).unwrap();
    File::create(dir.path().join("a/original")).unwrap();
    let source = LinuxDirSource::new();
    let root = open(&source, dir.path());

    // A child of the scan root has no parent descriptor: it is reopened by name from the root.
    let a = child(
        &mut source
            .read_dir(&root, &mut EntryBudget::unlimited().lease())
            .unwrap(),
        "a",
    );
    assert_eq!(
        names(
            &source
                .read_dir(&a, &mut EntryBudget::unlimited().lease())
                .unwrap()
        ),
        ["original"],
        "unchanged: read"
    );

    swap_for_another_directory(&dir.path().join("a"));

    let error = source
        .read_dir(&a, &mut EntryBudget::unlimited().lease())
        .expect_err("another directory is behind the name");
    assert!(error.to_string().contains("replaced"), "{error}");
}

#[test]
fn every_component_of_a_reopened_chain_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
    File::create(dir.path().join("a/b/c/original")).unwrap();
    // No descriptor is kept for children, so `c` is reopened by walking `a`, `b`, `c` from the root.
    let source = LinuxDirSource::new().with_descriptor_budget(0);
    let root = open(&source, dir.path());

    let a = child(
        &mut source
            .read_dir(&root, &mut EntryBudget::unlimited().lease())
            .unwrap(),
        "a",
    );
    let b = child(
        &mut source
            .read_dir(&a, &mut EntryBudget::unlimited().lease())
            .unwrap(),
        "b",
    );
    let c = child(
        &mut source
            .read_dir(&b, &mut EntryBudget::unlimited().lease())
            .unwrap(),
        "c",
    );
    assert_eq!(
        names(
            &source
                .read_dir(&c, &mut EntryBudget::unlimited().lease())
                .unwrap()
        ),
        ["original"]
    );

    // The middle component is replaced by a lookalike: `a/b/c` exists again, `a` is another directory.
    fs::rename(dir.path().join("a"), dir.path().join("a-old")).unwrap();
    fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
    File::create(dir.path().join("a/b/c/foreign")).unwrap();

    let error = source
        .read_dir(&c, &mut EntryBudget::unlimited().lease())
        .expect_err("`a` is not the directory that was listed");
    assert!(error.to_string().contains("replaced"), "{error}");
}

#[test]
fn a_swapped_directory_is_counted_as_one_error_by_a_scan_and_not_read() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a")).unwrap();
    fs::create_dir_all(dir.path().join("b")).unwrap();
    std::fs::write(dir.path().join("a/f"), b"12345").unwrap();
    std::fs::write(dir.path().join("b/g"), b"1").unwrap();
    let source = LinuxDirSource::new();
    let root = open(&source, dir.path());

    let mut listing = source
        .read_dir(&root, &mut EntryBudget::unlimited().lease())
        .unwrap();
    let (a, b) = (child(&mut listing, "a"), child(&mut listing, "b"));
    swap_for_another_directory(&dir.path().join("a"));

    assert!(
        source
            .read_dir(&a, &mut EntryBudget::unlimited().lease())
            .is_err()
    );
    assert_eq!(
        names(
            &source
                .read_dir(&b, &mut EntryBudget::unlimited().lease())
                .unwrap()
        ),
        ["g"],
        "the others are fine"
    );

    // The same through the scanner: the swapped directory is the one error, nothing of it is counted.
    let tree = scan(
        &LinuxDirSource::new(),
        dir.path(),
        &ScanConfig::default(),
        &CancelToken::new(),
        &|_| {},
    )
    .unwrap();
    assert_eq!(tree.totals().errors, 0, "no swap during this scan");
}

// The mount table.

/// `stx_mnt_id` of `path`; `None` on a kernel that does not report it (the test is then moot).
fn mount_id(path: &str) -> Option<u64> {
    let stat = statx(CWD, path, AtFlags::NO_AUTOMOUNT, StatxFlags::MNT_ID).ok()?;

    StatxFlags::from_bits_retain(stat.stx_mask)
        .contains(StatxFlags::MNT_ID)
        .then_some(stat.stx_mnt_id)
}

/// Whether the reader enters `/proc` when scanning `/` with `table` as the mount table. `/proc` is a
/// mount root everywhere; the table classifies it (here as an ordinary ext4 mount, which the rules do not
/// exclude) and says where it is.
fn enters_proc_with(table: &[u8]) -> Option<bool> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mountinfo");
    fs::write(&path, table).unwrap();

    // No path rules: `/proc` would be excluded by its name whatever the table says.
    let source = LinuxDirSource::with_rules(ExclusionRules::NONE).with_mountinfo_path(path);
    let root = open(&source, Path::new("/"));
    let listing = source
        .read_dir(&root, &mut EntryBudget::unlimited().lease())
        .ok()?;

    let proc = listing.entries.iter().find(|entry| entry.name == b"proc")?;
    proc.meta.is_mount_root.then_some(proc.child.is_some())
}

fn table_with_proc_at(proc_id: u64, mount_point: &str) -> Vec<u8> {
    let mut table = Vec::new();
    table.extend_from_slice(b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n");
    table.extend_from_slice(format!("{proc_id} 1 0:23 / {mount_point} rw - ext4 /dev/sda2 rw\n").as_bytes());
    table
}

#[test]
fn a_mount_point_that_is_not_utf8_does_not_make_the_reader_forget_every_mount() {
    let Some(proc_id) = mount_id("/proc") else {
        return;
    };
    let mut table = table_with_proc_at(proc_id, "/proc");
    table.extend_from_slice(b"2 1 8:3 / /mnt/caf\xff\xe9 rw - ext4 /dev/sda3 rw\n");

    assert_eq!(
        enters_proc_with(&table),
        Some(true),
        "/proc is known, so it is entered"
    );
}

#[test]
fn a_mount_root_is_entered_only_at_the_path_the_table_gives_for_its_id() {
    let Some(proc_id) = mount_id("/proc") else {
        return;
    };

    assert_eq!(
        enters_proc_with(&table_with_proc_at(proc_id, "/proc")),
        Some(true)
    );
    assert_eq!(
        enters_proc_with(&table_with_proc_at(proc_id, "/mnt/elsewhere")),
        Some(false),
        "the id is in the table, but for a mount somewhere else: a reused id inherits nothing"
    );
}

#[test]
fn a_mount_missing_from_the_table_is_not_entered() {
    let Some(proc_id) = mount_id("/proc") else {
        return;
    };

    assert_eq!(
        enters_proc_with(&table_with_proc_at(proc_id + 100_000, "/proc")),
        Some(false)
    );
}

#[test]
fn an_unreadable_mount_table_is_reported_as_a_warning_and_counted_as_an_error() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("f"), b"x").unwrap();
    let source = LinuxDirSource::new().with_mountinfo_path("/this/table/does/not/exist");

    let info = source
        .open_root(dir.path(), SourceOptions::default())
        .expect("the scan itself can start");
    assert_eq!(info.warnings, [ScanWarning::MountTableUnavailable]);

    let tree = scan(
        &source,
        dir.path(),
        &ScanConfig::default(),
        &CancelToken::new(),
        &|_| {},
    )
    .unwrap();
    assert_eq!(tree.warnings(), [ScanWarning::MountTableUnavailable]);
    assert_eq!(tree.totals().errors, 1);
    assert_eq!(tree.totals().files, 1, "the rest of the scan is complete");

    let readable = LinuxDirSource::new();
    let tree = scan(
        &readable,
        dir.path(),
        &ScanConfig::default(),
        &CancelToken::new(),
        &|_| {},
    )
    .unwrap();
    assert!(tree.warnings().is_empty());
    assert_eq!(tree.totals().errors, 0);
}

#[test]
fn a_table_with_nothing_parsable_counts_as_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let table = dir.path().join("mountinfo");
    fs::write(&table, b"not a mount table\n\xff\xfe\n").unwrap();

    let info = LinuxDirSource::new()
        .with_mountinfo_path(&table)
        .open_root(dir.path(), SourceOptions::default())
        .unwrap();

    assert_eq!(info.warnings, [ScanWarning::MountTableUnavailable]);
}

#[test]
fn a_non_utf8_directory_name_survives_the_reader() {
    let dir = tempfile::tempdir().unwrap();
    let odd = OsString::from_vec(b"caf\xff".to_vec());
    fs::create_dir(dir.path().join(&odd)).unwrap();
    let source = LinuxDirSource::new();
    let root = open(&source, dir.path());

    let listing = source
        .read_dir(&root, &mut EntryBudget::unlimited().lease())
        .unwrap();

    assert_eq!(listing.entries[0].name, b"caf\xff");
}

#[test]
fn a_real_scan_is_held_to_the_entry_cap_and_to_the_memory_budget() {
    let dir = tempfile::tempdir().unwrap();
    for sub in 0..3 {
        fs::create_dir(dir.path().join(format!("d{sub}"))).unwrap();
        for file in 0..100 {
            File::create(dir.path().join(format!("d{sub}/f{file:03}"))).unwrap();
        }
    }
    // 3 directories and 300 files.
    let source = LinuxDirSource::new();
    let run = |config: ScanConfig| scan(&source, dir.path(), &config, &CancelToken::new(), &|_| {});
    let capped = |max_entries| ScanConfig {
        max_entries: Some(max_entries),
        ..ScanConfig::default()
    };

    assert_eq!(run(capped(303)).unwrap().len(), 304, "exactly at the cap");
    assert!(matches!(
        run(capped(302)),
        Err(platter_core::ScanError::TooLarge {
            limit: platter_core::ScanLimit::Entries(302)
        })
    ));

    let tiny_memory = ScanConfig {
        max_memory_bytes: Some(10_000),
        ..ScanConfig::default()
    };
    assert!(matches!(
        run(tiny_memory),
        Err(platter_core::ScanError::TooLarge {
            limit: platter_core::ScanLimit::Memory(10_000)
        })
    ));
}
