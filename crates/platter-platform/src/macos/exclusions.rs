//! Default exclusions on macOS (package M1): which mounts and volume-root folders a scan never enters.
//!
//! The decisions are the pure rules of [`crate::mac_pure::same_disk`]. What this module adds is the
//! facts they need from the system: the kernel's own answer for what is mounted at a path (`statfs`,
//! which reads the mount table without touching the mount, so an `autofs` trigger is not set off), and
//! the device of the Data volume.
//!
//! macOS has no mount ids, so a mount root is recognised by its device differing from the folder it was
//! found in, and classified by the mount the kernel reports at its path. A mount that is not reported at
//! that very path (a table that changed since the listing, or a path that leads somewhere else now) has
//! an unknown filesystem and is not entered.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use platter_core::SkipReason;
use rustix::fs::{AtFlags, CWD, statat, statfs};

use crate::mac_pure::same_disk::{self, DATA_VOLUME};

/// What the kernel says is mounted at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Mount {
    /// The filesystem type name (`apfs`, `smbfs`, `devfs`...).
    pub fs_type: String,
}

/// The mount at `path`, when `path` is a mount point (and not merely a path inside a mount). `None` when
/// the kernel does not answer, or reports a mount at another place.
pub(super) fn mount_at(path: &Path) -> Option<Mount> {
    let stats = statfs(path).ok()?;

    let mounted_on = text_until_nul(&stats.f_mntonname);
    if !same_disk::is_mount_point_of(&mounted_on, path.as_os_str().as_bytes()) {
        return None;
    }

    let fs_type = String::from_utf8(text_until_nul(&stats.f_fstypename)).ok()?;
    Some(Mount { fs_type })
}

/// The bytes of a C string held in a fixed array.
pub(super) fn text_until_nul(raw: &[std::ffi::c_char]) -> Vec<u8> {
    raw.iter()
        .map(|&byte| byte as u8)
        .take_while(|&byte| byte != 0)
        .collect()
}

/// The device of the Data volume, where the system has one (every Mac since Catalina).
pub(super) fn data_volume_device() -> Option<u64> {
    let stat = statat(CWD, DATA_VOLUME, AtFlags::SYMLINK_NOFOLLOW).ok()?;

    Some(stat.st_dev as u64)
}

/// What a scan may enter, decided once at its start from its root.
#[derive(Debug)]
pub(super) struct Scope {
    /// The root, as the canonical path the kernel gives for its descriptor. Empty when it is not known.
    root: Vec<u8>,
    allowed: Vec<u64>,
    one_file_system: bool,
}

impl Scope {
    /// The scope of a scan of `root_path` (`None`: not known), whose root is on device `root_dev`.
    pub(super) fn new(
        root_path: Option<&Path>,
        root_dev: u64,
        data_volume_dev: Option<u64>,
        one_file_system: bool,
    ) -> Self {
        let root = root_path
            .map(|path| path.as_os_str().as_bytes().to_vec())
            .unwrap_or_default();
        let allowed = same_disk::allowed_devices(&root, root_dev, data_volume_dev);

        Self {
            root,
            allowed,
            one_file_system,
        }
    }

    /// Whether `dev` is part of the same disk as the root (firmlinked volumes included), which is not a
    /// mount boundary to anyone looking at the tree.
    pub(super) fn joins(&self, dev: u64) -> bool {
        self.allowed.contains(&dev)
    }

    /// Whether the folder at `path` is one a scan never enters whatever device it is on.
    pub(super) fn leaves_out_folder(&self, path: &Path) -> bool {
        same_disk::is_system_volumes(&self.root, path.as_os_str().as_bytes())
    }

    /// Whether the folder `name` found at the root of a volume is left out.
    pub(super) fn leaves_out_volume_root_folder(&self, name: &[u8]) -> bool {
        same_disk::is_excluded_volume_root_name(name)
    }

    /// Whether the mount root at `path` (`None`: not known), on device `dev`, is entered, and if not, why.
    pub(super) fn mount_verdict(&self, dev: u64, path: Option<&Path>) -> Result<(), SkipReason> {
        let fs_type = path.and_then(mount_at).map(|mount| mount.fs_type);

        same_disk::mount_verdict(&self.allowed, self.one_file_system, dev, fs_type.as_deref())
    }
}
