//! Parser for `/proc/self/mountinfo`.
//!
//! The table is bytes, not text: a mount point or a root can be any byte string, and one that is not
//! valid UTF-8 must not make every other mount disappear. So the file is read as bytes, split into
//! lines, and each line is parsed on its own; only a line that cannot be parsed at all is skipped.
//! Paths stay `PathBuf`s (the exact bytes), the short ASCII fields are text.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

/// The fields of a mountinfo line the scanner cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    /// Kernel mount id; equals `stx_mnt_id` from `statx` (kernel 5.8+).
    pub mount_id: u64,
    /// Id of the mount this one is mounted on (the mount id of the parent; itself for the namespace root).
    pub parent_id: u64,
    /// `major:minor` of the superblock. Every mount of one filesystem shares it (all subvolume mounts
    /// of a btrfs filesystem do), unlike the `st_dev` of the files, which differs per subvolume.
    pub device: String,
    /// Path inside the filesystem that is mounted here: `/` for a whole filesystem, `/@home` for a
    /// btrfs subvolume or a bind mount of that directory.
    pub root: PathBuf,
    pub mount_point: PathBuf,
    pub fs_type: String,
    /// The mount is read-only (`ro` among its mount options, not the filesystem's).
    pub read_only: bool,
    /// The mount source as the kernel reports it (a device path, `tmpfs`, `host:/export`, ...).
    pub source: OsString,
    /// The filesystem compresses what is written to it (btrfs `compress=` or `compress-force=` among its
    /// options), so a file takes less space on the disk than its size says.
    pub compressed: bool,
}

/// Parses mountinfo bytes; a line that cannot be parsed is skipped, the others are kept.
///
/// Line format: `id parent major:minor root mount_point options [optional...] - fs_type source super_options`.
pub fn parse(table: &[u8]) -> Vec<MountEntry> {
    table
        .split(|&byte| byte == b'\n')
        .filter_map(parse_line)
        .collect()
}

fn parse_line(line: &[u8]) -> Option<MountEntry> {
    let separator = line.windows(3).position(|window| window == b" - ")?;
    let (left, right) = (&line[..separator], &line[separator + 3..]);

    let mut left = left.split(|&byte| byte == b' ');
    let mount_id = number(left.next()?)?;
    let parent_id = number(left.next()?)?;
    let device = text(left.next()?);
    let root = left.next()?;
    let mount_point = left.next()?;
    let read_only = left
        .next()
        .is_some_and(|options| options.split(|&byte| byte == b',').any(|option| option == b"ro"));

    let mut right = right.split(|&byte| byte == b' ');
    let fs_type = text(right.next()?);
    let source = right.next().unwrap_or_default();
    let compressed = right.next().is_some_and(compresses);

    Some(MountEntry {
        mount_id,
        parent_id,
        device,
        root: PathBuf::from(OsString::from_vec(unescape(root))),
        mount_point: PathBuf::from(OsString::from_vec(unescape(mount_point))),
        fs_type,
        read_only,
        source: OsString::from_vec(unescape(source)),
        compressed,
    })
}

/// Whether the super options turn compression on: `compress=zstd:1`, `compress-force=lzo`, but not
/// `compress=no` (nor `compress=none`), which turns it off.
fn compresses(options: &[u8]) -> bool {
    options.split(|&byte| byte == b',').any(|option| {
        let value = option
            .strip_prefix(b"compress-force=")
            .or_else(|| option.strip_prefix(b"compress="));

        match value {
            Some(value) => !matches!(value, b"no" | b"none"),
            None => option == b"compress" || option == b"compress-force",
        }
    })
}

fn number(field: &[u8]) -> Option<u64> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

fn text(field: &[u8]) -> String {
    String::from_utf8_lossy(field).into_owned()
}

/// Decodes the `\040`-style octal escapes the kernel uses for spaces, tabs, newlines and backslashes.
fn unescape(field: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len());
    let mut i = 0;

    while i < field.len() {
        let value = match (field[i], field.get(i + 1..i + 4)) {
            (b'\\', Some(digits)) => octal(digits),
            _ => None,
        };

        match value {
            Some(value) => {
                out.push(value);
                i += 4;
            }
            None => {
                out.push(field[i]);
                i += 1;
            }
        }
    }
    out
}

/// Three octal digits as a byte; `None` when they are not digits or do not fit.
fn octal(digits: &[u8]) -> Option<u8> {
    if !digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
        return None;
    }

    let value = digits
        .iter()
        .fold(0u32, |acc, digit| acc * 8 + u32::from(digit - b'0'));
    u8::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const SAMPLE: &str = "\
24 1 0:21 / /proc rw,nosuid,nodev,noexec,relatime shared:12 - proc proc rw
26 1 259:2 /@home /home rw,relatime shared:3 - btrfs /dev/nvme0n1p2 rw,ssd,subvolid=256
31 1 8:1 / /run/media/me/My\\040Disk rw,relatime - ntfs3 /dev/sda1 rw
garbage line
";

    #[test]
    fn parses_ids_mount_points_and_types() {
        let mounts = parse(SAMPLE.as_bytes());

        assert_eq!(mounts.len(), 3);
        assert_eq!(
            mounts[0],
            MountEntry {
                mount_id: 24,
                parent_id: 1,
                device: "0:21".into(),
                root: "/".into(),
                mount_point: "/proc".into(),
                fs_type: "proc".into(),
                read_only: false,
                source: "proc".into(),
                compressed: false,
            }
        );
        assert_eq!(mounts[1].fs_type, "btrfs");
        assert_eq!(mounts[1].source, "/dev/nvme0n1p2");
        assert_eq!(mounts[1].device, "259:2");
        assert_eq!(mounts[1].root, Path::new("/@home"));
        assert_eq!(mounts[1].parent_id, 1);
        assert!(!mounts[1].compressed);
    }

    #[test]
    fn compression_is_read_from_the_super_options() {
        let table = "\
1 0 0:32 / / rw - btrfs /dev/sda2 rw,compress=zstd:1,ssd
2 0 0:32 / /b rw - btrfs /dev/sda2 rw,compress-force=lzo
3 0 0:32 / /c rw - btrfs /dev/sda2 rw,compress=no
4 0 0:32 / /d rw - btrfs /dev/sda2 rw,ssd
5 0 0:33 / /e rw - ext4 /dev/sda3
";
        let compressed: Vec<bool> = parse(table.as_bytes())
            .iter()
            .map(|mount| mount.compressed)
            .collect();

        assert_eq!(compressed, [true, true, false, false, false]);
    }

    #[test]
    fn a_read_only_mount_is_told_from_a_read_only_filesystem() {
        let table = "\
1 0 0:32 / / ro,noatime - btrfs /dev/sda2 rw
2 1 0:32 /@a /a rw,noatime - btrfs /dev/sda2 ro
3 1 0:32 /@b /b rw,relatime,errors=remount-ro - ext4 /dev/sda3 rw
";
        let mounts = parse(table.as_bytes());

        assert!(mounts[0].read_only);
        assert!(!mounts[1].read_only, "only the mount's own options count");
        assert!(!mounts[2].read_only, "`remount-ro` is not `ro`");
    }

    #[test]
    fn decodes_escaped_mount_points() {
        assert_eq!(
            parse(SAMPLE.as_bytes())[2].mount_point,
            Path::new("/run/media/me/My Disk")
        );
    }

    #[test]
    fn decodes_escaped_backslashes_newlines_and_tabs_and_leaves_odd_escapes_alone() {
        let line = b"5 1 0:9 /a\\134b\\012c\\011d /m\\134 rw - ext4 /dev/x\\8 rw\n";

        let mount = &parse(line)[0];

        assert_eq!(mount.root, Path::new("/a\\b\nc\td"));
        assert_eq!(mount.mount_point, Path::new("/m\\"));
        assert_eq!(mount.source, "/dev/x\\8", "not an octal escape: kept as written");
    }

    #[test]
    fn a_mount_point_that_is_not_valid_utf8_does_not_hide_the_other_mounts() {
        let mut table = Vec::new();
        table.extend_from_slice(b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n");
        table.extend_from_slice(b"2 1 8:2 / /mnt/caf\xff\xe9 rw - ext4 /dev/sda2 rw\n");
        table.extend_from_slice(b"3 1 8:3 / /data rw - ext4 /dev/sda3 rw\n");

        let mounts = parse(&table);

        assert_eq!(
            mounts.len(),
            3,
            "the line with byte 0xFF is kept, and so are the others"
        );
        assert_eq!(
            mounts[1].mount_point.as_os_str().as_encoded_bytes(),
            b"/mnt/caf\xff\xe9",
            "the exact bytes are preserved"
        );
        assert_eq!(mounts[2].mount_point, Path::new("/data"));
    }

    #[test]
    fn a_non_utf8_escape_in_a_mount_point_round_trips_too() {
        // The kernel escapes only space, tab, newline and backslash, but any other octal escape works.
        let mounts = parse(b"4 1 8:4 / /x\\377y rw - ext4 /dev/sda4 rw\n");

        assert_eq!(mounts[0].mount_point.as_os_str().as_encoded_bytes(), b"/x\xffy");
    }

    #[test]
    fn only_a_line_that_cannot_be_parsed_is_skipped() {
        let table = b"no separator here\n\
                      x y 0:1 / / rw - ext4 /dev/a rw\n\
                      7 1 0:7 / /ok rw - ext4 /dev/a rw\n\
                      \n";

        let mounts = parse(table);

        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].mount_id, 7);
    }
}
