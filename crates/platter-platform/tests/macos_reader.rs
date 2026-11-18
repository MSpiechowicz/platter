//! The macOS reader against real directories: the sizes it reports, links never followed, a folder the
//! system refuses, the check that an opened directory is the listed one (on both ways of opening), and
//! the exclusions that apply on the real system volume.

#![cfg(target_os = "macos")]

use std::fs::{self, File, Permissions};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;

use platter_core::{
    CancelToken, DirListing, DirSource, EntryBudget, EntryKind, NodeId, NodeKind, ScanConfig, SkipReason,
    SourceOptions, Tree, scan,
};
use platter_platform::macos::MacDirSource;
use platter_platform::macos::reader::MacHandle;

fn open(source: &MacDirSource, path: &Path) -> MacHandle {
    source
        .open_root(path, SourceOptions::default())
        .expect("open the root")
        .handle
}

fn list(source: &MacDirSource, handle: &MacHandle) -> std::io::Result<DirListing<MacHandle>> {
    source.read_dir(handle, &mut EntryBudget::unlimited().lease())
}

/// Takes the child handle of the entry called `name` out of a listing.
fn child(listing: &mut DirListing<MacHandle>, name: &str) -> MacHandle {
    listing
        .entries
        .iter_mut()
        .find(|entry| entry.name == name.as_bytes())
        .and_then(|entry| entry.child.take())
        .unwrap_or_else(|| panic!("{name} is listed as a directory"))
}

fn names(listing: &DirListing<MacHandle>) -> Vec<String> {
    let mut names: Vec<String> = listing
        .entries
        .iter()
        .map(|entry| String::from_utf8_lossy(&entry.name).into_owned())
        .collect();
    names.sort();
    names
}

fn scan_dir(path: &Path) -> Tree {
    scan(
        &MacDirSource::new(),
        path,
        &ScanConfig::default(),
        &CancelToken::new(),
        &|_| {},
    )
    .expect("scan succeeds")
}

fn find(tree: &Tree, path: &str) -> NodeId {
    let mut current = tree.root();

    for part in path.split('/') {
        current = tree
            .children(current)
            .find(|&c| tree.name(c) == part.as_bytes())
            .expect("node exists");
    }
    current
}

/// Replaces the directory `path` by an empty directory of the same name (another inode).
fn swap_for_another_directory(path: &Path) {
    let aside = path.with_file_name(format!("{}-old", path.file_name().unwrap().to_string_lossy()));

    fs::rename(path, aside).unwrap();
    fs::create_dir(path).unwrap();
    File::create(path.join("foreign")).unwrap();
}

#[test]
fn a_file_is_its_length_and_the_blocks_the_system_gave_it() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("data"), vec![7u8; 10_000]).unwrap();
    let source = MacDirSource::new();

    let listing = list(&source, &open(&source, dir.path())).unwrap();

    let entry = &listing.entries[0];
    let on_disk = fs::symlink_metadata(dir.path().join("data")).unwrap();
    assert_eq!(entry.meta.kind, EntryKind::File);
    assert_eq!(entry.meta.apparent, 10_000);
    assert_eq!(entry.meta.allocated, on_disk.blocks() * 512);
    assert!(entry.meta.allocated >= 10_000);
    assert_eq!((entry.meta.dev, entry.meta.ino), (on_disk.dev(), on_disk.ino()));
    assert_eq!(entry.meta.nlink, 1);
    assert_eq!(entry.meta.mtime, on_disk.mtime());
    assert!(!entry.meta.is_mount_root);
}

#[test]
fn a_hard_link_is_charged_once_and_a_sparse_file_costs_what_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("b")).unwrap();
    fs::write(dir.path().join("hard1"), vec![1u8; 300_000]).unwrap();
    fs::hard_link(dir.path().join("hard1"), dir.path().join("b/hard2")).unwrap();
    let sparse = File::create(dir.path().join("sparse.img")).unwrap();
    sparse.set_len(64 << 20).unwrap();

    let tree = scan_dir(dir.path());

    assert!(tree.allocated(find(&tree, "hard1")) >= 300_000);
    assert_eq!(
        tree.allocated(find(&tree, "b/hard2")),
        0,
        "the second name costs nothing"
    );
    assert_eq!(tree.apparent(find(&tree, "sparse.img")), 64 << 20);
    assert!(tree.allocated(find(&tree, "sparse.img")) < 1 << 20);
}

#[test]
fn a_symbolic_link_is_a_link_and_is_never_followed() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("real/deeper")).unwrap();
    fs::write(dir.path().join("real/deeper/f"), vec![1u8; 5000]).unwrap();
    symlink("real", dir.path().join("link")).unwrap();
    symlink("..", dir.path().join("real/loop")).unwrap();
    symlink("does-not-exist", dir.path().join("dangling")).unwrap();
    let source = MacDirSource::new();

    let listing = list(&source, &open(&source, dir.path())).unwrap();

    for name in ["link", "dangling"] {
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.name == name.as_bytes())
            .unwrap();
        assert_eq!(entry.meta.kind, EntryKind::Symlink, "{name}");
        assert!(entry.child.is_none(), "{name} is not entered");
    }

    // The scan ends and counts the file once, and each link as a file-like node of its own.
    let tree = scan_dir(dir.path());
    assert_eq!(tree.kind(find(&tree, "link")), NodeKind::Symlink);
    assert_eq!(tree.totals().files, 4, "f, link, dangling and loop");
    assert_eq!(tree.totals().errors, 0);
}

#[test]
fn the_scan_root_may_be_a_symbolic_link_and_a_file_is_no_root() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("real")).unwrap();
    fs::write(dir.path().join("real/f"), b"x").unwrap();
    symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
    let source = MacDirSource::new();

    let through_link = list(&source, &open(&source, &dir.path().join("link"))).unwrap();

    assert_eq!(names(&through_link), ["f"]);
    assert!(
        source
            .open_root(&dir.path().join("real/f"), SourceOptions::default())
            .is_err()
    );
    assert!(
        source
            .open_root(&dir.path().join("missing"), SourceOptions::default())
            .is_err()
    );
}

#[test]
fn a_folder_the_system_refuses_is_one_error_and_the_scan_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("locked")).unwrap();
    fs::write(dir.path().join("locked/secret"), vec![9u8; 5000]).unwrap();
    fs::create_dir(dir.path().join("open")).unwrap();
    fs::write(dir.path().join("open/f"), b"1").unwrap();
    fs::set_permissions(dir.path().join("locked"), Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read_dir(dir.path().join("locked")).is_ok();

    let tree = scan_dir(dir.path());

    fs::set_permissions(dir.path().join("locked"), Permissions::from_mode(0o755)).unwrap();
    if readable {
        eprintln!("running with permission overrides: skipping the unreadable-folder assertions");
        return;
    }
    assert_eq!(tree.totals().errors, 1);
    assert!(tree.is_unreadable(find(&tree, "locked")));
    assert!(!tree.is_unreadable(find(&tree, "open")));
    assert_eq!(tree.totals().files, 1, "the rest of the scan is complete");
}

#[test]
fn a_directory_swapped_for_another_one_is_not_read_through_the_parent_descriptor() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a/sub")).unwrap();
    File::create(dir.path().join("a/sub/original")).unwrap();
    let source = MacDirSource::new();
    let root = open(&source, dir.path());

    let a = child(&mut list(&source, &root).unwrap(), "a");
    let mut in_a = list(&source, &a).unwrap();
    let sub = child(&mut in_a, "sub");
    assert_eq!(
        names(&list(&source, &sub).unwrap()),
        ["original"],
        "unchanged: read"
    );

    // `sub` hangs under `a`'s descriptor, which is still open: one `openat` reaches whatever is named
    // `sub` now.
    swap_for_another_directory(&dir.path().join("a/sub"));

    let error = list(&source, &sub).expect_err("another directory is behind the name");
    assert!(error.to_string().contains("replaced"), "{error}");
}

#[test]
fn a_directory_swapped_for_a_symbolic_link_is_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a/sub")).unwrap();
    fs::create_dir_all(dir.path().join("elsewhere")).unwrap();
    File::create(dir.path().join("elsewhere/secret")).unwrap();
    let source = MacDirSource::new();
    let root = open(&source, dir.path());
    let a = child(&mut list(&source, &root).unwrap(), "a");
    let sub = child(&mut list(&source, &a).unwrap(), "sub");

    fs::remove_dir(dir.path().join("a/sub")).unwrap();
    symlink(dir.path().join("elsewhere"), dir.path().join("a/sub")).unwrap();

    assert!(list(&source, &sub).is_err());
}

#[test]
fn a_directory_swapped_for_another_one_is_not_read_when_reopened_from_the_root() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
    File::create(dir.path().join("a/b/c/original")).unwrap();
    // No descriptor is kept for children, so `c` is reopened by walking `a`, `b`, `c` from the root.
    let source = MacDirSource::new().with_descriptor_budget(0);
    let root = open(&source, dir.path());

    let a = child(&mut list(&source, &root).unwrap(), "a");
    let b = child(&mut list(&source, &a).unwrap(), "b");
    let c = child(&mut list(&source, &b).unwrap(), "c");
    assert_eq!(
        names(&list(&source, &c).unwrap()),
        ["original"],
        "unchanged: read"
    );

    // The middle component is replaced by a lookalike: `a/b/c` exists again, `a` is another directory.
    fs::rename(dir.path().join("a"), dir.path().join("a-old")).unwrap();
    fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
    File::create(dir.path().join("a/b/c/foreign")).unwrap();

    let error = list(&source, &c).expect_err("`a` is not the directory that was listed");
    assert!(error.to_string().contains("replaced"), "{error}");
}

#[test]
fn a_listing_stops_as_soon_as_the_budget_refuses_room() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..50 {
        File::create(dir.path().join(format!("f{i:02}"))).unwrap();
    }
    let source = MacDirSource::new();
    let read = |max_entries| {
        let budget = EntryBudget::new(
            platter_core::BudgetLimits {
                max_entries,
                ..platter_core::BudgetLimits::UNLIMITED
            },
            1,
        );
        let listing = source
            .read_dir(&open(&source, dir.path()), &mut budget.lease())
            .unwrap();

        (listing.entries.len(), budget.is_exhausted())
    };

    assert_eq!(read(10), (10, true));
    assert_eq!(read(u64::MAX), (50, false));
    assert_eq!(read(50), (50, false), "exactly at the cap is not cut");
    assert_eq!(read(0), (0, true));
}

#[test]
fn the_firmlinks_of_the_root_are_part_of_the_scan_and_the_virtual_mounts_and_system_volumes_are_not() {
    let source = MacDirSource::new();
    let root = open(&source, Path::new("/"));
    let mut listing = list(&source, &root).expect("list /");

    // `/dev` is `devfs`: a mount node that is never entered, whether or not the scan stays on one disk.
    let dev = listing
        .entries
        .iter()
        .find(|entry| entry.name == b"dev")
        .expect("/dev");
    assert!(dev.meta.is_mount_root);
    assert!(dev.child.is_none());
    assert_eq!(dev.skipped, Some(SkipReason::System));

    // `/Users`, `/Applications` and `/Library` are on the Data volume, a device of their own, and the same
    // disk as `/`: ordinary folders that are entered.
    for name in ["Users", "Applications", "Library"] {
        let entry = listing
            .entries
            .iter()
            .find(|entry| entry.name == name.as_bytes())
            .expect(name);
        assert!(entry.child.is_some(), "/{name} is entered");
        assert!(!entry.meta.is_mount_root, "/{name} is not another disk");
        assert_eq!(entry.skipped, None, "/{name}");
    }

    // Below `/System`, the folder of the volumes is where the Data volume would be counted a second time.
    let system = child(&mut listing, "System");
    let in_system = list(&source, &system).expect("list /System");
    let volumes = in_system
        .entries
        .iter()
        .find(|entry| entry.name == b"Volumes")
        .expect("/System/Volumes");
    assert!(volumes.meta.is_mount_root);
    assert!(volumes.child.is_none());
    assert_eq!(volumes.skipped, Some(SkipReason::System));
}

#[test]
fn a_scan_that_starts_in_the_system_volumes_enters_the_data_volume_like_any_other_mount() {
    let source = MacDirSource::new();
    let Ok(listing) = list(&source, &open(&source, Path::new("/System/Volumes"))) else {
        eprintln!("/System/Volumes is not readable here: skipping");
        return;
    };

    // Only a scan of `/` leaves `/System/Volumes` out: here the Data volume is another device and an
    // ordinary disk, entered unless the scan stays on one filesystem.
    let data = listing
        .entries
        .iter()
        .find(|entry| entry.name == b"Data")
        .expect("Data");
    assert!(data.meta.is_mount_root, "the Data volume is its own mount");
    assert!(data.child.is_some());
    assert_eq!(data.skipped, None);
}

#[test]
fn the_indexes_at_the_root_of_a_volume_are_never_entered() {
    let source = MacDirSource::new();
    let Ok(listing) = list(&source, &open(&source, Path::new("/System/Volumes/Data"))) else {
        eprintln!("the Data volume's root is not readable here: skipping");
        return;
    };

    let mut seen = 0;
    for entry in &listing.entries {
        let name = String::from_utf8_lossy(&entry.name).to_ascii_lowercase();
        let excluded = [
            ".spotlight-v100",
            ".fseventsd",
            ".documentrevisions-v100",
            ".trashes",
            ".mobilebackups",
        ];
        if excluded.contains(&name.as_str()) && entry.meta.kind == EntryKind::Dir {
            seen += 1;
            assert!(entry.child.is_none(), "{name} is not entered");
            assert_eq!(entry.skipped, Some(SkipReason::System), "{name}");
        }
    }
    eprintln!("{seen} excluded folders at the root of the Data volume");
}
