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
    /// What the source knows about a mount root it gave a child handle for (see [`MountFacts`]). The
    /// default, for every other entry and for a source that cannot tell, says nothing is known.
    pub mount: MountFacts,
}

/// What a source says about a mount root that the scan enters, kept on the tree so that cleanup can tell
/// later which mounts the scan went into and whether it may act inside them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MountFacts {
    /// The kernel's id of the mount, when the system reports one.
    pub mount_id: Option<u64>,
    /// The mount is part of the scan root's own filesystem, and not a view of a folder from elsewhere: on
    /// Linux a subvolume of the same btrfs filesystem at its own place (`/home`), as against a bind
    /// mount of some folder, or another disk entered because the scan may leave the root's disk. Only a
    /// source that can tell says `true`; the default fails closed.
    pub own_filesystem: bool,
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
    /// The kernel's id of the mount the root lives on, when the system reports one. Kept on the tree with
    /// the root's device and inode, so cleanup can tell that the scan root was replaced since the scan.
    pub mount_id: Option<u64>,
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
    /// When the entry was created, when the filesystem records it (ext4, btrfs, xfs do).
    pub birth: Option<BirthTime>,
}

/// When an entry was created: seconds and nanoseconds since the epoch. Together with device and inode it
/// tells an entry from another one made later under the same name, even when the filesystem gave it the
/// same inode number again (ext4 does, soon after a delete).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BirthTime {
    pub secs: i64,
    pub nanos: u32,
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

    /// Where the symbolic link at `path` points, as stored (the text of the link, relative or absolute),
    /// read without following the link and without looking at what it points to. `path` is absolute and
    /// exact; the caller passes one whose own components are known not to be links, because the system
    /// follows those. `InvalidInput` when `path` is not a link, `NotFound` when there is nothing there,
    /// `Unsupported` where a source cannot tell. Used only to learn where a kept folder really is, by
    /// text, without touching the disk the link leads to; never for a target.
    fn read_link(&self, _path: &[u8]) -> io::Result<Vec<u8>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// A mount root on the way to a target that the scan went into, so cleanup may pass through it. On Linux
/// a separate `/home` (a btrfs subvolume of the same disk) is one: the scan reads it as a plain folder.
/// It is recorded with the identity the plan saw, so only that mount at that place is allowed: a mount
/// the scan did not go into, one that appeared later, and a different one now at the same place are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnteredMount {
    /// Index of the mount's component in the target's path, counted from the first component below `/`
    /// (so the same index as in [`EntryProbe::probe`]'s answer).
    pub depth: usize,
    pub dev: u64,
    pub ino: u64,
    /// The mount id the scan saw. When there is one, the mount found later must report the very same id: a
    /// system that stops reporting ids is not the same mount.
    pub mount_id: Option<u64>,
}

impl EnteredMount {
    /// Whether a mount root found now at `depth` with this identity is this mount.
    pub fn is(&self, depth: usize, dev: u64, ino: u64, mount_id: Option<u64>) -> bool {
        let same_mount = match self.mount_id {
            Some(planned) => mount_id == Some(planned),
            None => true,
        };

        self.depth == depth && self.dev == dev && self.ino == ino && same_mount
    }
}

/// What a target has to be when it is acted on: what the plan saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedEntry {
    pub dev: u64,
    pub ino: u64,
    /// Compared when both the plan and the system report one.
    pub mount_id: Option<u64>,
    /// When the entry was created. Compared when both the plan and the system report one.
    pub birth: Option<BirthTime>,
    pub is_dir: bool,
    /// How many components the scan root has: no component below it may start a mount.
    pub root_depth: usize,
    /// The mounts the scan went into that lie between the scan root and the target. Any other mount below
    /// the scan root is a refusal. Empty means none is allowed.
    pub entered_mounts: Vec<EnteredMount>,
}

impl ExpectedEntry {
    /// Whether a mount root found now on the way, at `depth`, is one the plan allowed.
    pub fn allows_mount(&self, depth: usize, dev: u64, ino: u64, mount_id: Option<u64>) -> bool {
        self.entered_mounts
            .iter()
            .any(|mount| mount.is(depth, dev, ino, mount_id))
    }
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

/// What [`EntryRemover::trash`] reports (as the payload of its `io::Error`, see [`not_recycled`]) when the
/// entry is gone from its place but cannot be found in the trash afterwards: the system deleted it for
/// good (an item too large for the Recycle Bin, say), or moved it where it could not be told from other
/// items. It is not recorded as trashed, because it cannot be put back by Platter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotRecycled(pub String);

impl std::fmt::Display for NotRecycled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotRecycled {}

impl NotRecycled {
    /// The `io::Error` a remover returns for it.
    pub fn into_error(self) -> io::Error {
        io::Error::other(self)
    }
}

/// Why the error of a trash call says "gone, but not in the trash", when it does (see [`NotRecycled`]).
pub fn not_recycled(error: &io::Error) -> Option<&str> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<NotRecycled>())
        .map(|inner| inner.0.as_str())
}

/// What [`EntryRemover::trash`] reports (as the payload of its `io::Error`, see [`unconfirmed`]) when it
/// is not known what became of the entry: the system did not answer in time (it may be waiting on a prompt
/// nobody sees), so the entry may still be where it was, or on its way to the trash. It is neither a
/// removal nor a failure, is never counted as freed space, and is never offered for restoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unconfirmed(pub String);

impl std::fmt::Display for Unconfirmed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unconfirmed {}

impl Unconfirmed {
    /// The `io::Error` a remover returns for it.
    pub fn into_error(self) -> io::Error {
        io::Error::other(self)
    }
}

/// Why the error of a trash call says "not known what happened", when it does (see [`Unconfirmed`]).
pub fn unconfirmed(error: &io::Error) -> Option<&str> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<Unconfirmed>())
        .map(|inner| inner.0.as_str())
}

/// What happened to one direct child of a folder in [`EntryRemover::trash_children`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildTrashed {
    /// Moved to the trash, to this exact path.
    To(Vec<u8>),
    /// Gone from its place but not found in the trash (see [`NotRecycled`]): the reason.
    NotRecycled(String),
    /// It is not known what became of it (see [`Unconfirmed`]): the reason.
    Unconfirmed(String),
    /// Left where it was, with the reason (a link, in use, changed since it was looked at).
    Left(String),
}

/// One direct child of a folder that [`EntryRemover::trash_children`] looked at, and what became of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedChild {
    /// Its name (no separator).
    pub name: Vec<u8>,
    /// Identity as it was when looked at, for putting it back.
    pub dev: u64,
    pub ino: u64,
    pub is_dir: bool,
    /// Bytes it takes (the length of a file; 0 for a folder, whose size is not looked at).
    pub allocated: u64,
    pub outcome: ChildTrashed,
}

/// What [`EntryRemover::trash_children`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrashedChildren {
    /// The children acted on or left, with what became of each.
    pub children: Vec<TrashedChild>,
    /// How many more children there were beyond the limit; they were not looked at.
    pub more: u64,
}

/// Acts on the disk for cleanup. Every method checks the target against [`ExpectedEntry`] right before
/// it acts and fails without touching anything when it differs: a link, another file in its place, a
/// mount on the way. Nothing is resolved by path string below `/`; no symlink is ever followed.
pub trait EntryRemover: Send + Sync {
    /// Deletes for good. Never crosses into another mount inside a folder. `Unsupported` on Windows and
    /// macOS, where cleanup only ever moves to the trash.
    fn remove(&self, path: &[u8], expected: &ExpectedEntry, mode: RemoveMode) -> io::Result<Removed>;

    /// Moves the entry to the user's trash, so it can be restored. Returns where it went. Linux:
    /// freedesktop layout under `trash_dir`, which must be on the same filesystem. Windows: the Recycle
    /// Bin, `trash_dir` is ignored, and the answer is the path of the item inside `$Recycle.Bin`; an
    /// entry that was taken away but cannot be found there is an error carrying [`NotRecycled`]. macOS:
    /// `~/.Trash` (`trash_dir`) on its volume, else that volume's `.Trashes/<uid>`, never overwriting.
    fn trash(&self, path: &[u8], expected: &ExpectedEntry, trash_dir: &Path) -> io::Result<Vec<u8>>;

    /// Moves every direct child of the folder at `path` to the trash and leaves the folder (what
    /// "empty a folder" means where nothing is deleted for good): the folder is
    /// checked against `expected` first and held while its children are looked at. A child that is a
    /// link, or that cannot be moved, is left and reported and does not count against `limit`; at most `limit`
    /// children are moved (and a bounded multiple of it looked at), the rest stay for the next run (counted
    /// in [`TrashedChildren::more`]). `Unsupported` where [`remove`](Self::remove) with
    /// [`RemoveMode::Contents`] is how a folder is emptied (Linux).
    fn trash_children(
        &self,
        _path: &[u8],
        _expected: &ExpectedEntry,
        _trash_dir: &Path,
        _limit: usize,
    ) -> io::Result<TrashedChildren> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    /// Puts an entry that [`trash`](Self::trash) moved to `trashed` back at `original`, when it is still
    /// the entry that was trashed (`expected`) and nothing has taken its old name since; then removes its
    /// `.trashinfo` record (Linux). Linux and macOS never overwrite (`RENAME_NOREPLACE`, `RENAME_EXCL`). Windows looks at the old
    /// name before it moves and refuses a taken one, but the shell's move is not atomic with that look.
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

    /// Runs `commands` in order and answers for each, in the same order. A runner that can ask for the
    /// administrator password once for all of them does so. The default runs them one by one and, after
    /// the first [`ToolFailure::NotAuthorized`], does not ask again: every command left answers
    /// `NotAuthorized` without running.
    fn run_batch(&self, commands: &[ToolCommand<'_>]) -> Vec<Result<(), ToolFailure>> {
        run_each(self, commands)
    }
}

/// Runs `commands` one by one with `runner` and answers for each, in order. After the first
/// [`ToolFailure::NotAuthorized`] it asks no more: every command left answers `NotAuthorized` unrun.
pub fn run_each<R: ToolRunner + ?Sized>(
    runner: &R,
    commands: &[ToolCommand<'_>],
) -> Vec<Result<(), ToolFailure>> {
    let mut refused = false;

    commands
        .iter()
        .map(|command| {
            if refused {
                return Err(ToolFailure::NotAuthorized);
            }

            let answer = runner.run(command);
            refused = answer == Err(ToolFailure::NotAuthorized);

            answer
        })
        .collect()
}

/// Lists the volumes the machine has mounted.
pub trait VolumeProvider: Send + Sync {
    fn list(&self) -> Vec<Volume>;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Answers each program as told and records what it was asked to run.
    struct Scripted {
        answers: Vec<(&'static str, Result<(), ToolFailure>)>,
        ran: Mutex<Vec<&'static str>>,
    }

    impl ToolRunner for Scripted {
        fn run(&self, command: &ToolCommand<'_>) -> Result<(), ToolFailure> {
            let (name, answer) = self
                .answers
                .iter()
                .find(|(name, _)| *name == command.program)
                .expect("a scripted program");
            self.ran.lock().unwrap().push(name);

            answer.clone()
        }
    }

    fn command(program: &'static str) -> ToolCommand<'static> {
        ToolCommand {
            program,
            args: &[],
            elevated: true,
        }
    }

    #[test]
    fn a_batch_is_run_one_by_one_and_answers_each_in_order() {
        let runner = Scripted {
            answers: vec![
                ("a", Ok(())),
                ("b", Err(ToolFailure::NotInstalled)),
                ("c", Ok(())),
            ],
            ran: Mutex::new(Vec::new()),
        };

        let answers = runner.run_batch(&[command("a"), command("b"), command("c")]);

        assert_eq!(answers, [Ok(()), Err(ToolFailure::NotInstalled), Ok(())]);
        assert_eq!(*runner.ran.lock().unwrap(), ["a", "b", "c"]);
    }

    #[test]
    fn a_batch_asks_no_more_after_a_refusal() {
        let runner = Scripted {
            answers: vec![
                ("a", Ok(())),
                ("b", Err(ToolFailure::NotAuthorized)),
                ("c", Ok(())),
                ("d", Ok(())),
            ],
            ran: Mutex::new(Vec::new()),
        };

        let answers = runner.run_batch(&[command("a"), command("b"), command("c"), command("d")]);

        assert_eq!(
            answers,
            [
                Ok(()),
                Err(ToolFailure::NotAuthorized),
                Err(ToolFailure::NotAuthorized),
                Err(ToolFailure::NotAuthorized)
            ]
        );
        assert_eq!(*runner.ran.lock().unwrap(), ["a", "b"]);
    }

    #[test]
    fn an_empty_batch_runs_nothing() {
        let runner = Scripted {
            answers: Vec::new(),
            ran: Mutex::new(Vec::new()),
        };

        assert!(runner.run_batch(&[]).is_empty());
    }

    fn expecting(mounts: Vec<EnteredMount>) -> ExpectedEntry {
        ExpectedEntry {
            dev: 1,
            ino: 2,
            mount_id: None,
            birth: None,
            is_dir: true,
            root_depth: 1,
            entered_mounts: mounts,
        }
    }

    #[test]
    fn only_the_recorded_mount_at_its_place_is_allowed() {
        let mount = EnteredMount {
            depth: 2,
            dev: 7,
            ino: 8,
            mount_id: Some(30),
        };
        let expected = expecting(vec![mount]);

        assert!(expected.allows_mount(2, 7, 8, Some(30)));
        assert!(
            !expected.allows_mount(2, 7, 8, None),
            "a mount that reports no id now is not the mount that had one"
        );
        assert!(
            expecting(vec![EnteredMount {
                mount_id: None,
                ..mount
            }])
            .allows_mount(2, 7, 8, Some(30)),
            "a scan that saw no id compares device and inode only"
        );
        assert!(
            !expected.allows_mount(2, 7, 8, Some(31)),
            "another mount of the same filesystem"
        );
        assert!(!expected.allows_mount(2, 7, 9, Some(30)), "another root");
        assert!(!expected.allows_mount(2, 6, 8, Some(30)), "another device");
        assert!(
            !expected.allows_mount(3, 7, 8, Some(30)),
            "another place on the path"
        );
        assert!(
            !expecting(Vec::new()).allows_mount(2, 7, 8, Some(30)),
            "nothing is allowed by default"
        );
    }
}
