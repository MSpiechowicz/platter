//! Conversion from `std::fs::Metadata` to core's `EntryMeta`. The only OS-conditional part of the
//! generic reader.

use std::fs::{FileType, Metadata};

use platter_core::{EntryKind, EntryMeta};

pub(super) fn kind_of(file_type: FileType) -> EntryKind {
    if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// Unix: real blocks, device, inode and link count.
#[cfg(unix)]
pub(crate) fn entry_meta(meta: &Metadata) -> EntryMeta {
    use std::os::unix::fs::MetadataExt;

    EntryMeta {
        kind: kind_of(meta.file_type()),
        allocated: meta.blocks().saturating_mul(512),
        apparent: meta.len(),
        mtime: meta.mtime(),
        dev: meta.dev(),
        ino: meta.ino(),
        nlink: meta.nlink(),
        is_mount_root: false,
    }
}

/// Elsewhere: allocated falls back to the file length and there is no hardlink information.
#[cfg(not(unix))]
pub(crate) fn entry_meta(meta: &Metadata) -> EntryMeta {
    use std::time::UNIX_EPOCH;

    let mtime = match meta.modified().map(|time| time.duration_since(UNIX_EPOCH)) {
        Ok(Ok(since)) => since.as_secs() as i64,
        Ok(Err(before)) => -(before.duration().as_secs() as i64),
        Err(_) => 0,
    };

    EntryMeta {
        kind: kind_of(meta.file_type()),
        allocated: meta.len(),
        apparent: meta.len(),
        mtime,
        dev: 0,
        ino: 0,
        nlink: 1,
        is_mount_root: false,
    }
}
