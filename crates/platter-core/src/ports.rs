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

/// What an entry is right now, as the kernel reports it (never following a symlink).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbedEntry {
    pub meta: EntryMeta,
    /// Kernel mount id of the mount the entry lives on, when the system reports one.
    pub mount_id: Option<u64>,
}

/// Looks at a path as it is now, for code that is about to act on it (cleanup), not for scanning.
///
/// Contract:
/// - `path` is absolute and exact (the bytes of [`Tree::path_bytes`](crate::Tree::path_bytes));
/// - every component is looked at below the one before, starting from `/`, by descriptor where the
///   system allows it, and no symlink is ever followed: a component that is not a directory, other than
///   the last, fails with `NotADirectory`, a missing one with `NotFound`;
/// - `.`, `..` and empty components are refused (`InvalidInput`);
/// - the answer has one [`ProbedEntry`] per component below `/`, in order, the entry itself last, and
///   `meta.is_mount_root` is set on every component that starts a mount.
pub trait EntryProbe: Send + Sync {
    fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>>;

    /// Where `path` really is, every link on the way followed (the canonical path). Used only to learn
    /// where a protected place lives when it is reached through a link (`~/.config -> dotfiles/config`);
    /// never for a target. `Unsupported` where a source cannot tell.
    fn resolve(&self, _path: &[u8]) -> io::Result<Vec<u8>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// What a target has to be when it is acted on: what the plan saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedEntry {
    pub dev: u64,
    pub ino: u64,
    /// Compared when both the plan and the system report one.
    pub mount_id: Option<u64>,
    pub is_dir: bool,
    /// How many components the scan root has: no component below it may start a mount.
    pub root_depth: usize,
}

/// How much of a folder [`EntryRemover::remove`] removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveMode {
    /// The entry itself (a file, or a folder with everything inside).
    Whole,
    /// Everything inside the folder; the folder stays.
    Contents,
}

/// What removing one target did. Problems inside the target do not stop the rest of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Removed {
    /// Entries removed (files, links and folders).
    pub entries: u64,
    /// Folders inside that belong to another mount, left alone.
    pub skipped_mounts: u64,
    /// Entries that could not be removed: exact path and what went wrong. At most
    /// [`Removed::MAX_ERRORS`]; the rest are only counted in `more_errors`.
    pub errors: Vec<(Vec<u8>, String)>,
    pub more_errors: u64,
}

impl Removed {
    /// How many problems are kept with their path and message.
    pub const MAX_ERRORS: usize = 100;

    /// Records a problem, keeping the details of the first [`Self::MAX_ERRORS`].
    pub fn note(&mut self, path: Vec<u8>, what: String) {
        if self.errors.len() < Self::MAX_ERRORS {
            self.errors.push((path, what));
        } else {
            self.more_errors += 1;
        }
    }

    /// Adds what another removal did.
    pub fn absorb(&mut self, other: Removed) {
        self.entries += other.entries;
        self.skipped_mounts += other.skipped_mounts;
        self.more_errors += other.more_errors;
        for (path, what) in other.errors {
            self.note(path, what);
        }
    }

    /// Whether anything went wrong inside.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty() || self.more_errors > 0
    }
}

/// Acts on the disk for cleanup. Every method checks the target against [`ExpectedEntry`] right before
/// it acts and fails without touching anything when it differs: a link, another file in its place, a
/// mount on the way. Nothing is resolved by path string below `/`; no symlink is ever followed.
pub trait EntryRemover: Send + Sync {
    /// Deletes for good. Never crosses into another mount inside a folder.
    fn remove(&self, path: &[u8], expected: &ExpectedEntry, mode: RemoveMode) -> io::Result<Removed>;

    /// Moves the entry to the user's trash (freedesktop layout under `trash_dir`, which must be on the
    /// same filesystem), so it can be restored. Returns where it went.
    fn trash(&self, path: &[u8], expected: &ExpectedEntry, trash_dir: &Path) -> io::Result<Vec<u8>>;

    /// Puts an entry that [`trash`](Self::trash) moved to `trashed` back at `original`, when it is still
    /// the entry that was trashed (`expected`) and nothing has taken its old name since; then removes its
    /// `.trashinfo` record. Never overwrites.
    fn restore(&self, _trashed: &[u8], _expected: &ExpectedEntry, _original: &[u8]) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    /// Free bytes for unprivileged users on the filesystem holding `path`.
    fn free_bytes(&self, path: &Path) -> io::Result<u64>;

    /// Asks the filesystem holding `path` to finish pending work, so that [`free_bytes`](Self::free_bytes)
    /// sees space freed by deletes (btrfs frees it only when it commits, seconds later). Best effort.
    fn settle(&self, _path: &Path) -> io::Result<()> {
        Ok(())
    }
}

/// A program run by cleanup with fixed arguments (a package manager's own cache cleaning). Never built
/// from what a user or the UI typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolCommand<'a> {
    /// The program's file name; the runner finds it in the system's own program folders only.
    pub program: &'a str,
    pub args: &'a [&'a str],
    /// Run it as the administrator (the system asks for the password).
    pub elevated: bool,
}

/// Why a tool did not do its job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolFailure {
    /// The program is not installed (or not where the system keeps its programs).
    NotInstalled,
    /// The administrator password was not given, or was refused.
    NotAuthorized,
    /// The system has no way to ask for the administrator password (no `pkexec`).
    NoElevation,
    /// It ran and failed: its exit code and the last lines it wrote.
    Failed { code: Option<i32>, message: String },
    /// It could not be started.
    Io(String),
}

/// Runs cleanup tools. Elevated commands go through the system's authorization prompt.
pub trait ToolRunner: Send + Sync {
    fn run(&self, command: &ToolCommand<'_>) -> Result<(), ToolFailure>;
}

/// Lists the volumes the machine has mounted.
pub trait VolumeProvider: Send + Sync {
    fn list(&self) -> Vec<Volume>;
}
