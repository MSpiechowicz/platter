//! Filesystem labels from `/dev/disk/by-label` (udev maintains these symlinks).

use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

const BY_LABEL: &str = "/dev/disk/by-label";

/// Whether a mount's source can name a block device whose label is worth looking up: the filesystem is
/// not a FUSE one, and the source is a path under `/dev`. The source of a FUSE mount is whatever its
/// owner chose (`fsname=/dev/sda1` is accepted), so it could borrow a real disk's label. Plain `fuse`
/// and `fuse.*` are userspace filesystems; `fuseblk` is block-backed (NTFS through ntfs-3g) and fine.
pub fn is_labelable(source: &str, fs_type: &str) -> bool {
    let userspace = crate::pseudo::is_userspace_fs(fs_type);

    !userspace && Path::new(source).starts_with("/dev") && Path::new(source) != Path::new("/dev")
}

/// Label of the block device `source` (a mount source such as `/dev/sda1`) that carries a `fs_type`
/// filesystem, if udev knows one. Anything that is not a real block device under `/dev` has none.
pub fn label_of(source: &str, fs_type: &str) -> Option<String> {
    if !is_labelable(source, fs_type) {
        return None;
    }

    // `/dev/mapper/x` and friends are symlinks: follow them, and insist that what they lead to is a
    // block device in `/dev` (the lexical check above does not see `..` or a symlink out of `/dev`).
    let wanted = fs::canonicalize(source).ok()?;
    if !wanted.starts_with("/dev") || !fs::metadata(&wanted).ok()?.file_type().is_block_device() {
        return None;
    }

    fs::read_dir(Path::new(BY_LABEL))
        .ok()?
        .flatten()
        .find_map(|entry| {
            let target = fs::canonicalize(entry.path()).ok()?;
            (target == wanted).then(|| unescape(&entry.file_name().to_string_lossy()))
        })
}

/// udev escapes unusual bytes in link names as `\xNN`.
fn unescape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;

    while let Some(position) = rest.find("\\x") {
        out.push_str(&rest[..position]);
        let hex = rest
            .get(position + 2..position + 4)
            .and_then(|h| u8::from_str_radix(h, 16).ok());

        match hex {
            Some(byte) => {
                out.push(char::from(byte));
                rest = &rest[position + 4..];
            }
            None => {
                out.push_str("\\x");
                rest = &rest[position + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::{is_labelable, label_of, unescape};

    #[test]
    fn only_real_block_device_sources_of_non_fuse_filesystems_are_looked_up() {
        for (source, fs_type) in [
            ("/dev/sda1", "ext4"),
            ("/dev/nvme0n1p2", "btrfs"),
            ("/dev/mapper/vg-root", "xfs"),
            ("/dev/sdb1", "fuseblk"),
        ] {
            assert!(is_labelable(source, fs_type), "{source} {fs_type}");
        }
        for (source, fs_type) in [
            ("/dev/sda1", "fuse"),
            ("/dev/sda1", "fuse.sshfs"),
            ("/dev/sda1", "fuse.gocryptfs"),
            ("me@host:/export", "fuse.sshfs"),
            ("tmpfs", "tmpfs"),
            ("nas:/export", "nfs4"),
            ("/home/me/disk.img", "ext4"),
            ("/devious/sda1", "ext4"),
            ("/dev", "ext4"),
            ("", "ext4"),
        ] {
            assert!(!is_labelable(source, fs_type), "{source} {fs_type}");
        }
    }

    #[test]
    fn a_fuse_mount_cannot_borrow_the_label_of_a_real_disk() {
        // Whatever the machine has, a FUSE `fsname` of `/dev/...` gets no label from the disk it names.
        for source in ["/dev/sda1", "/dev/null", "/dev/disk/by-label/anything"] {
            assert_eq!(label_of(source, "fuse.sshfs"), None);
            assert_eq!(label_of(source, "fuse"), None);
        }
    }

    #[test]
    fn a_source_that_is_not_a_block_device_has_no_label() {
        assert_eq!(label_of("/dev/null", "ext4"), None, "a character device");
        assert_eq!(label_of("/dev/../etc/passwd", "ext4"), None, "leaves /dev");
        assert_eq!(label_of("/dev/does-not-exist", "ext4"), None);
    }

    #[test]
    fn decodes_udev_escapes() {
        assert_eq!(unescape("My\\x20Disk"), "My Disk");
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape("odd\\xZZ"), "odd\\xZZ");
    }
}
