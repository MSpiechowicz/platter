//! Which parts of the disk a scan on macOS may enter (package M1).
//!
//! The rule: when the scan root is `/`, the devices allowed are the root's and that of
//! `/System/Volumes/Data` (the firmlinks: `/Users`, `/Applications`, `/Library`, `/private` and others live
//! on the Data volume and show up in `/` through them), and `/System/Volumes` itself is left out, so the
//! Data volume is not counted twice. Any other root allows its own device only. `devfs`, `autofs`, `smbfs`,
//! `nfs`, `afpfs` and `webdav` are never entered, and neither are `.Spotlight-V100`, `.fseventsd`,
//! `.DocumentRevisions-V100`, `.Trashes` and `.MobileBackups` at a volume root.
//!
//! Everything here is a function of plain values (bytes, device numbers, a filesystem type name), so the
//! tests run on every OS. The system calls that produce those values are in `macos::exclusions`.

use platter_core::SkipReason;

/// Where the writable Data volume is mounted.
pub const DATA_VOLUME: &str = "/System/Volumes/Data";

/// The folder that holds every volume of the system, the Data volume among them.
const SYSTEM_VOLUMES: &[u8] = b"/System/Volumes";

/// Filesystem types a scan never enters: virtual (`devfs`, `autofs`) and network ones.
const EXCLUDED_FS_TYPES: &[&str] = &["devfs", "autofs", "smbfs", "nfs", "afpfs", "webdav"];

/// Folders at the root of a volume that a scan never enters: the system's indexes, logs and
/// the volume's own trash.
const EXCLUDED_VOLUME_ROOT_NAMES: &[&str] = &[
    ".Spotlight-V100",
    ".fseventsd",
    ".DocumentRevisions-V100",
    ".Trashes",
    ".MobileBackups",
];

/// The devices a scan that starts at `root` (the canonical path) may enter: the root's own, and for the
/// root `/` also that of the Data volume, when there is one.
pub fn allowed_devices(root: &[u8], root_dev: u64, data_volume_dev: Option<u64>) -> Vec<u64> {
    let mut allowed = vec![root_dev];

    if is_filesystem_root(root)
        && let Some(data) = data_volume_dev
        && data != root_dev
    {
        allowed.push(data);
    }
    allowed
}

/// Whether a scan that starts at `root` leaves out `/System/Volumes` (it is where the Data volume is
/// mounted a second time).
pub fn hides_system_volumes(root: &[u8]) -> bool {
    is_filesystem_root(root)
}

/// Whether the folder at `path` is the one a scan from `root` never enters: `/System/Volumes` when the
/// root is `/`.
pub fn is_system_volumes(root: &[u8], path: &[u8]) -> bool {
    hides_system_volumes(root) && path == SYSTEM_VOLUMES
}

/// Whether the mount point `path` is the folder of the system's own volumes or inside it (`VM`,
/// `Preboot`, `Data`...): none of them is a volume a user means, and the Data volume is the same disk
/// as `/` (see [`allowed_devices`]).
pub fn is_inside_system_volumes(path: &[u8]) -> bool {
    path.strip_prefix(SYSTEM_VOLUMES)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(b"/"))
}

/// Whether a filesystem type is never entered.
pub fn is_excluded_fs_type(fs_type: &str) -> bool {
    EXCLUDED_FS_TYPES.contains(&fs_type)
}

/// Whether `name`, a folder at a volume root, is never entered. The volumes are case-insensitive by
/// default, so the case of the name does not matter.
pub fn is_excluded_volume_root_name(name: &[u8]) -> bool {
    EXCLUDED_VOLUME_ROOT_NAMES
        .iter()
        .any(|excluded| name.eq_ignore_ascii_case(excluded.as_bytes()))
}

/// Whether a mount that the kernel reports at `mounted_on` is at `path`. The two are the same text, except
/// that a mount point below a firmlink is reported on the Data volume (`/home` is at
/// `/System/Volumes/Data/home`).
pub fn is_mount_point_of(mounted_on: &[u8], path: &[u8]) -> bool {
    if mounted_on == path {
        return true;
    }

    mounted_on
        .strip_prefix(DATA_VOLUME.as_bytes())
        .is_some_and(|rest| rest == path)
}

/// Whether the mount root of device `dev` and filesystem `fs_type` (`None`: not known) is entered, and if
/// not, why. A device in `allowed` is part of the same disk and needs no further look. Any other mount is
/// entered only when its filesystem is known and not excluded, and a scan that stays on one filesystem
/// (`one_file_system`) enters no other disk.
pub fn mount_verdict(
    allowed: &[u64],
    one_file_system: bool,
    dev: u64,
    fs_type: Option<&str>,
) -> Result<(), SkipReason> {
    if allowed.contains(&dev) {
        return Ok(());
    }

    let Some(fs_type) = fs_type else {
        return Err(SkipReason::Unknown);
    };
    if is_excluded_fs_type(fs_type) {
        return Err(SkipReason::System);
    }
    if one_file_system {
        return Err(SkipReason::OtherDisk);
    }

    Ok(())
}

fn is_filesystem_root(root: &[u8]) -> bool {
    root == b"/"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_the_root_of_the_disk_the_firmlinked_data_volume_is_part_of_the_scan() {
        assert_eq!(allowed_devices(b"/", 1, Some(2)), [1, 2]);
        assert_eq!(
            allowed_devices(b"/", 1, Some(1)),
            [1],
            "one device is listed once"
        );
        assert_eq!(
            allowed_devices(b"/", 1, None),
            [1],
            "no Data volume, nothing to add"
        );
    }

    #[test]
    fn any_other_root_allows_its_own_device_only() {
        assert_eq!(allowed_devices(b"/Users/me", 2, Some(2)), [2]);
        assert_eq!(allowed_devices(b"/Volumes/Backup", 7, Some(2)), [7]);
        assert_eq!(allowed_devices(b"/System/Volumes/Data", 2, Some(2)), [2]);
        assert_eq!(
            allowed_devices(b"", 3, Some(2)),
            [3],
            "an unknown root path adds nothing"
        );
    }

    #[test]
    fn system_volumes_is_left_out_of_a_scan_of_the_root_and_only_of_that() {
        assert!(is_system_volumes(b"/", b"/System/Volumes"));
        assert!(!is_system_volumes(b"/", b"/System"));
        assert!(!is_system_volumes(b"/", b"/System/Volumes/Data"));
        assert!(!is_system_volumes(b"/", b"/Users/me/System/Volumes"));
        assert!(!is_system_volumes(b"/System", b"/System/Volumes"));
        assert!(!is_system_volumes(b"/Users/me", b"/System/Volumes"));
        assert!(hides_system_volumes(b"/"));
        assert!(!hides_system_volumes(b"/System/Volumes/Data"));
    }

    #[test]
    fn everything_below_system_volumes_is_the_systems_own() {
        assert!(is_inside_system_volumes(b"/System/Volumes"));
        assert!(is_inside_system_volumes(b"/System/Volumes/Data"));
        assert!(is_inside_system_volumes(b"/System/Volumes/VM"));
        assert!(!is_inside_system_volumes(b"/System/VolumesX"));
        assert!(!is_inside_system_volumes(b"/System"));
        assert!(!is_inside_system_volumes(b"/"));
        assert!(!is_inside_system_volumes(b"/Volumes/Backup"));
    }

    #[test]
    fn virtual_and_network_filesystems_are_never_entered_and_disks_are() {
        for fs_type in ["devfs", "autofs", "smbfs", "nfs", "afpfs", "webdav"] {
            assert!(is_excluded_fs_type(fs_type), "{fs_type}");
        }
        for fs_type in ["apfs", "hfs", "msdos", "exfat", "ntfs", "udf", "cd9660", "tmpfs"] {
            assert!(!is_excluded_fs_type(fs_type), "{fs_type}");
        }
    }

    #[test]
    fn the_indexes_and_logs_of_a_volume_root_are_excluded_whatever_their_case() {
        for name in [
            ".Spotlight-V100",
            ".fseventsd",
            ".DocumentRevisions-V100",
            ".Trashes",
            ".MobileBackups",
            ".spotlight-v100",
            ".FSEVENTSD",
        ] {
            assert!(is_excluded_volume_root_name(name.as_bytes()), "{name}");
        }
        for name in [".Trash", "Users", ".Spotlight", ".fseventsd2", "", "Trashes"] {
            assert!(!is_excluded_volume_root_name(name.as_bytes()), "{name}");
        }
    }

    #[test]
    fn a_mount_below_a_firmlink_is_reported_on_the_data_volume() {
        assert!(is_mount_point_of(b"/Volumes/Backup", b"/Volumes/Backup"));
        assert!(is_mount_point_of(b"/System/Volumes/Data/home", b"/home"));
        assert!(is_mount_point_of(
            b"/System/Volumes/Data/Users/me/nas",
            b"/Users/me/nas"
        ));
        assert!(!is_mount_point_of(b"/Volumes/Backup", b"/Volumes/Other"));
        assert!(!is_mount_point_of(b"/home", b"/net"));
        assert!(
            !is_mount_point_of(b"/System/Volumes/DataX/home", b"/home"),
            "only the Data volume's own prefix"
        );
        assert!(
            !is_mount_point_of(b"/System/Volumes/Data", b"/"),
            "the Data volume is not `/`"
        );
    }

    #[test]
    fn a_mount_is_entered_by_its_device_or_by_what_it_is() {
        let allowed = [1, 2];

        // The same disk: no question about its type.
        assert_eq!(mount_verdict(&allowed, true, 2, None), Ok(()));
        assert_eq!(mount_verdict(&allowed, true, 1, Some("devfs")), Ok(()));

        // Another device: only a known, ordinary filesystem, and not when the scan stays on one.
        assert_eq!(mount_verdict(&allowed, false, 9, Some("apfs")), Ok(()));
        assert_eq!(
            mount_verdict(&allowed, true, 9, Some("apfs")),
            Err(SkipReason::OtherDisk)
        );
        assert_eq!(mount_verdict(&allowed, false, 9, None), Err(SkipReason::Unknown));
        assert_eq!(mount_verdict(&allowed, true, 9, None), Err(SkipReason::Unknown));
    }

    #[test]
    fn a_virtual_or_network_mount_is_a_system_mount_with_or_without_one_file_system() {
        for one_file_system in [false, true] {
            for fs_type in ["devfs", "autofs", "smbfs", "nfs", "afpfs", "webdav"] {
                assert_eq!(
                    mount_verdict(&[1], one_file_system, 9, Some(fs_type)),
                    Err(SkipReason::System),
                    "{fs_type}, one_file_system = {one_file_system}"
                );
            }
        }
    }
}
