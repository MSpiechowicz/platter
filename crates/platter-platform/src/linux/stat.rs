//! `statx` plumbing: one call per entry, never following symlinks.

use std::ffi::CStr;
use std::io;
use std::os::fd::AsFd;

use platter_core::{EntryKind, EntryMeta};
use rustix::fs::{AtFlags, FileType, Statx, StatxAttributes, StatxFlags, makedev, statx};

const MASK: StatxFlags = StatxFlags::TYPE
    .union(StatxFlags::MODE)
    .union(StatxFlags::NLINK)
    .union(StatxFlags::INO)
    .union(StatxFlags::SIZE)
    .union(StatxFlags::BLOCKS)
    .union(StatxFlags::MTIME)
    .union(StatxFlags::MNT_ID);

/// What checking a directory against its listing needs: device (always reported), inode, mount id.
const IDENTITY_MASK: StatxFlags = StatxFlags::TYPE.union(StatxFlags::INO).union(StatxFlags::MNT_ID);

/// Flags of every `statx` the walker makes. `NO_AUTOMOUNT` matters most: without it, looking at an
/// autofs or `x-systemd.automount` point mounts whatever is behind it, usually a network share that
/// the exclusion list (read before the scan) knows nothing about. With it, `statx` reports the trigger
/// directory itself, which is an autofs mount and therefore excluded.
const STAT_AT_FLAGS: AtFlags = AtFlags::SYMLINK_NOFOLLOW
    .union(AtFlags::NO_AUTOMOUNT)
    .union(AtFlags::STATX_DONT_SYNC);
const STAT_FD_FLAGS: AtFlags = AtFlags::EMPTY_PATH
    .union(AtFlags::NO_AUTOMOUNT)
    .union(AtFlags::STATX_DONT_SYNC);

/// What `statx` told us about one entry.
#[derive(Debug)]
pub(super) struct Stat {
    pub meta: EntryMeta,
    /// Kernel mount id of the mount the entry lives on (kernel 5.8+).
    pub mount_id: Option<u64>,
    /// Whether the entry is a mount root, as the kernel says; `None` when the kernel does not know the
    /// attribute (before 5.8). `meta.is_mount_root` is `false` then, until the reader decides with
    /// [`is_boundary`].
    pub reported_mount_root: Option<bool>,
}

/// Whether the entry starts another mount. With a kernel that reports it (`reported`), that settles it.
/// Without (before 5.8) the best evidence left is a different `st_dev` than the directory it was found
/// in; the reader then also requires the device to belong to a mount it may enter (see
/// [`MountSnapshot::may_enter_by_device`](super::MountSnapshot::may_enter_by_device)). A btrfs
/// subvolume has a device of its own without being a mount, so on such kernels it counts as a boundary
/// and stays out of the scan.
pub(super) fn is_boundary(reported: Option<bool>, entry_dev: u64, parent_dev: u64) -> bool {
    reported.unwrap_or(entry_dev != parent_dev)
}

/// The mount-root attribute, or `None` when the kernel does not report it (`mask` is what it supports).
fn mount_root(mask: StatxAttributes, attributes: StatxAttributes) -> Option<bool> {
    mask.contains(StatxAttributes::MOUNT_ROOT)
        .then(|| attributes.contains(StatxAttributes::MOUNT_ROOT))
}

/// Stats `name` relative to `dir` without following a trailing symlink.
pub(super) fn stat_at(dir: impl AsFd, name: &CStr) -> io::Result<Stat> {
    Ok(convert(&statx(dir, name, STAT_AT_FLAGS, MASK)?))
}

/// Stats the directory behind `fd` itself.
pub(super) fn stat_fd(fd: impl AsFd) -> io::Result<Stat> {
    Ok(convert(&statx(fd, c"", STAT_FD_FLAGS, MASK)?))
}

/// Like [`stat_fd`] but asks only for device, inode and mount id (the sizes and times of the other
/// fields are left zero): enough to check that an opened directory is the one that was listed.
pub(super) fn stat_fd_identity(fd: impl AsFd) -> io::Result<Stat> {
    Ok(convert(&statx(fd, c"", STAT_FD_FLAGS, IDENTITY_MASK)?))
}

fn convert(raw: &Statx) -> Stat {
    let kind = match FileType::from_raw_mode(u32::from(raw.stx_mode)) {
        FileType::Directory => EntryKind::Dir,
        FileType::RegularFile => EntryKind::File,
        FileType::Symlink => EntryKind::Symlink,
        _ => EntryKind::Other,
    };

    // Only trust the flag when the kernel says it knows it (it does since 5.8).
    let reported_mount_root = mount_root(raw.stx_attributes_mask, raw.stx_attributes);
    let has_mount_id = StatxFlags::from_bits_retain(raw.stx_mask).contains(StatxFlags::MNT_ID);

    Stat {
        meta: EntryMeta {
            kind,
            allocated: raw.stx_blocks.saturating_mul(512),
            apparent: raw.stx_size,
            mtime: raw.stx_mtime.tv_sec,
            dev: makedev(raw.stx_dev_major, raw.stx_dev_minor),
            ino: raw.stx_ino,
            nlink: u64::from(raw.stx_nlink),
            is_mount_root: reported_mount_root == Some(true),
        },
        mount_id: has_mount_id.then_some(raw.stx_mnt_id),
        reported_mount_root,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_statx_refuses_to_trigger_automounts_and_never_follows_symlinks() {
        assert!(STAT_AT_FLAGS.contains(AtFlags::NO_AUTOMOUNT));
        assert!(STAT_AT_FLAGS.contains(AtFlags::SYMLINK_NOFOLLOW));
        assert!(STAT_FD_FLAGS.contains(AtFlags::NO_AUTOMOUNT));
        assert!(STAT_FD_FLAGS.contains(AtFlags::EMPTY_PATH));
    }

    #[test]
    fn a_kernel_that_reports_the_mount_root_attribute_settles_the_question() {
        let known = StatxAttributes::MOUNT_ROOT;
        let none = StatxAttributes::empty();

        assert_eq!(mount_root(known, known), Some(true));
        assert_eq!(mount_root(known, none), Some(false));

        // Same device or not, the kernel's word wins.
        assert!(is_boundary(Some(true), 5, 5));
        assert!(!is_boundary(Some(false), 5, 6));
    }

    #[test]
    fn without_the_attribute_a_different_device_marks_a_boundary() {
        // Before kernel 5.8 the mask lacks `MOUNT_ROOT`: nothing is reported.
        assert_eq!(
            mount_root(StatxAttributes::empty(), StatxAttributes::MOUNT_ROOT),
            None
        );

        assert!(is_boundary(None, 0x802, 0x801), "another device than the parent");
        assert!(
            !is_boundary(None, 0x801, 0x801),
            "the same device is an ordinary directory"
        );
    }
}
