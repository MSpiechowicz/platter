//! The scan's directory reader on Windows.
//!
//! The listing is `std::fs::read_dir`, whose entries already carry the name, the kind, the length, the
//! attributes and the modification time (one `FindNextFile` call per batch, no call per entry). What the
//! listing does not carry is the identity: the volume serial, the file index and the link count. Those
//! need an open handle, so every file and folder is opened once, with no access to its contents (see
//! [`open_for_identity`](super::identity::open_for_identity)). Linux gets the same from `statx` in one call.
//! The cost is one open, one query and one close per entry, spread over the scan's worker threads. It is
//! paid for correctness: hard links are only noticed (and counted once) through the link count, loops and
//! the folders a user left out are only recognised through the identity, and NTFS has nothing cheaper
//! (the directory listing never reports the link count). A file that cannot be opened for its identity
//! keeps what the listing said and only loses the hard link check.
//!
//! What is read as what:
//!
//! - A folder, a symbolic link or a junction that is a reparse point is a *link* (`Symlink`): never
//!   entered, whatever its target (the folder of a cloud service that keeps files on demand, a mount point,
//!   a loop). A volume mounted in a folder is such a link too, so it is not entered.
//! - A file that is a reparse point but not a link (a placeholder of a cloud service, a compressed system
//!   file, a deduplicated file) is a file: it has data and its size counts. A placeholder whose data is
//!   not on this disk counts for 0 bytes.
//! - A folder on another volume than its parent (its serial differs) is a mount root, and with
//!   `one_file_system` it is left alone.
//! - The default [`ExclusionRules`] apply to the entries of a drive's root folder.
//!
//! The allocated size of a file is its length rounded up to the cluster size. There is no safe way to ask
//! the volume for its cluster size (nothing in `std`, `winapi-util` or `sysinfo` reports it), so the size
//! NTFS uses by default, 4 KiB, is assumed. A compressed or sparse file, or one so small that NTFS keeps
//! it inside its record, is over-counted, and a volume with larger clusters under-counted.
//!
//! # Races
//!
//! Like the portable reader, this one works on joined paths. Right before a folder is listed it is opened
//! again and has to be, still, a real folder (not a reparse point) with the identity its parent's listing
//! saw. A swap in the gap between that check and the listing, or one of an ancestor, is still possible.

use std::ffi::OsString;
use std::fs::{self, Metadata, OpenOptions};
use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use platter_core::{
    DirListing, DirSource, EntryKind, EntryMeta, Lease, RawEntry, RootInfo, SkipReason, SourceOptions,
};

use super::exclusions::ExclusionRules;
use super::identity::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
    FILE_SHARE_ALL, FileIdentity, identity_of, identity_of_path,
};
use crate::generic::meta::{entry_meta, round_up};

/// `FILE_ATTRIBUTE_OFFLINE`: the data is not on this disk.
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
/// `FILE_ATTRIBUTE_RECALL_ON_OPEN`: the data is fetched when the file is opened.
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
/// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`: the data is fetched when it is read (a dehydrated placeholder).
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
/// Attributes of a file whose data lives elsewhere (a cloud service), so it takes no room here.
const NOT_ON_THIS_DISK: u32 =
    FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;

/// The cluster size assumed for every volume: the default of NTFS.
const ASSUMED_CLUSTER_BYTES: u64 = 4096;

/// Reads directories through `std::fs`, with the identity of every entry read through an open handle.
#[derive(Debug, Clone, Copy)]
pub struct WindowsDirSource {
    rules: ExclusionRules,
}

impl WindowsDirSource {
    /// Reader with the default exclusions.
    pub fn new() -> Self {
        Self::with_rules(ExclusionRules::DEFAULT)
    }

    pub fn with_rules(rules: ExclusionRules) -> Self {
        Self { rules }
    }
}

impl Default for WindowsDirSource {
    fn default() -> Self {
        Self::new()
    }
}

/// A folder to read: its path and what its parent's listing saw of it.
#[derive(Debug)]
pub struct WindowsHandle {
    path: PathBuf,
    /// Volume serial and file index, `None` when the listing could not read them.
    identity: Option<(u64, u64)>,
    /// The scan root: its path may be a link (it is followed), and everything below must not be.
    is_root: bool,
    one_file_system: bool,
    rules: ExclusionRules,
}

impl WindowsHandle {
    fn volume(&self) -> Option<u64> {
        self.identity.map(|(volume, _)| volume)
    }

    /// Whether this is the root folder of a drive (`C:\`), where the default exclusions apply to what
    /// is listed. The scan root is never excluded: only its children are looked at.
    fn is_drive_root(&self) -> bool {
        self.path.has_root() && self.path.parent().is_none()
    }
}

impl DirSource for WindowsDirSource {
    type Handle = WindowsHandle;

    fn open_root(&self, path: &Path, options: SourceOptions) -> io::Result<RootInfo<WindowsHandle>> {
        // The root itself may be a link; it is followed, and nothing below it is.
        let metadata = fs::metadata(path)?;
        let identity = identity_following(path)?;

        let kind = if identity.is_dir() {
            EntryKind::Dir
        } else {
            EntryKind::File
        };
        let meta = describe(&metadata, kind, Some(&identity));

        Ok(RootInfo {
            handle: WindowsHandle {
                path: path.to_path_buf(),
                identity: Some((identity.volume_serial, identity.file_index)),
                is_root: true,
                one_file_system: options.one_file_system,
                rules: self.rules,
            },
            meta,
            warnings: Vec::new(),
        })
    }

    fn read_dir(&self, dir: &WindowsHandle, lease: &mut Lease<'_>) -> io::Result<DirListing<WindowsHandle>> {
        ensure_same_directory(dir)?;

        let at_drive_root = dir.is_drive_root();
        let mut listing = DirListing::default();

        for entry in fs::read_dir(&dir.path)? {
            let Ok(entry) = entry else {
                listing.errors += 1;
                continue;
            };

            // The listing's own data: no further call.
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => {
                    listing.errors += 1;
                    continue;
                }
            };

            let name = entry.file_name();
            let kind = kind_of(&metadata);

            let left_out = at_drive_root
                && match kind {
                    EntryKind::Dir => dir.rules.excludes_folder(&name),
                    EntryKind::File => {
                        if dir.rules.excludes_file(&name) {
                            continue;
                        }
                        false
                    }
                    _ => false,
                };

            // Room is reserved before the entry is materialised: a refusal ends the listing, because
            // the scan is over (it has hit a limit).
            if !lease.admit(name.as_encoded_bytes().len(), kind == EntryKind::Dir) {
                break;
            }

            listing.entries.push(if left_out {
                skipped_folder(name, &metadata)
            } else {
                read_entry(dir, name, &metadata, kind)
            });
        }

        Ok(listing)
    }
}

/// The entry for a folder the rules leave out: a mount root that is not entered.
fn skipped_folder(name: OsString, metadata: &Metadata) -> RawEntry<WindowsHandle> {
    let mut meta = describe(metadata, EntryKind::Dir, None);
    meta.is_mount_root = true;

    RawEntry {
        name: name.as_encoded_bytes().to_vec(),
        meta,
        child: None,
        skipped: Some(SkipReason::System),
    }
}

/// The entry for `name` in `dir`, whose listing data is `metadata`.
fn read_entry(
    dir: &WindowsHandle,
    name: OsString,
    metadata: &Metadata,
    kind: EntryKind,
) -> RawEntry<WindowsHandle> {
    let path = dir.path.join(&name);

    // A link is described by its listing alone: nothing is gained by looking at it.
    let identity = match kind {
        EntryKind::Dir | EntryKind::File => identity_of_path(&path).ok(),
        _ => None,
    };
    let mut meta = describe(metadata, kind, identity.as_ref());

    let mut child = None;
    let mut skipped = None;
    if kind == EntryKind::Dir {
        meta.is_mount_root = match (identity.map(|identity| identity.volume_serial), dir.volume()) {
            (Some(own), Some(parent)) => own != parent,
            _ => false,
        };

        if meta.is_mount_root && dir.one_file_system {
            // Another volume than its parent is all this reader can tell: another disk.
            skipped = Some(SkipReason::OtherDisk);
        } else {
            child = Some(WindowsHandle {
                path,
                identity: identity.map(|identity| (identity.volume_serial, identity.file_index)),
                is_root: false,
                one_file_system: dir.one_file_system,
                rules: dir.rules,
            });
        }
    }

    RawEntry {
        name: name.as_encoded_bytes().to_vec(),
        meta,
        child,
        skipped,
    }
}

/// What the listing's `metadata` says about an entry: a link is a reparse point that is a folder or a name
/// for another place (a symbolic link, a junction). See the module documentation.
fn kind_of(metadata: &Metadata) -> EntryKind {
    let attributes = metadata.file_attributes();
    let is_dir = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    let is_reparse_point = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;

    if is_reparse_point && (is_dir || metadata.file_type().is_symlink()) {
        EntryKind::Symlink
    } else if is_dir {
        EntryKind::Dir
    } else {
        EntryKind::File
    }
}

/// The ports' description of an entry: the listing's time and length, the identity from the handle (when
/// there was one) and the size on disk.
fn describe(metadata: &Metadata, kind: EntryKind, identity: Option<&FileIdentity>) -> EntryMeta {
    let mut meta = entry_meta(metadata);
    meta.kind = kind;
    meta.allocated = match kind {
        EntryKind::File => allocated_size(metadata.len(), metadata.file_attributes()),
        _ => 0,
    };

    if let Some(identity) = identity {
        meta.dev = identity.volume_serial;
        meta.ino = identity.file_index;
        meta.nlink = identity.links.max(1);
    }

    meta
}

/// Bytes a file of `len` bytes with `attributes` takes on this disk.
fn allocated_size(len: u64, attributes: u32) -> u64 {
    if attributes & NOT_ON_THIS_DISK != 0 {
        return 0;
    }

    round_up(len, ASSUMED_CLUSTER_BYTES)
}

/// The identity of what `path` leads to: a link at the last name is followed (the scan root may be one).
pub(crate) fn identity_following(path: &Path) -> io::Result<FileIdentity> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;

    identity_of(&file)
}

/// Fails unless `dir` is, right now, the folder its parent's listing saw: a real folder, not a link, with
/// the same identity. The scan root is followed, so only its identity is compared.
fn ensure_same_directory(dir: &WindowsHandle) -> io::Result<()> {
    let replaced = || io::Error::other("the directory was replaced while the scan was running");

    let now = if dir.is_root {
        identity_following(&dir.path)?
    } else {
        identity_of_path(&dir.path)?
    };

    if !now.is_dir() || (!dir.is_root && now.is_reparse_point()) {
        return Err(replaced());
    }
    if let Some((volume, index)) = dir.identity
        && (now.volume_serial, now.file_index) != (volume, index)
    {
        return Err(replaced());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_takes_whole_clusters() {
        assert_eq!(allocated_size(0, 0), 0);
        assert_eq!(allocated_size(1, 0), 4096);
        assert_eq!(allocated_size(4097, 0), 8192);
    }

    #[test]
    fn a_file_kept_in_the_cloud_takes_no_room_here() {
        for attribute in [
            FILE_ATTRIBUTE_OFFLINE,
            FILE_ATTRIBUTE_RECALL_ON_OPEN,
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        ] {
            assert_eq!(allocated_size(10_000_000, attribute), 0);
        }
    }

    #[test]
    fn the_drive_root_is_the_only_folder_the_exclusions_apply_to() {
        let handle = |path: &str| WindowsHandle {
            path: PathBuf::from(path),
            identity: None,
            is_root: false,
            one_file_system: false,
            rules: ExclusionRules::DEFAULT,
        };

        assert!(handle("C:\\").is_drive_root());
        assert!(!handle("C:\\Users").is_drive_root());
        assert!(!handle("C:\\Users\\a").is_drive_root());
    }
}
