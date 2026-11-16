//! `VolumeProvider` on top of the `sysinfo` crate.

use std::path::Path;

use platter_core::{Volume, VolumeProvider};
use sysinfo::Disks;

use crate::pseudo::{is_pseudo_fs, is_userspace_fs};

/// Filesystem types that are real but are never what a user means by a volume: snap images. (Desktop
/// plumbing FUSE mounts are userspace filesystems and hidden with all the others.)
const HIDDEN_FS_TYPES: &[&str] = &["squashfs"];

/// Mount point names (last component) that mark temporary mounts, e.g. an AppImage's `.mount_*`.
const HIDDEN_MOUNT_NAME_PREFIXES: &[&str] = &[".mount_"];

/// Lists mounted volumes, hiding pseudo filesystems and userspace (FUSE) filesystems. A FUSE mount is
/// not a disk: its name is chosen by whoever mounted it and what it holds is unknown (a network share, an
/// archive, an encrypted view of another folder), so it is not offered as a volume, can not borrow the
/// label of a real disk, and a scan that walks past one does not enter it. Scanning one is still
/// possible by picking its folder. `fuseblk` (NTFS through ntfs-3g) is a real disk and is listed.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemVolumes;

impl VolumeProvider for SystemVolumes {
    fn list(&self) -> Vec<Volume> {
        let Ok(disks) = Disks::new_with_refreshed_list() else {
            return Vec::new();
        };

        let mut volumes: Vec<Volume> = disks
            .iter()
            .filter_map(|disk| {
                let fs_type = disk.file_system().to_string_lossy().into_owned();
                let mount_point = disk.mount_point().to_path_buf();
                if is_hidden(&fs_type, &mount_point) {
                    return None;
                }

                let device = disk.name().to_string_lossy().into_owned();
                Some(Volume {
                    id: mount_point.to_string_lossy().into_owned(),
                    label: label_for(&device, &fs_type, &mount_point),
                    mount_point,
                    device,
                    fs_type,
                    total: disk.total_space(),
                    available: disk.available_space(),
                    removable: disk.is_removable(),
                })
            })
            .collect();

        volumes.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
        volumes
    }
}

fn is_hidden(fs_type: &str, mount_point: &Path) -> bool {
    let name = mount_point
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();

    is_pseudo_fs(fs_type)
        || is_userspace_fs(fs_type)
        || HIDDEN_FS_TYPES.contains(&fs_type)
        || HIDDEN_MOUNT_NAME_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// Filesystem label when the OS knows one, else the mount point's last component, else `Root`.
fn label_for(device: &str, fs_type: &str, mount_point: &Path) -> String {
    if let Some(label) = os_label(device, fs_type) {
        return label;
    }
    match mount_point.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => "Root".to_owned(),
    }
}

#[cfg(target_os = "linux")]
fn os_label(device: &str, fs_type: &str) -> Option<String> {
    crate::linux::labels::label_of(device, fs_type)
}

#[cfg(not(target_os = "linux"))]
fn os_label(_device: &str, _fs_type: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_real_volumes_without_pseudo_filesystems() {
        let volumes = SystemVolumes.list();

        for volume in &volumes {
            assert!(!is_pseudo_fs(&volume.fs_type), "{} is pseudo", volume.fs_type);
            assert!(volume.available <= volume.total || volume.total == 0);
            assert!(!volume.label.is_empty());
        }
    }

    #[test]
    fn hides_pseudo_snap_and_temporary_mounts() {
        assert!(is_hidden("tmpfs", Path::new("/tmp")));
        assert!(is_hidden("squashfs", Path::new("/snap/core/1")));
        assert!(is_hidden("fuse.some-app", Path::new("/tmp/.mount_someXYZ")));
        assert!(!is_hidden("btrfs", Path::new("/home")));
        assert!(!is_hidden("ext4", Path::new("/")));
        assert!(
            !is_hidden("fuseblk", Path::new("/mnt/windows")),
            "NTFS through ntfs-3g is a disk"
        );
    }

    #[test]
    fn userspace_filesystems_are_not_listed_as_volumes() {
        for fs_type in [
            "fuse",
            "fuse.sshfs",
            "fuse.rclone",
            "fuse.portal",
            "fuse.gvfsd-fuse",
            "fuse.mergerfs",
        ] {
            assert!(is_hidden(fs_type, Path::new("/mnt/nas")), "{fs_type}");
        }
    }

    #[test]
    fn labels_fall_back_to_the_mount_point_name() {
        assert_eq!(
            label_for("/dev/none-existent", "ext4", Path::new("/mnt/backup")),
            "backup"
        );
        assert_eq!(label_for("/dev/none-existent", "ext4", Path::new("/")), "Root");
    }
}
