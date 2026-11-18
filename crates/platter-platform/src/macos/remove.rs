//! The entry remover on macOS (package M2a): moving to the trash and putting back, and nothing else.
//!
//! [`MacEntryRemover`] never deletes: `remove` refuses every mode (`Unsupported`), as on Windows. A target
//! is moved into the trash (see [`trash`](super::trash)) and nothing is ever copied or deleted for good.
//!
//! Every step goes through descriptors:
//!
//! 1. The folder holding the target is reached from `/` one component at a time with
//!    `O_NOFOLLOW | O_DIRECTORY`: a link on the way fails, and below the scan root no component may start a
//!    mount.
//! 2. The target is described with `fstatat(AT_SYMLINK_NOFOLLOW)` and compared with the plan (device,
//!    inode, creation time, file or folder, not a mount).
//! 3. It is moved with `renameat` below those two descriptors, which does not replace anything, and what
//!    arrived is described again and compared with the plan once more.
//!
//! What remains between the check of a name and its `renameat` is a swap for another entry by someone who
//! can write to that folder. The last comparison tells it (the entry is then moved back, or reported), and
//! `renameat` never follows a link.

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;

use platter_core::ports::TrashedChildren;
use platter_core::{EntryKind, EntryRemover, ExpectedEntry, RemoveMode, Removed};
use rustix::fs::{CWD, Mode, OFlags, RenameFlags, fstatvfs, openat, renameat_with};
use rustix::io::Errno;

use super::probe::{DIR_FLAGS, Stat, Volumes, c_name, components, stat_at, stat_fd};

/// [`EntryRemover`] on macOS descriptors.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacEntryRemover;

impl MacEntryRemover {
    /// [`EntryRemover::restore`] for a home trash at `home_trash`, where the trait method takes the
    /// user's own (`~/.Trash`, from the environment). The same rules apply to it: a direct child, and a
    /// folder the user owns. For a caller that moved into another home trash (a test's own folder).
    pub fn restore_from(
        &self,
        trashed: &[u8],
        expected: &ExpectedEntry,
        original: &[u8],
        home_trash: &Path,
    ) -> io::Result<()> {
        super::trash::restore(trashed, expected, original, Some(home_trash))
    }
}

impl EntryRemover for MacEntryRemover {
    /// Refuses every mode: cleanup on macOS only moves to the trash.
    fn remove(&self, _path: &[u8], _expected: &ExpectedEntry, _mode: RemoveMode) -> io::Result<Removed> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Platter only moves to the Trash on macOS",
        ))
    }

    fn trash(&self, path: &[u8], expected: &ExpectedEntry, trash_dir: &Path) -> io::Result<Vec<u8>> {
        super::trash::move_to_trash(path, expected, trash_dir)
    }

    fn trash_children(
        &self,
        path: &[u8],
        expected: &ExpectedEntry,
        trash_dir: &Path,
        limit: usize,
    ) -> io::Result<TrashedChildren> {
        super::trash::trash_children(path, expected, trash_dir, limit)
    }

    fn restore(&self, trashed: &[u8], expected: &ExpectedEntry, original: &[u8]) -> io::Result<()> {
        let home_trash = crate::environment::current_home_trash();

        super::trash::restore(trashed, expected, original, home_trash.as_deref())
    }

    fn free_bytes(&self, path: &Path) -> io::Result<u64> {
        // Not `O_DIRECTORY`: a file's volume is as good. `O_NONBLOCK` so that a pipe cannot hold the open.
        let fd = openat(
            CWD,
            path,
            OFlags::RDONLY.union(OFlags::NONBLOCK).union(OFlags::CLOEXEC),
            Mode::empty(),
        )?;
        let stats = fstatvfs(&fd)?;

        Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
    }
}

/// The descriptor of the folder holding `path` and the last name. No link is followed on the way, and below
/// the first `root_depth` components no component may start a mount.
pub(super) fn open_parent(
    path: &[u8],
    root_depth: usize,
    volumes: &Volumes,
) -> io::Result<(OwnedFd, CString)> {
    let parts = components(path)?;
    let Some((last, way)) = parts.split_last() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the filesystem root is never moved",
        ));
    };

    let mut dir = openat(CWD, c"/", DIR_FLAGS, Mode::empty())?;
    let mut dev = stat_fd(&dir)?.meta.dev;

    for (position, part) in way.iter().enumerate() {
        let name = c_name(part)?;
        let next = openat(&dir, &name, DIR_FLAGS, Mode::empty()).map_err(|errno| {
            if matches!(errno, Errno::LOOP | Errno::NOTDIR) {
                changed("a folder on the way is now a link or a file")
            } else {
                io::Error::from(errno)
            }
        })?;
        let stat = stat_fd(&next)?;

        // Component `position` sits at depth `position + 1`; the scan root's own components may be mounts.
        if position + 1 > root_depth && volumes.starts_mount(stat.meta.dev, dev) {
            return Err(changed("a mount lies on the way"));
        }

        dev = stat.meta.dev;
        dir = next;
    }

    Ok((dir, c_name(last)?))
}

/// Describes `name` below `parent` and compares it with the plan.
pub(super) fn check(
    parent: &OwnedFd,
    name: &CStr,
    expected: &ExpectedEntry,
    volumes: &Volumes,
) -> io::Result<Stat> {
    let stat = stat_at(parent, name)?;
    let parent_dev = stat_fd(parent)?.meta.dev;

    let kind_ok = match stat.meta.kind {
        EntryKind::Dir => expected.is_dir,
        EntryKind::File => !expected.is_dir,
        _ => false,
    };
    if !kind_ok {
        return Err(changed("it is no longer the same kind of entry"));
    }
    if stat.meta.dev != expected.dev || stat.meta.ino != expected.ino {
        return Err(changed("another entry now has its name"));
    }
    if let (Some(planned), Some(now)) = (expected.birth, stat.birth)
        && planned != now
    {
        return Err(changed("another entry now has its name"));
    }
    if volumes.starts_mount(stat.meta.dev, parent_dev) {
        return Err(changed("it is a mount now"));
    }

    Ok(stat)
}

/// Whether `now` is the entry `planned` described: the same kind, device, inode and, when both have one,
/// creation time.
pub(super) fn is_same_entry(planned: &Stat, now: &Stat) -> bool {
    let births_agree = match (planned.birth, now.birth) {
        (Some(planned), Some(now)) => planned == now,
        _ => true,
    };

    planned.meta.kind == now.meta.kind
        && planned.meta.dev == now.meta.dev
        && planned.meta.ino == now.meta.ino
        && births_agree
}

pub(super) fn changed(what: &str) -> io::Error {
    io::Error::other(format!("changed since the plan: {what}"))
}

/// Renames with `RENAME_EXCL`, so nothing at the destination is ever overwritten. The error is the raw
/// one: a taken name is `Errno::EXIST`, which the caller tells from the rest.
pub(super) fn rename_no_replace(
    from: &OwnedFd,
    from_name: &CStr,
    to: &OwnedFd,
    to_name: &CStr,
) -> Result<(), Errno> {
    renameat_with(from, from_name, to, to_name, RenameFlags::NOREPLACE)
}

/// What a refused rename is, in words the user can act on. Never a reason to copy and delete instead.
pub(super) fn rename_error(errno: Errno) -> io::Error {
    match errno {
        Errno::XDEV => io::Error::new(
            io::ErrorKind::CrossesDevices,
            "it is on another volume than its trash, and Platter never copies",
        ),
        Errno::PERM | Errno::ACCESS => io::Error::new(
            io::ErrorKind::PermissionDenied,
            "macOS did not allow the move (the item is locked, or Platter needs Full Disk Access)",
        ),
        Errno::NOTSUP | Errno::OPNOTSUPP => io::Error::new(
            io::ErrorKind::Unsupported,
            "this volume cannot move an entry without replacing, so nothing was moved",
        ),
        other => io::Error::from(other),
    }
}
