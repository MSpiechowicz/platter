//! What identifies a file or folder on Windows, read through an open handle.
//!
//! The reader, the probe and the remover all compare the same thing, so it is read in one place. In terms
//! of the core ports: `dev` is the volume serial number, `ino` is the 64-bit file index, `nlink` the
//! link count, `birth` the creation time and `mount_id` is `None`.
//!
//! A path is opened without following a reparse point (symbolic link, junction, mount point, cloud
//! placeholder) and with backup semantics (so a folder can be opened). Every reparse point is treated as
//! a link by the callers: it is never descended into and never acted through.
//!
//! The file index is only unique within a volume, and on ReFS it may not be unique at all (the 128-bit id
//! does not fit; `GetFileInformationByHandle` reports a 64-bit one). Callers that must not confuse two
//! files compare the volume serial, the index and the creation time together.

use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use platter_core::BirthTime;
use winapi_util::file::{Information, information};

/// `FILE_FLAG_BACKUP_SEMANTICS`: lets `CreateFile` open a folder.
pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
/// `FILE_FLAG_OPEN_REPARSE_POINT`: opens the reparse point itself and does not follow it.
pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// `FILE_ATTRIBUTE_DIRECTORY`.
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
/// `FILE_ATTRIBUTE_REPARSE_POINT`: a symbolic link, junction, mount point or placeholder.
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
/// `FILE_READ_ATTRIBUTES`: enough to ask for the identity, so a file nobody may read can still be looked at.
pub const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`: looking at a file must not block anybody.
pub const FILE_SHARE_ALL: u32 = 0x0000_0007;

/// Seconds between 1601-01-01 (the start of a `FILETIME`) and 1970-01-01.
const FILETIME_UNIX_EPOCH_SECS: i64 = 11_644_473_600;
/// A `FILETIME` counts 100 ns ticks.
const TICKS_PER_SECOND: u64 = 10_000_000;

/// What one open file or folder is, as the system reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    /// Serial number of the volume the file is on (a 32-bit number, kept in 64 bits like `dev`).
    pub volume_serial: u64,
    /// The file index: unique within the volume (not guaranteed on ReFS).
    pub file_index: u64,
    /// Number of hard links to the file.
    pub links: u64,
    /// Creation time as a `FILETIME` (100 ns ticks since 1601); `None` when the system gives none.
    pub creation_time: Option<u64>,
    /// `dwFileAttributes`.
    pub attributes: u32,
    /// File length in bytes (0 for a folder).
    pub size: u64,
}

impl FileIdentity {
    fn from_information(info: &Information) -> Self {
        Self {
            volume_serial: info.volume_serial_number(),
            file_index: info.file_index(),
            links: info.number_of_links(),
            creation_time: info.creation_time(),
            attributes: info.file_attributes() as u32,
            size: info.file_size(),
        }
    }

    /// A symbolic link, junction, mount point or placeholder: never followed.
    pub fn is_reparse_point(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    pub fn is_dir(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    /// The creation time as the ports carry it.
    pub fn birth(&self) -> Option<BirthTime> {
        self.creation_time.and_then(birth_time)
    }
}

/// Opens `path` to look at it: no access to its contents is asked for, nobody is kept from using it, a
/// reparse point is opened itself and not followed, and a folder can be opened.
pub fn open_for_identity(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

/// The identity of what `file` holds open.
pub fn identity_of(file: &File) -> io::Result<FileIdentity> {
    information(file).map(|info| FileIdentity::from_information(&info))
}

/// The identity of the entry at `path` (a reparse point at the last name is described, not followed).
pub fn identity_of_path(path: &Path) -> io::Result<FileIdentity> {
    identity_of(&open_for_identity(path)?)
}

/// Whether `meta` (from `symlink_metadata`, or of an open handle) describes a reparse point.
pub fn is_reparse_point(meta: &Metadata) -> bool {
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// A `FILETIME` as the ports carry a creation time. `None` for 0 (no time recorded).
pub fn birth_time(filetime: u64) -> Option<BirthTime> {
    if filetime == 0 {
        return None;
    }

    let secs = i64::try_from(filetime / TICKS_PER_SECOND).ok()? - FILETIME_UNIX_EPOCH_SECS;
    let nanos = (filetime % TICKS_PER_SECOND) as u32 * 100;

    Some(BirthTime { secs, nanos })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filetime_becomes_unix_seconds_and_nanoseconds() {
        // 2001-09-09T01:46:40Z, 1_000_000_000 unix seconds, plus 1.5 microseconds.
        let ticks = (1_000_000_000 + FILETIME_UNIX_EPOCH_SECS) as u64 * TICKS_PER_SECOND + 15;

        assert_eq!(
            birth_time(ticks),
            Some(BirthTime {
                secs: 1_000_000_000,
                nanos: 1_500
            })
        );
    }

    #[test]
    fn no_filetime_is_no_birth_time() {
        assert_eq!(birth_time(0), None);
    }

    #[test]
    fn the_same_file_has_the_same_identity_every_time_and_another_file_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::write(&first, b"one").unwrap();
        std::fs::write(&second, b"two").unwrap();

        let once = identity_of_path(&first).unwrap();
        let again = identity_of_path(&first).unwrap();
        let other = identity_of_path(&second).unwrap();

        assert_eq!(once, again);
        assert_ne!(
            (once.volume_serial, once.file_index),
            (other.volume_serial, other.file_index)
        );
        assert_eq!(once.size, 3);
        assert!(!once.is_dir() && !once.is_reparse_point());
        assert!(once.creation_time.is_some());
    }

    #[test]
    fn a_hard_link_shares_the_identity_and_counts_its_links() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original");
        let linked = dir.path().join("linked");
        std::fs::write(&original, b"data").unwrap();
        std::fs::hard_link(&original, &linked).unwrap();

        let a = identity_of_path(&original).unwrap();
        let b = identity_of_path(&linked).unwrap();

        assert_eq!((a.volume_serial, a.file_index), (b.volume_serial, b.file_index));
        assert_eq!(a.links, 2);
    }

    #[test]
    fn a_folder_can_be_opened_and_is_a_folder() {
        let dir = tempfile::tempdir().unwrap();

        let identity = identity_of_path(dir.path()).unwrap();

        assert!(identity.is_dir());
        assert!(!identity.is_reparse_point());
        assert!(!is_reparse_point(&std::fs::symlink_metadata(dir.path()).unwrap()));
    }
}
