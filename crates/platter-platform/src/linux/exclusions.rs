//! Default skip list of the Linux reader, expressed as data.
//!
//! An excluded mount is still listed (as a `mount` node of size 0) but never entered, even without
//! `one_file_system`. The rules never apply to the scan root itself. Overlay filesystems are not
//! excluded, so scanning inside a container works.

use std::collections::HashSet;
use std::path::Path;

use super::mountinfo::MountEntry;
use crate::pseudo::PSEUDO_FS_TYPES;

/// What to skip: mount points by path, and mounts by filesystem type.
#[derive(Debug, Clone, Copy)]
pub struct ExclusionRules {
    /// Mount points skipped whatever their type (compared as paths, so a trailing slash does not matter).
    pub paths: &'static [&'static str],
    /// Groups of exact filesystem type names.
    pub fs_types: &'static [&'static [&'static str]],
    /// Filesystem type prefixes (`fuse.` matches `fuse.sshfs`, but not `fuseblk`).
    pub fs_type_prefixes: &'static [&'static str],
}

/// Network filesystems: slow, and not local disk usage.
const NETWORK_FS_TYPES: &[&str] = &[
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "sshfs",
    "afs",
    "ceph",
    "glusterfs",
    "9p",
];

/// Plain `fuse` (no subtype) is a userspace filesystem of unknown nature, usually a network or virtual
/// one; `fuse.*` is covered by the prefix rule and `fuseblk` (block-backed, e.g. NTFS) stays scanned.
const FUSE_FS_TYPES: &[&str] = &["fuse"];

impl ExclusionRules {
    pub const DEFAULT: ExclusionRules = ExclusionRules {
        paths: &["/proc", "/sys", "/dev", "/run"],
        fs_types: &[PSEUDO_FS_TYPES, NETWORK_FS_TYPES, FUSE_FS_TYPES],
        fs_type_prefixes: &["cgroup", "fuse."],
    };

    /// No exclusions at all.
    pub const NONE: ExclusionRules = ExclusionRules {
        paths: &[],
        fs_types: &[],
        fs_type_prefixes: &[],
    };

    pub fn excludes(&self, mount_point: &Path, fs_type: &str) -> bool {
        self.paths.iter().any(|path| Path::new(path) == mount_point)
            || self.fs_types.iter().any(|group| group.contains(&fs_type))
            || self
                .fs_type_prefixes
                .iter()
                .any(|prefix| fs_type.starts_with(prefix))
    }

    /// Mount ids of the mounts these rules exclude.
    pub fn excluded_mount_ids(&self, mounts: &[MountEntry]) -> HashSet<u64> {
        mounts
            .iter()
            .filter(|mount| self.excludes(&mount.mount_point, &mount.fs_type))
            .map(|mount| mount.mount_id)
            .collect()
    }
}

impl Default for ExclusionRules {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::mountinfo::parse;

    #[test]
    fn skips_pseudo_and_network_filesystems_but_not_real_ones() {
        let rules = ExclusionRules::DEFAULT;

        for fs_type in [
            "proc",
            "sysfs",
            "devtmpfs",
            "tmpfs",
            "cgroup2",
            "cgroup",
            "nfs4",
            "cifs",
            "sshfs",
            "fuse",
            "fuse.sshfs",
            "fuse.portal",
            "autofs",
        ] {
            assert!(
                rules.excludes(Path::new("/somewhere"), fs_type),
                "{fs_type} is skipped"
            );
        }
        for fs_type in [
            "ext4", "btrfs", "xfs", "ntfs3", "vfat", "overlay", "fuseblk", "zfs", "f2fs",
        ] {
            assert!(
                !rules.excludes(Path::new("/somewhere"), fs_type),
                "{fs_type} is scanned"
            );
        }
    }

    #[test]
    fn skips_well_known_paths_whatever_their_type() {
        let rules = ExclusionRules::DEFAULT;

        for path in ["/proc", "/sys", "/dev", "/run"] {
            assert!(rules.excludes(Path::new(path), "ext4"));
        }
        assert!(!rules.excludes(Path::new("/home"), "ext4"));
        assert!(!rules.excludes(Path::new("/home/proc"), "ext4"));
    }

    #[test]
    fn selects_mount_ids_from_mountinfo() {
        let text = "\
1 0 0:1 / / rw - overlay overlay rw
2 1 0:2 / /proc rw - proc proc rw
3 1 0:3 / /data rw - ext4 /dev/sdb1 rw
4 1 0:4 / /mnt/nas rw - fuse.sshfs me@nas: rw
";
        let ids = ExclusionRules::DEFAULT.excluded_mount_ids(&parse(text.as_bytes()));

        assert_eq!(ids, HashSet::from([2, 4]));
        assert!(
            ExclusionRules::NONE
                .excluded_mount_ids(&parse(text.as_bytes()))
                .is_empty()
        );
    }
}
