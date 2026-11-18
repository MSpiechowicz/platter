//! Fast Linux `DirSource`: `openat` + `getdents` (via `rustix::fs::RawDir`) + `statx` relative to the
//! directory descriptor. No `unsafe`: rustix wraps the syscalls safely.
//!
//! Mount boundaries come from `STATX_ATTR_MOUNT_ROOT` (not `st_dev`, which differs per btrfs
//! subvolume). Mounts matching the [`ExclusionRules`] (pseudo and network filesystems) are reported
//! without a child handle, so they are never entered. Allocated size is `st_blocks * 512`.
//!
//! The reader enters the mounts that belong to the same disk as the scan root (subvolumes of one btrfs
//! filesystem) and never counts one directory twice (bind mounts, a subvolume mounted again): see
//! [`same_fs`] for the rules, which apply with and without `one_file_system`. Every mount root that is not
//! entered is reported without a child handle, so the walker shows it as a `mount` node.
//!
//! Safety against hostile or changing mounts: `statx` never triggers automounts; a mount root whose id
//! was not in the mount table read at scan start, or that is not at the path the table gives for it, is
//! not entered (see [`MountSnapshot`]); and a directory is opened only if it is still the one that was
//! listed (see the `open` module: `openat2` with `RESOLVE_NO_XDEV`, then a `statx` of the descriptor
//! compared with the listing).
//!
//! The mount table is read as bytes, line by line, so one odd mount point cannot hide the others. If it
//! cannot be read at all, no mount is entered and the scan carries a
//! [`ScanWarning::MountTableUnavailable`].
//!
//! Kernels before 5.8 report neither `STATX_ATTR_MOUNT_ROOT` nor `STATX_MNT_ID`. The reader notices the
//! missing attribute and falls back to comparing `st_dev` with the directory the entry was found in: a
//! different device is a boundary, entered only if a mount of the table that may be entered has that
//! very device at that very path. On such kernels a btrfs subvolume (a device of its own, not a mount)
//! counts as a boundary and is left out.

pub mod exclusions;
pub mod labels;
pub mod mountinfo;
pub mod same_fs;
mod snapshot;
mod stat;

mod open;
pub mod pinned;
pub mod probe;
pub mod remove;
pub(crate) mod runtime;
#[cfg(test)]
mod tests;
pub mod tools;
mod trash;

pub use crate::fd_budget::descriptors_held;
pub use exclusions::ExclusionRules;
pub use snapshot::MountSnapshot;

use std::cell::RefCell;
use std::ffi::{CStr, OsStr};
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use platter_core::{
    DirListing, DirSource, EntryKind, Lease, RawEntry, RootInfo, ScanWarning, SkipReason, SourceOptions,
};
use rustix::fs::{CWD, Mode, OFlags, RawDir, openat};
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

use crate::fd_budget::{self as budget, Slot};
use open::{Chain, Identity, open_child};
use snapshot::device_text;
use stat::{Stat, is_boundary, stat_at, stat_fd};

const MOUNTINFO: &str = "/proc/self/mountinfo";
const DIR_BUFFER_BYTES: usize = 64 * 1024;

thread_local! {
    /// Reused `getdents` buffer, one per worker thread.
    static DIR_BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(DIR_BUFFER_BYTES));
}

/// Raises the soft `RLIMIT_NOFILE` to the hard limit, so a deep scan on many workers does not run out
/// of descriptors at the usual default of 1024. Returns the soft limit now in force (`None` means
/// unlimited), or `Err` if it could not be read or raised.
///
/// When the hard limit is "unlimited" the kernel still caps the soft limit at `fs.nr_open`, so a
/// refused request falls back to 1 048 576 (or the current hard limit, whichever is smaller).
pub fn raise_fd_limit() -> io::Result<Option<u64>> {
    let Rlimit { current, maximum } = getrlimit(Resource::Nofile);
    if current == maximum {
        return Ok(current);
    }

    let wanted = Rlimit {
        current: maximum,
        maximum,
    };
    if setrlimit(Resource::Nofile, wanted).is_ok() {
        return Ok(maximum);
    }

    let capped = Some(maximum.map_or(1 << 20, |hard| hard.min(1 << 20)));
    setrlimit(
        Resource::Nofile,
        Rlimit {
            current: capped,
            maximum,
        },
    )?;
    Ok(capped)
}

#[derive(Debug, Clone)]
pub struct LinuxDirSource {
    rules: ExclusionRules,
    mountinfo: PathBuf,
    descriptor_budget: Option<usize>,
}

impl Default for LinuxDirSource {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxDirSource {
    /// Reader with the default exclusions.
    pub fn new() -> Self {
        Self::with_rules(ExclusionRules::DEFAULT)
    }

    pub fn with_rules(rules: ExclusionRules) -> Self {
        Self {
            rules,
            mountinfo: PathBuf::from(MOUNTINFO),
            descriptor_budget: None,
        }
    }

    /// Reads the mount table from another file (for tests).
    pub fn with_mountinfo_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.mountinfo = path.into();
        self
    }

    /// Caps the descriptors that scans of this process may keep open as parents of pending children
    /// (default: half the soft `RLIMIT_NOFILE`). The count is shared by every scan of the process, see
    /// [`descriptors_held`]; this only changes the cap this source's scans respect.
    pub fn with_descriptor_budget(mut self, limit: usize) -> Self {
        self.descriptor_budget = Some(limit);
        self
    }

    /// The mount table as it is now, read once at the start of every scan, and what it says about the
    /// mounts below the scan root (open as `root`, living on mount `root_mount`, at `root_path`). The
    /// warning list is not empty when the table could not be read.
    fn mount_snapshot(
        &self,
        root_mount: Option<u64>,
        root_path: Option<&Path>,
        one_file_system: bool,
    ) -> (MountSnapshot, Vec<ScanWarning>) {
        let table = std::fs::read(&self.mountinfo).unwrap_or_default();
        let mounts = mountinfo::parse(&table);

        let warnings = if mounts.is_empty() {
            vec![ScanWarning::MountTableUnavailable]
        } else {
            Vec::new()
        };

        let enterable = match root_mount.zip(root_path) {
            Some((mount_id, path)) => {
                same_fs::enterable_mounts(&mounts, &self.rules, mount_id, path, one_file_system)
            }
            None => one_file_system.then(Default::default),
        };
        (MountSnapshot::new(&self.rules, &mounts, enterable), warnings)
    }
}

/// State shared by every handle of one scan.
#[derive(Debug)]
struct Shared {
    mounts: MountSnapshot,
    /// The scan root, kept open for the whole scan: the fallback when a directory has to be reopened
    /// without a parent descriptor.
    root: OwnedFd,
    /// `st_dev` of the scan root, to spot mount boundaries below it on kernels without the attribute.
    root_dev: u64,
    /// Where the scan root is, as the process sees it. A mount root met by the walk is compared with the
    /// table at this path. `None` when it cannot be told: no mount is entered then.
    root_path: Option<PathBuf>,
    /// The cap on directory descriptors kept open as parents (the count is process-wide, see `budget`).
    budget: usize,
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

/// A directory to read: the root, or a subdirectory found while reading its parent.
///
/// Children are opened lazily inside `read_dir`, so a directory with a hundred thousand subdirectories
/// does not hold a hundred thousand descriptors. Opening is cheap and relative (`openat` under the
/// parent's descriptor) while the process stays within its descriptor budget: a parent descriptor is
/// kept open until its last pending child has been opened, which on deep trees grows with depth times
/// the number of workers. Once the budget (half the soft `RLIMIT_NOFILE`, counted over all scans of the
/// process together) is used up, further directories keep no descriptor for their children; those are
/// reopened later by walking their name chain down from the root descriptor, one `openat` per
/// component, never following a symlink. That costs a few syscalls per directory but holds at most two
/// descriptors, so a scan can neither fail nor lose directories because of the limit, whatever the
/// depth. Descriptor exhaustion caused by the rest of the process is retried with a short backoff
/// before it is reported. Either way every open is checked against what the listing saw (see `open`).
#[derive(Debug)]
pub struct LinuxHandle {
    location: Location,
    shared: Arc<Shared>,
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

impl DirSource for LinuxDirSource {
    type Handle = LinuxHandle;

    fn open_root(&self, path: &Path, options: SourceOptions) -> io::Result<RootInfo<LinuxHandle>> {
        // The root itself may be a symlink (`/home -> /var/home`); resolve it, but nothing below it.
        // Callers that resolved the path beforehand can pin what they saw with
        // `ScanConfig::root_identity`; the check runs on this very descriptor.
        let fd = openat(
            CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let Stat { meta, mount_id, .. } = stat_fd(&fd)?;

        let root_path = same_fs::path_of_fd(&fd);
        let (mounts, warnings) = self.mount_snapshot(mount_id, root_path.as_deref(), options.one_file_system);

        let handle = LinuxHandle {
            location: Location::Root,
            shared: Arc::new(Shared {
                mounts,
                root: fd,
                root_dev: meta.dev,
                root_path,
                budget: self.descriptor_budget.unwrap_or_else(budget::default_limit),
            }),
        };
        Ok(RootInfo {
            handle,
            meta,
            warnings,
        })
    }

    fn read_dir(&self, dir: &LinuxHandle, lease: &mut Lease<'_>) -> io::Result<DirListing<LinuxHandle>> {
        match &dir.location {
            Location::Root => list(dir, &dir.shared.root, None, None, dir.shared.root_dev, lease),
            Location::Child { chain, parent } => {
                let parent_fd = parent.as_deref().map(|held| &held.fd);
                let (fd, opened) = open_child(&dir.shared.root, chain, parent_fd)?;
                let held = Arc::new(Held::new(fd, dir.shared.budget));

                // Children share the descriptor only while it is within budget; otherwise it is
                // closed when this function returns.
                let keep = held.slot.is_some().then(|| held.clone());
                list(dir, &held.fd, Some(chain), keep, opened.meta.dev, lease)
            }
        }
    }
}

/// Lists the open directory `fd`, whose device is `dev`, stopping as soon as `lease` refuses room for
/// another entry. `chain` is the directory's own chain (`None` for the root) and `keep` the descriptor
/// handle its subdirectories may open under.
fn list(
    dir: &LinuxHandle,
    fd: &OwnedFd,
    chain: Option<&Arc<Chain>>,
    keep: Option<Arc<Held>>,
    dev: u64,
    lease: &mut Lease<'_>,
) -> io::Result<DirListing<LinuxHandle>> {
    let mut listing = DirListing::default();

    DIR_BUFFER.with_borrow_mut(|buffer| -> io::Result<()> {
        let mut raw = RawDir::new(fd, buffer.spare_capacity_mut());

        while let Some(entry) = raw.next() {
            let entry = entry?;
            let name = entry.file_name();

            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            match stat_at(fd, name) {
                Ok(stat) => {
                    // Room is reserved before the entry is materialised: a refusal ends the listing,
                    // because the scan is over (it has hit a limit).
                    if !lease.admit(name.to_bytes().len(), stat.meta.kind == EntryKind::Dir) {
                        break;
                    }
                    listing
                        .entries
                        .push(make_entry(dir, chain, &keep, dev, name, stat));
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(_) => listing.errors += 1,
            }
        }
        Ok(())
    })?;

    Ok(listing)
}

fn make_entry(
    dir: &LinuxHandle,
    chain: Option<&Arc<Chain>>,
    keep: &Option<Arc<Held>>,
    parent_dev: u64,
    name: &CStr,
    stat: Stat,
) -> RawEntry<LinuxHandle> {
    let mut meta = stat.meta;
    let is_dir = meta.kind == EntryKind::Dir;

    // A kernel that reports the attribute settles whether this is a mount root; before 5.8 the device
    // is all there is.
    if is_dir {
        meta.is_mount_root = is_boundary(stat.reported_mount_root, meta.dev, parent_dev);
    }

    // A mount root is entered only if the mount table of the scan says so; otherwise it says why not.
    let verdict = if is_dir && meta.is_mount_root {
        mount_verdict(dir, chain, name, &stat, meta.dev)
    } else {
        Ok(())
    };
    let child = (is_dir && verdict.is_ok()).then(|| LinuxHandle {
        location: Location::Child {
            chain: Arc::new(Chain {
                name: name.to_owned(),
                identity: Identity::new(&stat, meta.is_mount_root),
                parent: chain.cloned(),
            }),
            parent: keep.clone(),
        },
        shared: dir.shared.clone(),
    });

    RawEntry {
        name: name.to_bytes().to_vec(),
        meta,
        child,
        skipped: verdict.err(),
    }
}

/// Whether the mount root `name` (found in the directory `chain`) may be entered, and if not, why.
fn mount_verdict(
    dir: &LinuxHandle,
    chain: Option<&Arc<Chain>>,
    name: &CStr,
    stat: &Stat,
    dev: u64,
) -> Result<(), SkipReason> {
    let shared = &dir.shared;
    let Some(path) = path_of(shared.root_path.as_deref(), chain, name) else {
        return Err(SkipReason::Unknown);
    };

    match stat.mount_id {
        Some(mount_id) => shared.mounts.verdict(mount_id, &path),
        None => shared.mounts.verdict_by_device(&device_text(dev), &path),
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
