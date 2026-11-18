//! The scan's directory reader on macOS: `openat` relative to the parent's descriptor, `Dir` (`fdopendir`)
//! for the listing and `fstatat` on every entry. No `unsafe`: rustix wraps the calls safely.
//!
//! What each entry is made of: the kind from `st_mode` (a symbolic link is a `Symlink` and is never
//! followed), the allocated size `st_blocks * 512`, the length `st_size`, the modification time, and
//! `st_dev` and `st_ino` (the identity that counts a hard link once) with `st_nlink`. A file that is
//! kept in the cloud (dataless) or compressed by the filesystem reports no blocks and counts for 0.
//!
//! Which mounts are entered, and which folders are left out, is [`super::exclusions`]: the devices of the
//! scan's own disk (with `/System/Volumes/Data` when the scan starts at `/`), no virtual or network
//! filesystem, nothing in `/System/Volumes`, and none of the indexes at the root of a volume. A mount that
//! is not entered is reported without a child handle, so the walker shows it as a `mount` node. A device
//! that differs from the folder's own, but is part of the same disk (a firmlink), is an ordinary folder.
//!
//! A directory is opened only if it is still the one that was listed: the descriptor is opened with
//! `O_NOFOLLOW | O_DIRECTORY` below its parent, and `fstat` of it (device and inode) is compared with the
//! listing. That catches a plain swap, a symbolic link put in its place, and a mount over the name (the
//! mounted filesystem has another device). A descriptor of a directory stays open while it has pending
//! children, within the process-wide budget of [`crate::fd_budget`]; a directory that cannot claim a slot
//! keeps none, and its children are reopened by walking their names down from the scan root, each step
//! checked in the same way.
//!
//! The system's privacy protection (TCC) refuses some folders of a user (`EPERM`, `EACCES`) unless the
//! app has the permission. That is a folder that cannot be read, like any other: it is counted as one
//! error and left empty, and the scan goes on. A refusal on a single entry is counted the same way.

pub use crate::fd_budget::descriptors_held;

use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use platter_core::{
    DirListing, DirSource, EntryKind, EntryMeta, Lease, RawEntry, RootInfo, SkipReason, SourceOptions,
};
use rustix::fs::{AtFlags, CWD, Dir, FileType, Mode, OFlags, Stat, fstat, getpath, openat, statat};
use rustix::io::Errno;

use super::exclusions::{Scope, data_volume_device, mount_at};
use crate::fd_budget::{self as budget, Slot};

/// How often a directory open that hits the descriptor limit is retried (with growing sleeps).
const EXHAUSTED_RETRIES: u32 = 40;

/// The flags of every directory open: never a link, always a directory.
const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// The directory reader on macOS.
#[derive(Debug, Clone, Copy, Default)]
pub struct MacDirSource {
    descriptor_budget: Option<usize>,
}

impl MacDirSource {
    /// Reader with the default exclusions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Caps the descriptors that scans of this process may keep open as parents of pending children
    /// (default: half the soft `RLIMIT_NOFILE`). The count is shared by every scan of the process, see
    /// [`descriptors_held`]; this only changes the cap this source's scans respect.
    pub fn with_descriptor_budget(mut self, limit: usize) -> Self {
        self.descriptor_budget = Some(limit);
        self
    }
}

/// State shared by every handle of one scan.
#[derive(Debug)]
struct Shared {
    /// The scan root, kept open for the whole scan: the fallback when a directory has to be reopened
    /// without a parent descriptor.
    root: OwnedFd,
    root_dev: u64,
    /// Where the scan root is, as the kernel names it. `None` when it cannot be told: no mount is
    /// entered then, and nothing is left out by its path.
    root_path: Option<PathBuf>,
    scope: Scope,
    /// The cap on directory descriptors kept open as parents (the count is process-wide, see `budget`).
    budget: usize,
}

/// What the listing saw of a directory: the one that has to be behind its name when it is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
}

/// The names from the scan root down to a directory, as a linked list shared with the siblings'
/// ancestors, each with the identity it was listed with.
#[derive(Debug)]
struct Chain {
    name: CString,
    identity: Identity,
    parent: Option<Arc<Chain>>,
}

/// A directory descriptor shared by the handles of that directory's subdirectories, so they can be
/// opened with a single `openat`. It owns a slot of the process-wide descriptor budget while `slot` is
/// set; without one it is closed as soon as the directory has been listed.
#[derive(Debug)]
struct Held {
    fd: OwnedFd,
    slot: Option<Slot>,
}

impl Held {
    fn new(fd: OwnedFd, budget: usize) -> Self {
        Self {
            fd,
            slot: Slot::claim(budget),
        }
    }
}

/// A directory to read: the root, or a subdirectory found while reading its parent. Children are opened
/// lazily inside `read_dir`, so a directory with a hundred thousand subdirectories does not hold a
/// hundred thousand descriptors.
#[derive(Debug)]
pub struct MacHandle {
    location: Location,
    shared: Arc<Shared>,
    /// The directory is the root of a volume (a mount that is entered, or the scan root when it is one).
    volume_root: bool,
}

#[derive(Debug)]
enum Location {
    /// The scan root; its descriptor is `Shared::root`.
    Root,
    Child {
        /// The names down to this directory; its own link carries the identity to check on opening.
        chain: Arc<Chain>,
        /// The parent's descriptor, when the parent was within budget.
        parent: Option<Arc<Held>>,
    },
}

impl DirSource for MacDirSource {
    type Handle = MacHandle;

    fn open_root(&self, path: &Path, options: SourceOptions) -> io::Result<RootInfo<MacHandle>> {
        // The root itself may be a symlink (`/tmp`); resolve it, but nothing below it. Callers that
        // resolved the path beforehand can pin what they saw with `ScanConfig::root_identity`.
        let fd = openat(
            CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let meta = describe(&fstat(&fd)?);

        let root_path = getpath(&fd)
            .ok()
            .map(|path| PathBuf::from(OsStr::from_bytes(path.to_bytes())));
        let scope = Scope::new(
            root_path.as_deref(),
            meta.dev,
            data_volume_device(),
            options.one_file_system,
        );
        let root_is_volume_root = root_path.as_deref().is_some_and(|path| mount_at(path).is_some());

        let handle = MacHandle {
            location: Location::Root,
            shared: Arc::new(Shared {
                root: fd,
                root_dev: meta.dev,
                root_path,
                scope,
                budget: self.descriptor_budget.unwrap_or_else(budget::default_limit),
            }),
            volume_root: root_is_volume_root,
        };
        Ok(RootInfo {
            handle,
            meta,
            warnings: Vec::new(),
        })
    }

    fn read_dir(&self, dir: &MacHandle, lease: &mut Lease<'_>) -> io::Result<DirListing<MacHandle>> {
        match &dir.location {
            Location::Root => list(dir, &dir.shared.root, None, None, dir.shared.root_dev, lease),
            Location::Child { chain, parent } => {
                let parent_fd = parent.as_deref().map(|held| &held.fd);
                let (fd, dev) = open_child(&dir.shared.root, chain, parent_fd)?;
                let held = Arc::new(Held::new(fd, dir.shared.budget));

                // Children share the descriptor only while it is within budget; otherwise it is
                // closed when this function returns.
                let keep = held.slot.is_some().then(|| held.clone());
                list(dir, &held.fd, Some(chain), keep, dev, lease)
            }
        }
    }
}

/// Lists the open directory `fd`, whose device is `dev`, stopping as soon as `lease` refuses room for
/// another entry. `chain` is the directory's own chain (`None` for the root) and `keep` the descriptor
/// handle its subdirectories may open under.
fn list(
    dir: &MacHandle,
    fd: &OwnedFd,
    chain: Option<&Arc<Chain>>,
    keep: Option<Arc<Held>>,
    dev: u64,
    lease: &mut Lease<'_>,
) -> io::Result<DirListing<MacHandle>> {
    let mut listing = DirListing::default();
    let entries = retry_exhausted(|| Ok(Dir::read_from(fd)?))?;

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();

        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        match statat(fd, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => {
                let meta = describe(&stat);

                // Room is reserved before the entry is materialised: a refusal ends the listing,
                // because the scan is over (it has hit a limit).
                if !lease.admit(name.to_bytes().len(), meta.kind == EntryKind::Dir) {
                    break;
                }
                listing
                    .entries
                    .push(make_entry(dir, chain, &keep, dev, name, meta));
            }
            Err(Errno::NOENT) => {}
            Err(_) => listing.errors += 1,
        }
    }

    Ok(listing)
}

/// The entry `name` of the directory `chain` (on device `parent_dev`) whose `fstatat` is `meta`.
fn make_entry(
    dir: &MacHandle,
    chain: Option<&Arc<Chain>>,
    keep: &Option<Arc<Held>>,
    parent_dev: u64,
    name: &CStr,
    mut meta: EntryMeta,
) -> RawEntry<MacHandle> {
    let mut entry = RawEntry {
        name: name.to_bytes().to_vec(),
        meta,
        child: None,
        skipped: None,
    };
    if meta.kind != EntryKind::Dir {
        return entry;
    }

    match classify(dir, chain, name, &meta, parent_dev) {
        Fate::Enter { mount_root } => {
            meta.is_mount_root = mount_root;
            entry.meta = meta;
            entry.child = Some(MacHandle {
                location: Location::Child {
                    chain: Arc::new(Chain {
                        name: name.to_owned(),
                        identity: Identity {
                            dev: meta.dev,
                            ino: meta.ino,
                        },
                        parent: chain.cloned(),
                    }),
                    parent: keep.clone(),
                },
                shared: dir.shared.clone(),
                volume_root: mount_root,
            });
        }
        Fate::Skip(reason) => {
            meta.is_mount_root = true;
            entry.meta = meta;
            entry.skipped = Some(reason);
        }
    }

    entry
}

/// What becomes of a directory the listing found.
enum Fate {
    /// It is read. `mount_root`: it starts another mount that is entered (an external disk), as opposed
    /// to a folder of the same disk.
    Enter { mount_root: bool },
    /// It is listed as a `mount` node and never entered, for this reason.
    Skip(SkipReason),
}

fn classify(
    dir: &MacHandle,
    chain: Option<&Arc<Chain>>,
    name: &CStr,
    meta: &EntryMeta,
    parent_dev: u64,
) -> Fate {
    let shared = &dir.shared;
    let scope = &shared.scope;

    if dir.volume_root && scope.leaves_out_volume_root_folder(name.to_bytes()) {
        return Fate::Skip(SkipReason::System);
    }

    // Only a folder called `Volumes` can be `/System/Volumes`: the path is built for no other name.
    if name.to_bytes() == b"Volumes" {
        let path = path_of(shared.root_path.as_deref(), chain, name);
        if path.is_some_and(|path| scope.leaves_out_folder(&path)) {
            return Fate::Skip(SkipReason::System);
        }
    }

    // The same device as the folder around it, or one that belongs to the same disk (a firmlink to the
    // Data volume): an ordinary folder.
    if meta.dev == parent_dev || scope.joins(meta.dev) {
        return Fate::Enter { mount_root: false };
    }

    let path = path_of(shared.root_path.as_deref(), chain, name);
    match scope.mount_verdict(meta.dev, path.as_deref()) {
        Ok(()) => Fate::Enter { mount_root: true },
        Err(reason) => Fate::Skip(reason),
    }
}

/// The path of `name` inside the directory `chain`, below the scan root at `root_path`.
fn path_of(root_path: Option<&Path>, chain: Option<&Arc<Chain>>, name: &CStr) -> Option<PathBuf> {
    let mut path = root_path?.to_path_buf();

    let mut names = Vec::new();
    let mut link = chain.map(|chain| &**chain);
    while let Some(current) = link {
        names.push(current.name.as_bytes());
        link = current.parent.as_deref();
    }

    for component in names.into_iter().rev() {
        path.push(OsStr::from_bytes(component));
    }
    path.push(OsStr::from_bytes(name.to_bytes()));
    Some(path)
}

/// What `fstat` says about an entry, in the ports' terms.
fn describe(stat: &Stat) -> EntryMeta {
    let kind = match FileType::from_raw_mode(stat.st_mode) {
        FileType::Directory => EntryKind::Dir,
        FileType::RegularFile => EntryKind::File,
        FileType::Symlink => EntryKind::Symlink,
        _ => EntryKind::Other,
    };

    EntryMeta {
        kind,
        allocated: u64::try_from(stat.st_blocks).unwrap_or(0).saturating_mul(512),
        apparent: u64::try_from(stat.st_size).unwrap_or(0),
        mtime: stat.st_mtime,
        dev: stat.st_dev as u64,
        ino: stat.st_ino,
        nlink: u64::from(stat.st_nlink),
        is_mount_root: false,
    }
}

/// Opens the directory `chain` names and checks it against the listing, returning its device.
///
/// With `parent` (the descriptor of the directory it was listed in) that is one `openat`; without, the
/// chain is walked down from `root`, one component at a time. Retries while the process is out of file
/// descriptors.
fn open_child(root: &OwnedFd, chain: &Chain, parent: Option<&OwnedFd>) -> io::Result<(OwnedFd, u64)> {
    retry_exhausted(|| match parent {
        Some(parent) => open_checked(parent, &chain.name, chain.identity),
        None => open_from_root(root, chain),
    })
}

/// Opens the directory `chain` names by descending from `root` one component at a time, checking each.
fn open_from_root(root: &OwnedFd, chain: &Chain) -> io::Result<(OwnedFd, u64)> {
    let mut links = Vec::new();
    let mut link = Some(chain);
    while let Some(current) = link {
        links.push(current);
        link = current.parent.as_deref();
    }

    let mut current: Option<(OwnedFd, u64)> = None;
    for link in links.into_iter().rev() {
        let parent = current.as_ref().map_or(root.as_fd(), |(fd, _)| fd.as_fd());

        let opened = open_checked(parent, &link.name, link.identity)?;
        current = Some(opened);
    }

    current.ok_or_else(|| io::Error::other("a directory chain names at least one directory"))
}

/// One `openat` of `name` below `parent`, then the check against what the listing saw.
fn open_checked(parent: impl AsFd, name: &CStr, listed: Identity) -> io::Result<(OwnedFd, u64)> {
    let fd = openat(parent, name, DIR_FLAGS, Mode::empty())?;

    let opened = fstat(&fd)?;
    let now = Identity {
        dev: opened.st_dev as u64,
        ino: opened.st_ino,
    };
    if now != listed {
        return Err(io::Error::other(
            "the directory was replaced or mounted over while the scan was running",
        ));
    }

    Ok((fd, now.dev))
}

fn is_fd_exhausted(error: &io::Error) -> bool {
    error
        .raw_os_error()
        .is_some_and(|code| code == Errno::MFILE.raw_os_error() || code == Errno::NFILE.raw_os_error())
}

/// Runs `open`, sleeping and retrying while the process is out of file descriptors; other workers
/// close theirs as they finish, so this normally succeeds within a few milliseconds.
fn retry_exhausted<T>(mut open: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut delay = Duration::from_millis(1);

    for _ in 0..EXHAUSTED_RETRIES {
        match open() {
            Err(error) if is_fd_exhausted(&error) => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(50));
            }
            result => return result,
        }
    }
    open()
}
