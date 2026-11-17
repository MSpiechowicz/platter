//! The Windows reader on small real trees: real identity, hard links, links that are never entered, the
//! default exclusions of a drive root. Every tree here has a handful of entries.

#![cfg(windows)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use platter_core::{
    CancelToken, DirListing, DirSource, EntryBudget, NodeId, NodeKind, ScanConfig, SkipReason, SourceOptions,
    Tree, scan,
};
use platter_platform::root::resolve_root;
use platter_platform::windows::WindowsDirSource;
use platter_platform::windows::exclusions::ExclusionRules;
use platter_platform::windows::identity::identity_of_path;
use platter_platform::windows::reader::WindowsHandle;

fn temp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn scan_tree(root: &Path) -> Tree {
    scan(
        &WindowsDirSource::default(),
        root,
        &ScanConfig::default(),
        &CancelToken::new(),
        &|_| {},
    )
    .expect("the scan succeeds")
}

fn find(tree: &Tree, path: &str) -> NodeId {
    let mut current = tree.root();

    for part in path.split('/') {
        current = tree
            .children(current)
            .find(|&child| tree.name(child) == part.as_bytes())
            .unwrap_or_else(|| panic!("{path} is in the tree"));
    }

    current
}

fn list(source: &WindowsDirSource, path: &Path) -> DirListing<WindowsHandle> {
    let root = source.open_root(path, SourceOptions::default()).unwrap();

    source
        .read_dir(&root.handle, &mut EntryBudget::unlimited().lease())
        .unwrap()
}

/// Makes the junction `link` to the folder `target` (a junction needs no privilege, a symbolic link does).
fn junction(link: &Path, target: &Path) {
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(Stdio::null())
        .status()
        .unwrap();

    assert!(status.success(), "mklink /J {link:?} {target:?}");
}

#[test]
fn an_entry_carries_the_volume_serial_and_the_file_index_of_its_file() {
    let dir = temp();
    fs::write(dir.path().join("first"), vec![b'x'; 5000]).unwrap();
    fs::write(dir.path().join("second"), b"y").unwrap();
    fs::create_dir(dir.path().join("folder")).unwrap();

    let listing = list(&WindowsDirSource::default(), dir.path());

    assert_eq!(listing.entries.len(), 3);
    let mut seen = BTreeSet::new();
    for entry in &listing.entries {
        let name = String::from_utf8(entry.name.clone()).unwrap();
        let identity = identity_of_path(&dir.path().join(&name)).unwrap();

        assert_eq!(entry.meta.dev, identity.volume_serial, "{name}");
        assert_eq!(entry.meta.ino, identity.file_index, "{name}");
        assert_eq!(entry.meta.nlink, 1, "{name}");
        assert_ne!(entry.meta.ino, 0, "{name}");
        assert!(seen.insert(entry.meta.ino), "{name} has an index of its own");
        assert!(!entry.meta.is_mount_root, "{name}");
    }

    let first = listing
        .entries
        .iter()
        .find(|entry| entry.name == b"first")
        .unwrap();
    assert_eq!(first.meta.apparent, 5000);
    assert_eq!(first.meta.allocated, 8192, "whole 4 KiB clusters");
    assert!(first.child.is_none());
    let folder = listing
        .entries
        .iter()
        .find(|entry| entry.name == b"folder")
        .unwrap();
    assert!(folder.child.is_some());
}

#[test]
fn the_root_has_the_identity_the_scan_is_pinned_to() {
    let dir = temp();
    let resolved = resolve_root(dir.path()).unwrap();

    let root = WindowsDirSource::default()
        .open_root(&resolved.path, SourceOptions::default())
        .unwrap();

    assert_eq!(resolved.identity, Some((root.meta.dev, root.meta.ino)));
    assert!(
        !resolved.path.to_string_lossy().starts_with(r"\\?\"),
        "{:?}",
        resolved.path
    );
}

#[test]
fn a_hard_link_is_charged_once_and_counts_its_links() {
    let dir = temp();
    fs::create_dir(dir.path().join("a")).unwrap();
    fs::create_dir(dir.path().join("b")).unwrap();
    fs::write(dir.path().join("a/hard1"), vec![1u8; 10_000]).unwrap();
    fs::hard_link(dir.path().join("a/hard1"), dir.path().join("b/hard2")).unwrap();

    let listing = list(&WindowsDirSource::default(), &dir.path().join("a"));
    let tree = scan_tree(dir.path());

    assert_eq!(listing.entries[0].meta.nlink, 2);
    assert_eq!(tree.totals().files, 2);
    assert_eq!(
        tree.totals().allocated,
        12_288,
        "one copy of 10 000 bytes in 4 KiB clusters"
    );
    assert_eq!(tree.allocated(find(&tree, "a/hard1")), 12_288);
    assert_eq!(
        tree.allocated(find(&tree, "b/hard2")),
        0,
        "the second name costs nothing"
    );
}

#[test]
fn a_junction_is_a_link_and_is_never_entered() {
    let dir = temp();
    fs::create_dir(dir.path().join("target")).unwrap();
    fs::write(dir.path().join("target/inside.txt"), vec![7u8; 3000]).unwrap();
    junction(&dir.path().join("link"), &dir.path().join("target"));
    // A junction back to the folder above it: entering it would never end.
    junction(&dir.path().join("target").join("loop"), dir.path());

    let tree = scan_tree(dir.path());

    assert_eq!(tree.kind(find(&tree, "link")), NodeKind::Symlink);
    assert!(!tree.has_children(find(&tree, "link")));
    assert_eq!(tree.kind(find(&tree, "target/loop")), NodeKind::Symlink);
    assert_eq!(tree.totals().files, 3, "inside.txt once, and the two links");
    assert_eq!(
        tree.allocated(tree.root()),
        4096,
        "the file in `target`, not through the junction"
    );
}

#[test]
fn a_scan_root_that_is_a_junction_is_followed() {
    let dir = temp();
    fs::create_dir(dir.path().join("real")).unwrap();
    fs::write(dir.path().join("real/f"), b"x").unwrap();
    junction(&dir.path().join("link"), &dir.path().join("real"));

    let listing = list(&WindowsDirSource::default(), &dir.path().join("link"));

    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name, b"f");
}

#[test]
fn a_folder_swapped_for_a_junction_is_refused_not_followed() {
    let dir = temp();
    fs::create_dir_all(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/inside.txt"), b"x").unwrap();
    fs::create_dir_all(dir.path().join("elsewhere")).unwrap();
    fs::write(dir.path().join("elsewhere/secret.txt"), b"y").unwrap();

    let source = WindowsDirSource::default();
    let root = source.open_root(dir.path(), SourceOptions::default()).unwrap();
    let mut listing = source
        .read_dir(&root.handle, &mut EntryBudget::unlimited().lease())
        .unwrap();
    let sub = listing
        .entries
        .iter_mut()
        .find(|entry| entry.name == b"sub")
        .and_then(|entry| entry.child.take())
        .expect("sub is listed as a folder");

    fs::remove_dir_all(dir.path().join("sub")).unwrap();
    junction(&dir.path().join("sub"), &dir.path().join("elsewhere"));

    let error = source
        .read_dir(&sub, &mut EntryBudget::unlimited().lease())
        .expect_err("the junction must not be listed");
    assert!(error.to_string().contains("replaced"), "{error}");
}

#[test]
fn a_folder_swapped_for_another_folder_is_refused() {
    let dir = temp();
    fs::create_dir_all(dir.path().join("sub")).unwrap();

    let source = WindowsDirSource::default();
    let root = source.open_root(dir.path(), SourceOptions::default()).unwrap();
    let mut listing = source
        .read_dir(&root.handle, &mut EntryBudget::unlimited().lease())
        .unwrap();
    let sub = listing.entries[0].child.take().unwrap();

    fs::remove_dir(dir.path().join("sub")).unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();

    let error = source
        .read_dir(&sub, &mut EntryBudget::unlimited().lease())
        .expect_err("another folder is not the one that was listed");
    assert!(error.to_string().contains("replaced"), "{error}");
}

#[test]
fn the_default_exclusions_apply_to_a_drive_root_only() {
    // Not a drive root: the same names are scanned.
    let dir = temp();
    fs::write(dir.path().join("pagefile.sys"), b"x").unwrap();
    fs::create_dir(dir.path().join("$Recycle.Bin")).unwrap();

    let tree = scan_tree(dir.path());

    assert_eq!(tree.kind(find(&tree, "pagefile.sys")), NodeKind::File);
    assert_eq!(tree.kind(find(&tree, "$Recycle.Bin")), NodeKind::Dir);
}

#[test]
fn the_system_files_of_the_system_drive_are_left_out_and_its_folders_are_not_entered() {
    let drive = PathBuf::from(format!(
        "{}\\",
        std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into())
    ));
    let rules = ExclusionRules::DEFAULT;
    let name_of =
        |entry: &platter_core::RawEntry<WindowsHandle>| String::from_utf8_lossy(&entry.name).into_owned();

    let everything = list(&WindowsDirSource::with_rules(ExclusionRules::NONE), &drive);
    let by_default = list(&WindowsDirSource::default(), &drive);

    // Left out of the listing: exactly the files the rules name.
    let listed: BTreeSet<String> = by_default.entries.iter().map(name_of).collect();
    for entry in &everything.entries {
        let name = name_of(entry);
        let is_excluded_file = entry.meta.kind == platter_core::EntryKind::File
            && rules.excludes_file(std::ffi::OsStr::new(&name));

        assert_eq!(listed.contains(&name), !is_excluded_file, "{name}");
    }

    // A folder the rules name is listed as a mount that is not entered.
    for entry in &by_default.entries {
        let name = name_of(entry);

        if entry.meta.kind == platter_core::EntryKind::Dir
            && rules.excludes_folder(std::ffi::OsStr::new(&name))
        {
            assert!(entry.meta.is_mount_root, "{name}");
            assert!(entry.child.is_none(), "{name}");
            assert_eq!(entry.skipped, Some(SkipReason::System), "{name}");
        }
    }
    assert!(
        listed.iter().any(|name| name.eq_ignore_ascii_case("Windows")),
        "the rest of the drive is listed"
    );
}
