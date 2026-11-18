//! Mount handling of the Linux reader against the real mount table.

#![cfg(target_os = "linux")]

use std::path::Path;

use platter_core::{DirSource, EntryBudget, EntryKind, SkipReason, SourceOptions};
use platter_platform::linux::{ExclusionRules, LinuxDirSource};

fn entry_of(
    source: &LinuxDirSource,
    name: &str,
) -> Option<platter_core::RawEntry<platter_platform::linux::LinuxHandle>> {
    entry_of_with(source, name, SourceOptions::default())
}

fn entry_of_with(
    source: &LinuxDirSource,
    name: &str,
    options: SourceOptions,
) -> Option<platter_core::RawEntry<platter_platform::linux::LinuxHandle>> {
    let root = source.open_root(Path::new("/"), options).ok()?;
    let listing = source
        .read_dir(&root.handle, &mut EntryBudget::unlimited().lease())
        .ok()?;

    listing
        .entries
        .into_iter()
        .find(|entry| entry.name == name.as_bytes())
}

#[test]
fn pseudo_filesystem_mounts_are_listed_but_not_entered() {
    let Ok(table) = std::fs::read("/proc/self/mountinfo") else {
        return;
    };
    let mounts = platter_platform::linux::mountinfo::parse(&table);
    if !mounts
        .iter()
        .any(|mount| mount.mount_point == Path::new("/proc") && mount.fs_type == "proc")
    {
        return;
    }

    let source = LinuxDirSource::new();
    let Some(proc) = entry_of(&source, "proc") else {
        return;
    };

    assert_eq!(proc.meta.kind, EntryKind::Dir);
    if proc.meta.is_mount_root {
        assert!(proc.child.is_none(), "/proc must not be descended");
    }
}

#[test]
fn without_rules_mounts_get_a_handle() {
    let source = LinuxDirSource::with_rules(ExclusionRules::NONE);
    let Some(proc) = entry_of(&source, "proc") else {
        return;
    };

    assert!(proc.child.is_some());
}

#[test]
fn ordinary_directories_always_get_a_handle() {
    let source = LinuxDirSource::new();
    let Some(usr) = entry_of(&source, "usr") else {
        return;
    };

    assert!(usr.child.is_some());
}

/// `(mount point, device)` of the mounts directly below `/`, and the device of `/` itself.
fn top_level_mounts() -> Option<(String, Vec<(String, String)>)> {
    let table = std::fs::read("/proc/self/mountinfo").ok()?;
    let mounts = platter_platform::linux::mountinfo::parse(&table);

    let root = mounts
        .iter()
        .rev()
        .find(|mount| mount.mount_point == Path::new("/"))?;
    let below = mounts
        .iter()
        .filter(|mount| {
            mount.mount_point.parent() == Some(Path::new("/")) && mount.mount_point != Path::new("/")
        })
        .filter_map(|mount| Some((mount.mount_point.to_str()?.to_owned(), mount.device.clone())))
        .collect();

    Some((root.device.clone(), below))
}

#[test]
fn one_file_system_never_enters_a_mount_on_another_device() {
    let Some((root_device, below)) = top_level_mounts() else {
        return;
    };
    let source = LinuxDirSource::with_rules(ExclusionRules::NONE);
    let options = SourceOptions {
        one_file_system: true,
    };

    for (mount_point, device) in below.iter().filter(|(_, device)| *device != root_device) {
        let name = mount_point.trim_start_matches('/');
        let Some(entry) = entry_of_with(&source, name, options) else {
            continue;
        };

        if entry.meta.is_mount_root {
            assert!(
                entry.child.is_none(),
                "{mount_point} ({device}) is another filesystem"
            );
        }
    }
}

#[test]
fn one_file_system_still_enters_ordinary_directories() {
    let source = LinuxDirSource::new();
    let options = SourceOptions {
        one_file_system: true,
    };
    let Some(usr) = entry_of_with(&source, "usr", options) else {
        return;
    };
    if usr.meta.is_mount_root {
        return;
    }

    assert!(usr.child.is_some());
}

#[test]
fn a_pseudo_filesystem_is_skipped_as_a_system_mount_with_and_without_one_file_system() {
    let Ok(table) = std::fs::read("/proc/self/mountinfo") else {
        return;
    };
    let mounts = platter_platform::linux::mountinfo::parse(&table);
    if !mounts
        .iter()
        .any(|mount| mount.mount_point == Path::new("/proc") && mount.fs_type == "proc")
    {
        return;
    }

    for one_file_system in [false, true] {
        let source = LinuxDirSource::new();
        let Some(proc) = entry_of_with(&source, "proc", SourceOptions { one_file_system }) else {
            return;
        };
        if !proc.meta.is_mount_root {
            return;
        }

        assert!(proc.child.is_none());
        assert_eq!(
            proc.skipped,
            Some(SkipReason::System),
            "one_file_system = {one_file_system}"
        );
    }
}

#[test]
fn another_disk_left_out_by_one_file_system_says_so_and_is_entered_without_it() {
    let Some((root_device, below)) = top_level_mounts() else {
        return;
    };
    let table = std::fs::read("/proc/self/mountinfo").unwrap_or_default();
    let mounts = platter_platform::linux::mountinfo::parse(&table);
    let rules = platter_platform::linux::ExclusionRules::DEFAULT;

    // A top-level mount of another block device that the rules do not exclude, e.g. `/boot`.
    let Some(name) = below.iter().find_map(|(mount_point, device)| {
        let mount = mounts
            .iter()
            .find(|mount| mount.mount_point == Path::new(mount_point))?;
        (*device != root_device && !rules.excludes(&mount.mount_point, &mount.fs_type))
            .then(|| mount_point.trim_start_matches('/').to_owned())
    }) else {
        return;
    };

    let source = LinuxDirSource::new();
    let Some(left_out) = entry_of_with(
        &source,
        &name,
        SourceOptions {
            one_file_system: true,
        },
    ) else {
        return;
    };
    if !left_out.meta.is_mount_root {
        return;
    }
    assert!(left_out.child.is_none());
    assert_eq!(left_out.skipped, Some(SkipReason::OtherDisk), "/{name}");

    let entered = entry_of_with(&source, &name, SourceOptions::default()).unwrap();
    assert!(
        entered.child.is_some(),
        "/{name} is another disk, which is entered by default"
    );
    assert_eq!(entered.skipped, None, "an entered mount has no reason");
    assert!(
        !entered.mount.own_filesystem,
        "/{name} is another disk: entered, but not the scan root's own filesystem"
    );
}

#[test]
fn a_subvolume_of_the_scan_roots_btrfs_is_entered_as_its_own_filesystem() {
    let table = std::fs::read("/proc/self/mountinfo").unwrap_or_default();
    let mounts = platter_platform::linux::mountinfo::parse(&table);
    let Some(root) = mounts
        .iter()
        .rev()
        .find(|mount| mount.mount_point == Path::new("/"))
    else {
        return;
    };
    if root.fs_type != "btrfs" {
        return;
    }

    // A top-level mount of the same btrfs filesystem, such as `/home` on a machine with `@home`.
    let subvolumes: Vec<String> = mounts
        .iter()
        .filter(|mount| mount.device == root.device && mount.fs_type == "btrfs")
        .filter(|mount| {
            mount.mount_point.parent() == Some(Path::new("/")) && mount.mount_point != Path::new("/")
        })
        .filter_map(|mount| mount.mount_point.to_str()?.strip_prefix('/').map(str::to_owned))
        .collect();

    let source = LinuxDirSource::new();
    for name in subvolumes {
        for one_file_system in [true, false] {
            let options = SourceOptions { one_file_system };
            let Some(entry) = entry_of_with(&source, &name, options) else {
                continue;
            };
            if entry.child.is_none() {
                continue;
            }

            assert!(
                entry.mount.own_filesystem,
                "/{name}, one_file_system: {one_file_system}"
            );
            assert!(entry.mount.mount_id.is_some(), "/{name}");
        }
    }
}

#[test]
fn a_mount_missing_from_the_table_is_skipped_as_unknown() {
    // The table is read from a file that lists no mount at all: every mount root is unknown.
    let dir = tempfile::tempdir().unwrap();
    let table = dir.path().join("mountinfo");
    std::fs::write(&table, b"1 0 8:1 / /somewhere/else rw - ext4 /dev/sda1 rw\n").unwrap();

    let source = LinuxDirSource::with_rules(ExclusionRules::NONE).with_mountinfo_path(&table);
    let Some(proc) = entry_of(&source, "proc") else {
        return;
    };
    if !proc.meta.is_mount_root {
        return;
    }

    assert!(proc.child.is_none());
    assert_eq!(proc.skipped, Some(SkipReason::Unknown));
}
