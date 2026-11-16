//! Filesystem types that are not user data. Shared by the Linux exclusion rules and volume listing.

/// Exact filesystem type names of kernel-provided pseudo filesystems.
pub const PSEUDO_FS_TYPES: &[&str] = &[
    "proc",
    "sysfs",
    "devtmpfs",
    "devpts",
    "tmpfs",
    "ramfs",
    "securityfs",
    "debugfs",
    "tracefs",
    "configfs",
    "pstore",
    "bpf",
    "mqueue",
    "hugetlbfs",
    "autofs",
    "binfmt_misc",
    "fusectl",
    "efivarfs",
    "selinuxfs",
    "nsfs",
    "rpc_pipefs",
    "cgroup",
    "cgroup2",
];

/// Prefixes that identify a family of pseudo filesystem types.
pub const PSEUDO_FS_PREFIXES: &[&str] = &["cgroup"];

pub fn is_pseudo_fs(fs_type: &str) -> bool {
    PSEUDO_FS_TYPES.contains(&fs_type)
        || PSEUDO_FS_PREFIXES
            .iter()
            .any(|prefix| fs_type.starts_with(prefix))
}

/// Userspace filesystems: plain `fuse` and `fuse.<subtype>`. Not `fuseblk`, the block-backed kind (NTFS
/// through ntfs-3g) that is a real disk.
pub fn is_userspace_fs(fs_type: &str) -> bool {
    fs_type == "fuse" || fs_type.starts_with("fuse.")
}
