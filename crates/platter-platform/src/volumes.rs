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

                let Identity { id, device, label } = identify(
                    disk.name().to_string_lossy().into_owned(),
                    &fs_type,
                    &mount_point,
                    disk.is_removable(),
                );
                Some(Volume {
                    id,
                    label,
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

/// How a volume is named to the user.
struct Identity {
    id: String,
    device: String,
    label: String,
}

/// The id, device and label of a volume, from the name `sysinfo` gives the disk (the device node on
/// Linux, the volume label on Windows).
#[cfg(not(windows))]
fn identify(name: String, fs_type: &str, mount_point: &Path, _removable: bool) -> Identity {
    let label = label_for(&name, fs_type, mount_point);

    Identity {
        id: mount_point.to_string_lossy().into_owned(),
        device: name,
        label,
    }
}

/// On Windows a drive is known by its letter: that is its id and its device (the page lists the device
/// next to the label, so two drives never merge into one row), and the label is the volume label.
#[cfg(windows)]
fn identify(name: String, _fs_type: &str, mount_point: &Path, removable: bool) -> Identity {
    let mount = mount_point.to_string_lossy();
    let drive = drive_letter(&mount);

    Identity {
        id: drive.clone().unwrap_or_else(|| mount.clone().into_owned()),
        device: drive.unwrap_or_default(),
        label: windows_label(&name, removable),
    }
}

/// The label of a Windows volume: the volume label, else the name Explorer gives a drive without one.
/// Never `Root` (every drive is the root of its own tree). The drive letter is the device.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_label(volume_label: &str, removable: bool) -> String {
    let label = volume_label.trim();

    if !label.is_empty() {
        label.to_owned()
    } else if removable {
        "Removable Disk".to_owned()
    } else {
        "Local Disk".to_owned()
    }
}

/// `C:` for the mount point `C:\` (or `c:`), `None` for anything else (a volume mounted in a folder).
#[cfg_attr(not(windows), allow(dead_code))]
fn drive_letter(mount_point: &str) -> Option<String> {
    let mut chars = mount_point.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next() != Some(':') {
        return None;
    }
    if !matches!(chars.as_str(), "" | "\\" | "/") {
        return None;
    }

    Some(format!("{}:", letter.to_ascii_uppercase()))
}

/// Filesystem label when the OS knows one, else the mount point's last component, else `Root`.
#[cfg(not(windows))]
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

#[cfg(not(any(target_os = "linux", windows)))]
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

    #[cfg(not(windows))]
    #[test]
    fn labels_fall_back_to_the_mount_point_name() {
        assert_eq!(
            label_for("/dev/none-existent", "ext4", Path::new("/mnt/backup")),
            "backup"
        );
        assert_eq!(label_for("/dev/none-existent", "ext4", Path::new("/")), "Root");
    }

    #[test]
    fn a_windows_volume_is_labelled_by_its_volume_label_and_never_called_root() {
        assert_eq!(windows_label("Windows", false), "Windows");
        assert_eq!(windows_label(" Games ", false), "Games");
        assert_eq!(windows_label("", false), "Local Disk");
        assert_eq!(windows_label("  ", true), "Removable Disk");
    }

    #[test]
    fn a_drive_letter_is_only_a_drive_root() {
        assert_eq!(drive_letter("C:\\").as_deref(), Some("C:"));
        assert_eq!(drive_letter("C:\\mnt"), None);
        assert_eq!(drive_letter("1:\\"), None);
        assert_eq!(drive_letter("/mnt/c"), None);
        assert_eq!(drive_letter(""), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_system_drive_is_listed_by_its_letter_and_never_called_root() {
        let drive = std::env::var("SystemDrive")
            .unwrap_or_else(|_| "C:".into())
            .to_ascii_uppercase();

        let volumes = SystemVolumes.list();

        let system = volumes
            .iter()
            .find(|volume| volume.id == drive)
            .unwrap_or_else(|| panic!("{drive} is listed in {volumes:?}"));
        assert_eq!(system.device, drive);
        assert_eq!(system.mount_point, Path::new(&format!("{drive}\\")));
        assert_ne!(system.label, "Root");
        assert!(system.total > 0 && system.available <= system.total);
        for volume in &volumes {
            assert!(!volume.label.is_empty() && volume.label != "Root", "{volume:?}");
        }
    }
}
