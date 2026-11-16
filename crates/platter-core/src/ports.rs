//! Ports: the only places where the domain meets the operating system.
//!
//! Everything else (walker, tree, aggregation, queries) lives in this crate and is tested against the
//! in-memory fake in `crate::testing`.

use std::io;
use std::path::Path;

use crate::budget::Lease;
use crate::model::{EntryKind, ScanWarning, SkipReason, Volume};

/// Facts about one filesystem entry, never following symlinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryMeta {
    pub kind: EntryKind,
    /// Bytes allocated on disk (blocks * block size).
    pub allocated: u64,
    /// File length in bytes. The scanner ignores it for directories.
    pub apparent: u64,
    /// Modification time, unix seconds.
    pub mtime: i64,
    /// Device and inode: together the identity used to deduplicate hardlinks. 0/0 when unknown.
    pub dev: u64,
    pub ino: u64,
    /// Hardlink count; 1 when unknown.
    pub nlink: u64,
    /// The entry is the root of a mounted filesystem.
    pub is_mount_root: bool,
}

/// One directory entry as read from a [`DirSource`].
#[derive(Debug)]
pub struct RawEntry<H> {
    /// File name as raw bytes (no path separators).
    pub name: Vec<u8>,
    pub meta: EntryMeta,
    /// Handle to read the entry's children. Only meaningful for directories; a source sets `None` for a
    /// directory it wants left alone: a pseudo filesystem, or, under
    /// [`SourceOptions::one_file_system`], a mount that is not part of the scan root's filesystem. The
    /// walker then keeps the directory as an empty node, or as a `mount` node when `meta.is_mount_root`
    /// is set. The walker never second-guesses this: whether a mount root is entered is decided here.
    pub child: Option<H>,
    /// Why the source withheld the handle of a mount root (`child` is `None` and `meta.is_mount_root`):
    /// the reader knows, the walker only passes it on to the node. `None` when it cannot tell, which the
    /// tree records as [`SkipReason::Unknown`]. Ignored for every other entry.
    pub skipped: Option<SkipReason>,
}

/// Result of reading one directory.
#[derive(Debug)]
pub struct DirListing<H> {
    pub entries: Vec<RawEntry<H>>,
    /// Entries that could not be examined (they are absent from `entries`). Entries that vanished
    /// between listing and stat are not errors.
    pub errors: u32,
}

impl<H> Default for DirListing<H> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            errors: 0,
        }
    }
}

/// The scan root: its handle and its own metadata.
#[derive(Debug)]
pub struct RootInfo<H> {
    pub handle: H,
    pub meta: EntryMeta,
    /// Things the source could not do that make the scan less complete (see [`ScanWarning`]). The scan
    /// counts each as one error and keeps them on the tree. Empty for a source with nothing to report.
    pub warnings: Vec<ScanWarning>,
}

/// What a scan asks of a [`DirSource`] when it opens the root.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceOptions {
    /// Stay on the filesystem of the scan root: a mount root (`meta.is_mount_root`) below the root is
    /// listed without a child handle unless the source judges it part of the same filesystem. "Same
    /// filesystem" is the source's call because only it can tell: on Linux several mounts (btrfs
    /// subvolumes) of one disk count as one, while the generic reader can only compare `st_dev`.
    pub one_file_system: bool,
}

/// Handle-based directory reading.
///
/// Reading is split into `open_root` and `read_dir(&handle)` so an implementation can resolve children
/// relative to an already open directory (Linux: `openat` + `statx` on a directory fd).
///
/// The handle is an associated type rather than a boxed `dyn Any`: the walker is generic over the
/// source, handles stay unboxed and typed, and a source cannot be handed a handle from another
/// implementation. The cost is that a source is picked at compile time (`Platform::current()` does it).
///
/// Contract:
/// - never follow symlinks below the root (the root path itself may be a symlink and is resolved);
/// - `read_dir` omits `.` and `..`;
/// - honour [`SourceOptions::one_file_system`] by withholding the child handle of every mount root that
///   is not on the root's filesystem, and decide that once, at `open_root`, so the walk is deterministic;
/// - handles must be cheap to hold: a source should open the directory lazily inside `read_dir` instead
///   of keeping a file descriptor open per pending child;
/// - `read_dir` gets a [`Lease`]: before it returns an entry it calls [`Lease::admit`] with the length
///   of the entry's name, and when that says no it stops listing at once and returns what it has (the
///   caller discards the listing: the scan has hit a limit). That reserves the scan's entry and memory
///   budget while entries are read, so many directories read at the same time, or one huge or endless
///   directory, cannot hold more than the budget allows. A source that never asks is correct, only less
///   frugal: the finished listing is checked against the budget afterwards;
/// - implementations are shared between worker threads.
pub trait DirSource: Send + Sync {
    type Handle: Send + 'static;

    fn open_root(&self, path: &Path, options: SourceOptions) -> io::Result<RootInfo<Self::Handle>>;

    fn read_dir(&self, dir: &Self::Handle, lease: &mut Lease<'_>) -> io::Result<DirListing<Self::Handle>>;
}

/// Lists the volumes the machine has mounted.
pub trait VolumeProvider: Send + Sync {
    fn list(&self) -> Vec<Volume>;
}
