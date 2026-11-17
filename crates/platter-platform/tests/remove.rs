//! Deleting and trashing on real (small) temp folders.
#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use platter_core::{BirthTime, EntryKind, EntryProbe, EntryRemover, ExpectedEntry, RemoveMode};
use platter_platform::linux::probe::LinuxEntryProbe;
use platter_platform::linux::remove::LinuxEntryRemover;

fn bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}

/// What the plan would have recorded for `path` right now, with `root` as the scan root.
fn expected(path: &Path, root: &Path) -> ExpectedEntry {
    let found = LinuxEntryProbe.probe(&bytes(path)).unwrap();
    let last = found.last().unwrap();
    ExpectedEntry {
        dev: last.meta.dev,
        ino: last.meta.ino,
        mount_id: last.mount_id,
        birth: last.birth,
        is_dir: last.meta.kind == EntryKind::Dir,
        root_depth: root.components().count() - 1,
    }
}

fn temp() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

fn write(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"data").unwrap();
}

#[test]
fn a_folder_is_removed_whole_and_a_link_inside_is_removed_not_followed() {
    let (_dir, root) = temp();
    write(&root.join("keep/precious"));
    write(&root.join("t/a/b/c"));
    write(&root.join("t/x"));
    symlink(root.join("keep"), root.join("t/a/link")).unwrap();
    let target = root.join("t");

    let removed = LinuxEntryRemover
        .remove(&bytes(&target), &expected(&target, &root), RemoveMode::Whole)
        .unwrap();

    assert!(removed.errors.is_empty(), "{removed:?}");
    assert_eq!(removed.entries, 6, "c, b, link, a, x and t: {removed:?}");
    assert!(!target.exists());
    assert!(
        root.join("keep/precious").exists(),
        "the link's target is untouched"
    );
}

#[test]
fn emptying_keeps_the_folder() {
    let (_dir, root) = temp();
    write(&root.join("cache/a/b"));
    write(&root.join("cache/c"));
    let target = root.join("cache");

    let removed = LinuxEntryRemover
        .remove(&bytes(&target), &expected(&target, &root), RemoveMode::Contents)
        .unwrap();

    assert!(removed.errors.is_empty());
    assert!(target.is_dir());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
}

#[test]
fn a_file_is_removed() {
    let (_dir, root) = temp();
    write(&root.join("old.iso"));
    let target = root.join("old.iso");

    LinuxEntryRemover
        .remove(&bytes(&target), &expected(&target, &root), RemoveMode::Whole)
        .unwrap();

    assert!(!target.exists());
}

#[test]
fn a_folder_swapped_for_a_link_after_the_plan_is_refused_and_nothing_is_touched() {
    let (_dir, root) = temp();
    write(&root.join("target/obj"));
    write(&root.join("elsewhere/precious"));
    let target = root.join("target");
    let plan = expected(&target, &root);

    fs::rename(&target, root.join("moved")).unwrap();
    symlink(root.join("elsewhere"), &target).unwrap();
    let error = LinuxEntryRemover
        .remove(&bytes(&target), &plan, RemoveMode::Whole)
        .unwrap_err();

    assert!(error.to_string().contains("changed since the plan"), "{error}");
    assert!(root.join("elsewhere/precious").exists());
    assert!(root.join("moved/obj").exists());
    assert!(fs::symlink_metadata(&target).unwrap().file_type().is_symlink());
}

#[test]
fn a_folder_on_the_way_swapped_for_a_link_is_refused() {
    let (_dir, root) = temp();
    write(&root.join("p/target/obj"));
    write(&root.join("q/target/precious"));
    let target = root.join("p/target");
    let plan = expected(&target, &root);

    fs::rename(root.join("p"), root.join("p-old")).unwrap();
    symlink(root.join("q"), root.join("p")).unwrap();
    let error = LinuxEntryRemover
        .remove(&bytes(&target), &plan, RemoveMode::Whole)
        .unwrap_err();

    assert!(error.to_string().contains("changed since the plan"), "{error}");
    assert!(root.join("q/target/precious").exists());
}

#[test]
fn another_folder_with_the_same_name_is_refused() {
    let (_dir, root) = temp();
    write(&root.join("target/obj"));
    let target = root.join("target");
    let plan = expected(&target, &root);

    fs::remove_dir_all(&target).unwrap();
    write(&root.join("target/new"));
    let error = LinuxEntryRemover
        .remove(&bytes(&target), &plan, RemoveMode::Whole)
        .unwrap_err();

    assert!(error.to_string().contains("another entry"), "{error}");
    assert!(root.join("target/new").exists());
}

/// What ext4 does soon after a delete: the folder made again gets the same inode number. Simulated here
/// (the test's own filesystem may not reuse it) by a plan that differs only in the creation time.
#[test]
fn an_entry_made_again_under_the_same_inode_number_is_refused() {
    let (_dir, root) = temp();
    write(&root.join("target/new"));
    let target = root.join("target");
    let mut plan = expected(&target, &root);
    let Some(birth) = plan.birth else {
        eprintln!("this filesystem records no creation time: nothing to compare");
        return;
    };
    plan.birth = Some(BirthTime {
        secs: birth.secs - 60,
        ..birth
    });

    let error = LinuxEntryRemover
        .remove(&bytes(&target), &plan, RemoveMode::Whole)
        .unwrap_err();

    assert!(error.to_string().contains("another entry"), "{error}");
    assert!(root.join("target/new").exists());
}

#[test]
fn read_only_folders_are_emptied() {
    let (_dir, root) = temp();
    write(&root.join("mod/pkg@v1/file.go"));
    fs::set_permissions(root.join("mod/pkg@v1"), fs::Permissions::from_mode(0o555)).unwrap();
    let target = root.join("mod");

    let removed = LinuxEntryRemover
        .remove(&bytes(&target), &expected(&target, &root), RemoveMode::Contents)
        .unwrap();

    assert!(removed.errors.is_empty(), "{removed:?}");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
}

#[test]
fn a_kept_read_only_folder_gets_its_mode_back() {
    let (_dir, root) = temp();
    write(&root.join("cache/old"));
    fs::set_permissions(root.join("cache"), fs::Permissions::from_mode(0o555)).unwrap();
    let target = root.join("cache");

    let removed = LinuxEntryRemover
        .remove(&bytes(&target), &expected(&target, &root), RemoveMode::Contents)
        .unwrap();
    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o7777;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(removed.errors.is_empty(), "{removed:?}");
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    assert_eq!(mode, 0o555, "the folder that stays keeps its own permissions");
}

#[test]
fn an_unreadable_folder_inside_is_reported_and_the_rest_is_removed() {
    let (_dir, root) = temp();
    write(&root.join("t/locked/inner"));
    write(&root.join("t/a"));
    write(&root.join("t/b"));
    fs::set_permissions(root.join("t/locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let target = root.join("t");

    let removed = LinuxEntryRemover
        .remove(&bytes(&target), &expected(&target, &root), RemoveMode::Whole)
        .unwrap();
    fs::set_permissions(root.join("t/locked"), fs::Permissions::from_mode(0o755)).ok();

    // Running as root reads everything; the test only means something for a normal user.
    if rustix::process::getuid().is_root() {
        return;
    }
    assert!(!root.join("t/a").exists() && !root.join("t/b").exists());
    assert!(root.join("t/locked/inner").exists());
    assert!(
        removed
            .errors
            .iter()
            .any(|(path, _)| path.ends_with(b"/t/locked")),
        "{removed:?}"
    );
}

#[test]
fn a_mount_on_the_way_below_the_scan_root_is_refused() {
    // `/proc` is a mount; with the scan root at `/` it lies below the root.
    let plan = ExpectedEntry {
        dev: 0,
        ino: 0,
        mount_id: None,
        birth: None,
        is_dir: false,
        root_depth: 0,
    };

    let error = LinuxEntryRemover
        .remove(b"/proc/self/comm", &plan, RemoveMode::Whole)
        .unwrap_err();

    assert!(error.to_string().contains("mount"), "{error}");
}

#[test]
fn trashing_moves_the_entry_and_writes_its_record() {
    let (_dir, root) = temp();
    write(&root.join("Downloads/old.iso"));
    write(&root.join("Downloads/sub/old.iso"));
    let trash = root.join("Trash");

    for path in ["Downloads/old.iso", "Downloads/sub/old.iso"] {
        let target = root.join(path);
        LinuxEntryRemover
            .trash(&bytes(&target), &expected(&target, &root), &trash)
            .unwrap();
        assert!(!target.exists());
    }

    assert!(trash.join("files/old.iso").exists());
    assert!(
        trash.join("files/old.iso.2").exists(),
        "the second gets a free name"
    );
    let record = fs::read_to_string(trash.join("info/old.iso.2.trashinfo")).unwrap();
    assert!(record.starts_with("[Trash Info]\nPath="), "{record}");
    assert!(record.contains("/Downloads/sub/old.iso\n"), "{record}");
}

#[test]
fn trashing_a_swapped_entry_is_refused() {
    let (_dir, root) = temp();
    write(&root.join("a.iso"));
    write(&root.join("precious"));
    let target = root.join("a.iso");
    let plan = expected(&target, &root);

    fs::remove_file(&target).unwrap();
    symlink(root.join("precious"), &target).unwrap();
    let error = LinuxEntryRemover
        .trash(&bytes(&target), &plan, &root.join("Trash"))
        .unwrap_err();

    assert!(error.to_string().contains("changed since the plan"), "{error}");
    assert!(root.join("precious").exists());
}

#[test]
fn free_space_is_reported() {
    let (_dir, root) = temp();

    assert!(LinuxEntryRemover.free_bytes(&root).unwrap() > 0);
}

/// Set in the child process that runs inside a user and mount namespace (see the test below).
const MOUNT_CHILD: &str = "PLATTER_TEST_MOUNT_CHILD";

#[test]
fn a_mount_inside_a_removed_folder_is_left_alone() {
    if let Some(dir) = std::env::var_os(MOUNT_CHILD) {
        // In the namespace: root there, so a tmpfs can be mounted inside the folder.
        let root = PathBuf::from(dir);
        write(&root.join("t/a"));
        fs::create_dir_all(root.join("t/m")).unwrap();
        let mounted = std::process::Command::new("mount")
            .args(["-t", "tmpfs", "none"])
            .arg(root.join("t/m"))
            .status()
            .unwrap();
        assert!(mounted.success());
        write(&root.join("t/m/precious"));
        let target = root.join("t");

        let removed = LinuxEntryRemover
            .remove(&bytes(&target), &expected(&target, &root), RemoveMode::Whole)
            .unwrap();

        assert_eq!(removed.skipped_mounts, 1, "{removed:?}");
        assert!(
            root.join("t/m/precious").exists(),
            "nothing on the mount is touched"
        );
        assert!(!root.join("t/a").exists());
        assert!(target.exists(), "the folder stays, it still holds the mount");
        return;
    }

    let usable = std::process::Command::new("unshare")
        .args(["-Urm", "true"])
        .status()
        .is_ok_and(|status| status.success());
    if !usable {
        eprintln!("skipped: unprivileged user namespaces are not available here");
        return;
    }

    let (_dir, root) = temp();
    let status = std::process::Command::new("unshare")
        .arg("-Urm")
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_mount_inside_a_removed_folder_is_left_alone",
            "--nocapture",
        ])
        .env(MOUNT_CHILD, &root)
        .status()
        .unwrap();

    assert!(status.success(), "the test in the namespace failed");
}

/// Runs the test `name` again inside an unprivileged user and mount namespace, with `MOUNT_CHILD` set to a
/// fresh temp folder. Returns `false` (skipped) where such namespaces are not available.
fn in_namespace(name: &str) -> bool {
    let usable = std::process::Command::new("unshare")
        .args(["-Urm", "true"])
        .status()
        .is_ok_and(|status| status.success());
    if !usable {
        eprintln!("skipped: unprivileged user namespaces are not available here");
        return false;
    }

    let (_dir, root) = temp();
    let status = std::process::Command::new("unshare")
        .arg("-Urm")
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env(MOUNT_CHILD, &root)
        .status()
        .unwrap();
    assert!(status.success(), "the test in the namespace failed");
    true
}

fn mount_tmpfs(at: &Path) {
    fs::create_dir_all(at).unwrap();
    let mounted = std::process::Command::new("mount")
        .args(["-t", "tmpfs", "none"])
        .arg(at)
        .status()
        .unwrap();
    assert!(mounted.success());
}

#[test]
fn an_entry_on_another_disk_goes_to_that_disks_own_trash() {
    let Some(dir) = std::env::var_os(MOUNT_CHILD) else {
        in_namespace("an_entry_on_another_disk_goes_to_that_disks_own_trash");
        return;
    };
    let root = PathBuf::from(dir);
    mount_tmpfs(&root.join("disk"));
    write(&root.join("disk/photos/old.iso"));
    let target = root.join("disk/photos/old.iso");
    let uid = rustix::process::getuid().as_raw();

    let to = LinuxEntryRemover
        .trash(
            &bytes(&target),
            &expected(&target, &root.join("disk")),
            &root.join("home-trash"),
        )
        .unwrap();

    let disk_trash = root.join(format!("disk/.Trash-{uid}"));
    assert_eq!(to, bytes(&disk_trash.join("files/old.iso")));
    assert!(!target.exists());
    assert!(disk_trash.join("files/old.iso").exists());
    let record = fs::read_to_string(disk_trash.join("info/old.iso.trashinfo")).unwrap();
    assert!(
        record.contains("\nPath=photos/old.iso\n"),
        "relative to the disk's top: {record}"
    );
    assert_eq!(
        fs::metadata(&disk_trash).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn a_disks_trash_open_to_others_is_refused() {
    let Some(dir) = std::env::var_os(MOUNT_CHILD) else {
        in_namespace("a_disks_trash_open_to_others_is_refused");
        return;
    };
    let root = PathBuf::from(dir);
    mount_tmpfs(&root.join("disk"));
    write(&root.join("disk/old.iso"));
    let uid = rustix::process::getuid().as_raw();
    let planted = root.join(format!("disk/.Trash-{uid}"));
    fs::create_dir(&planted).unwrap();
    fs::set_permissions(&planted, fs::Permissions::from_mode(0o777)).unwrap();
    let target = root.join("disk/old.iso");

    let error = LinuxEntryRemover
        .trash(
            &bytes(&target),
            &expected(&target, &root.join("disk")),
            &root.join("home-trash"),
        )
        .unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied, "{error}");
    assert!(target.exists());
}

#[test]
fn an_entry_in_a_nested_btrfs_subvolume_goes_to_that_subvolumes_trash() {
    // The cargo target folder is on the developer's own disk (here btrfs); `/tmp` usually is not.
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let created = std::process::Command::new("btrfs")
        .args(["subvolume", "create"])
        .arg(root.join("sub"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !created {
        eprintln!("skipped: no btrfs subvolume can be made here");
        return;
    }
    write(&root.join("sub/data/old.iso"));
    let target = root.join("sub/data/old.iso");
    let uid = rustix::process::getuid().as_raw();

    let to = LinuxEntryRemover
        .trash(
            &bytes(&target),
            &expected(&target, &root),
            &root.join("home-trash"),
        )
        .unwrap();

    let trash = root.join(format!("sub/.Trash-{uid}"));
    assert_eq!(to, bytes(&trash.join("files/old.iso")));
    let record = fs::read_to_string(trash.join("info/old.iso.trashinfo")).unwrap();
    assert!(record.contains("\nPath=data/old.iso\n"), "{record}");

    fs::remove_dir_all(root.join("sub")).ok();
}

#[test]
fn a_big_folder_is_removed_in_batches() {
    let (_dir, root) = temp();
    fs::create_dir(root.join("big")).unwrap();
    for number in 0..3000 {
        fs::write(root.join(format!("big/f{number:05}")), b"").unwrap();
    }
    let target = root.join("big");

    let removed = LinuxEntryRemover
        .remove(&bytes(&target), &expected(&target, &root), RemoveMode::Whole)
        .unwrap();

    assert!(!removed.has_errors(), "{:?}", removed.errors.first());
    assert_eq!(removed.entries, 3001);
    assert!(!target.exists());
}

#[test]
fn problems_inside_are_counted_beyond_the_ones_kept_in_detail() {
    if rustix::process::getuid().is_root() {
        return;
    }
    let (_dir, root) = temp();
    for number in 0..150 {
        let locked = root.join(format!("t/locked{number:03}"));
        write(&locked.join("inner"));
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    }
    let target = root.join("t");

    let removed = LinuxEntryRemover
        .remove(&bytes(&target), &expected(&target, &root), RemoveMode::Contents)
        .unwrap();
    for number in 0..150 {
        fs::set_permissions(
            root.join(format!("t/locked{number:03}")),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }

    assert_eq!(removed.errors.len(), platter_core::Removed::MAX_ERRORS);
    assert!(removed.more_errors >= 50, "{}", removed.more_errors);
}

#[test]
fn a_trashed_entry_is_put_back_and_its_record_removed() {
    let (_dir, root) = temp();
    write(&root.join("Downloads/old.iso"));
    let target = root.join("Downloads/old.iso");
    let plan = expected(&target, &root);
    let trash = root.join("Trash");
    let to = LinuxEntryRemover.trash(&bytes(&target), &plan, &trash).unwrap();

    LinuxEntryRemover.restore(&to, &plan, &bytes(&target)).unwrap();

    assert!(target.exists());
    assert!(!trash.join("files/old.iso").exists());
    assert!(!trash.join("info/old.iso.trashinfo").exists());
}

#[test]
fn putting_back_never_overwrites_and_checks_what_it_moves() {
    let (_dir, root) = temp();
    write(&root.join("a.iso"));
    write(&root.join("b.iso"));
    let trash = root.join("Trash");
    let a = root.join("a.iso");
    let b = root.join("b.iso");
    let plan_a = expected(&a, &root);
    let plan_b = expected(&b, &root);
    let a_to = LinuxEntryRemover.trash(&bytes(&a), &plan_a, &trash).unwrap();
    let b_to = LinuxEntryRemover.trash(&bytes(&b), &plan_b, &trash).unwrap();

    // Something new took `a.iso`'s old name: it stays, and the trashed one stays in the trash.
    fs::write(&a, b"new").unwrap();
    let taken = LinuxEntryRemover.restore(&a_to, &plan_a, &bytes(&a)).unwrap_err();
    assert_eq!(taken.kind(), std::io::ErrorKind::AlreadyExists, "{taken}");
    assert_eq!(fs::read(&a).unwrap(), b"new");
    assert!(trash.join("files/a.iso").exists());

    // The entry in the trash is not the one that was trashed: refused.
    let wrong = LinuxEntryRemover.restore(&b_to, &plan_a, &bytes(&b)).unwrap_err();
    assert!(wrong.to_string().contains("changed since the plan"), "{wrong}");
    assert!(!b.exists());
}

#[test]
fn putting_back_needs_the_trash_record_to_name_the_original_place() {
    let (_dir, root) = temp();
    write(&root.join("a.iso"));
    let a = root.join("a.iso");
    let plan = expected(&a, &root);
    let trash = root.join("Trash");
    let to = LinuxEntryRemover.trash(&bytes(&a), &plan, &trash).unwrap();

    // Another place asked for: refused, nothing moves.
    let elsewhere = root.join("elsewhere.iso");
    let refused = LinuxEntryRemover
        .restore(&to, &plan, &bytes(&elsewhere))
        .unwrap_err();
    assert!(refused.to_string().contains("trash record"), "{refused}");
    assert!(!elsewhere.exists());

    // The record is gone (the trash was emptied by hand): refused too.
    fs::remove_file(trash.join("info/a.iso.trashinfo")).unwrap();
    assert!(LinuxEntryRemover.restore(&to, &plan, &bytes(&a)).is_err());
    assert!(trash.join("files/a.iso").exists());
}
