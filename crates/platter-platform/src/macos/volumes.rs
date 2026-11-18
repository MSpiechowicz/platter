//! Volume rules on macOS (package M1), for the volume list ([`crate::volumes`]).
//!
//! The list hides what a user does not mean by a volume: everything below `/System/Volumes` (the
//! Data volume is the same disk as `/`, and the user sees the boot volume once, as `/`), and the virtual
//! and network filesystems the scan never enters (`devfs`, `autofs`, `smbfs`, `nfs`, `afpfs`,
//! `webdav`). `sysinfo` already leaves out the volumes macOS itself does not show in Finder, so this is
//! the second line, kept in the same rules as the reader's.
//!
//! The device of a volume is the node the kernel says holds it (`/dev/disk3s1s1`), not the volume name
//! `sysinfo` gives, which is the label.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rustix::fs::statfs;

use super::exclusions::text_until_nul;
use crate::mac_pure::same_disk;

/// Whether a volume of filesystem `fs_type` mounted at `mount_point` is left out of the volume list.
pub fn is_hidden(fs_type: &str, mount_point: &Path) -> bool {
    same_disk::is_excluded_fs_type(fs_type)
        || same_disk::is_inside_system_volumes(mount_point.as_os_str().as_bytes())
}

/// The device node that holds the volume mounted at `mount_point`, as the kernel reports it. `None` when
/// it does not say.
pub fn device_of(mount_point: &Path) -> Option<String> {
    let stats = statfs(mount_point).ok()?;
    let device = String::from_utf8(text_until_nul(&stats.f_mntfromname)).ok()?;

    (!device.is_empty()).then_some(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_volumes_and_the_virtual_filesystems_are_hidden() {
        assert!(is_hidden("apfs", Path::new("/System/Volumes/Data")));
        assert!(is_hidden("apfs", Path::new("/System/Volumes/VM")));
        assert!(is_hidden("devfs", Path::new("/dev")));
        assert!(is_hidden("autofs", Path::new("/System/Volumes/Data/home")));
        assert!(is_hidden("smbfs", Path::new("/Volumes/share")));
    }

    #[test]
    fn the_boot_volume_and_external_disks_are_listed() {
        assert!(!is_hidden("apfs", Path::new("/")));
        assert!(!is_hidden("apfs", Path::new("/Volumes/Backup")));
        assert!(!is_hidden("exfat", Path::new("/Volumes/USB")));
    }

    #[test]
    fn the_root_is_on_a_device_node() {
        let device = device_of(Path::new("/")).expect("the kernel names the device of /");

        assert!(device.starts_with('/'), "{device}");
    }
}
