//! Deleting and trashing for cleanup (see `platter_core::EntryRemover`).
//!
//! Every step goes through descriptors:
//!
//! 1. The parent is reached from `/` one component at a time with `O_PATH | O_NOFOLLOW | O_DIRECTORY`;
//!    a link on the way fails, and below the scan root no component may start a mount.
//! 2. The target is described with `statx(parent, name, AT_SYMLINK_NOFOLLOW)` and compared with the
//!    plan (device, inode, mount id, file or folder). A folder is then opened with `O_NOFOLLOW` and its
//!    descriptor described again: what is emptied is the folder that was checked, whatever happens to
//!    the name meanwhile.
//! 3. A folder is emptied through its descriptor: names are read first, then each entry is described
//!    again and removed with `unlinkat`. A folder inside that starts another mount is left alone. A
//!    folder without write permission (Go's module cache makes its folders read-only) is made writable
//!    for its owner when a removal is refused.
//!
//! What remains between the check of a name and `unlinkat` of that name is a swap for another entry by
//! someone who can write to that folder; `unlinkat` never follows a link, so the most such a swap can do
//! is have their own entry removed instead.

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;

use platter_core::{EntryKind, EntryRemover, ExpectedEntry, RemoveMode, Removed};
use rustix::fs::{
    AtFlags, CWD, Mode, OFlags, RawDir, RenameFlags, SeekFrom, fchmod, fstatfs, openat, renameat_with, seek,
    statvfs, syncfs, unlinkat,
};

use super::stat::{Stat, is_boundary, stat_at, stat_fd};

const PATH_FLAGS: OFlags = OFlags::PATH
    .union(OFlags::NOFOLLOW)
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);
const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// Deepest folder emptied; deeper trees are left with an error instead of exhausting descriptors.
const MAX_DEPTH: usize = 512;
/// Names read before they are removed.
const BATCH: usize = 1024;
/// Passes over one folder; more only happen when something keeps adding entries while it is emptied.
const MAX_PASSES: usize = 16;

/// [`EntryRemover`] on Linux descriptors.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxEntryRemover;

impl EntryRemover for LinuxEntryRemover {
    fn remove(&self, path: &[u8], expected: &ExpectedEntry, mode: RemoveMode) -> io::Result<Removed> {
        let (parent, name) = open_parent(path, expected.root_depth)?;
        let stat = check(&parent, &name, expected)?;
        let mut removed = Removed::default();

        if !expected.is_dir {
            if mode == RemoveMode::Contents {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "only a folder can be emptied",
                ));
            }
            unlinkat(&parent, name.as_c_str(), AtFlags::empty())?;
            removed.entries = 1;
            return Ok(removed);
        }

        let dir = openat(&parent, name.as_c_str(), DIR_FLAGS, Mode::empty())?;
        let opened = stat_fd(&dir)?;
        if !same(&opened, &stat) {
            return Err(changed("the folder was replaced while it was opened"));
        }

        let mut prefix = path.to_vec();
        empty(&dir, &opened, &mut prefix, 0, &mut removed);

        if mode == RemoveMode::Whole {
            match unlinkat(&parent, name.as_c_str(), AtFlags::REMOVEDIR) {
                Ok(()) => removed.entries += 1,
                Err(error) => removed
                    .errors
                    .push((path.to_vec(), io::Error::from(error).to_string())),
            }
        }

        Ok(removed)
    }

    fn trash(&self, path: &[u8], expected: &ExpectedEntry, trash_dir: &Path) -> io::Result<Vec<u8>> {
        super::trash::move_to_trash(path, expected, trash_dir)
    }

    fn restore(&self, trashed: &[u8], expected: &ExpectedEntry, original: &[u8]) -> io::Result<()> {
        super::trash::restore(trashed, expected, original)
    }

    fn free_bytes(&self, path: &Path) -> io::Result<u64> {
        let stats = statvfs(path)?;
        Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
    }

    /// Only btrfs needs it (it frees deleted space at its next commit); elsewhere a sync could only
    /// make the run wait, on a busy or network filesystem for a long time.
    fn settle(&self, path: &Path) -> io::Result<()> {
        const BTRFS_SUPER_MAGIC: i64 = 0x9123_683E;

        let dir = openat(CWD, path, DIR_FLAGS, Mode::empty())?;
        // `f_type` is `i64` on most targets and narrower on some; compare as `i64` everywhere.
        #[allow(clippy::unnecessary_cast)]
        let kind = fstatfs(&dir)?.f_type as i64;
        if kind == BTRFS_SUPER_MAGIC {
            syncfs(&dir)?;
        }
        Ok(())
    }
}

/// The descriptor of the folder holding `path` and the last name. No link is followed on the way, and
/// below the first `root_depth` components no component may start a mount.
pub(super) fn open_parent(path: &[u8], root_depth: usize) -> io::Result<(OwnedFd, CString)> {
    let parts = components(path)?;
    let Some((last, way)) = parts.split_last() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the filesystem root is never removed",
        ));
    };

    let mut dir = openat(CWD, c"/", PATH_FLAGS, Mode::empty())?;
    let mut dev = stat_fd(&dir)?.meta.dev;

    for (position, part) in way.iter().enumerate() {
        let name = CString::new(*part).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let next = openat(&dir, name.as_c_str(), PATH_FLAGS, Mode::empty()).map_err(|errno| {
            if matches!(errno, rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) {
                changed("a folder on the way is now a link or a file")
            } else {
                io::Error::from(errno)
            }
        })?;
        let stat = stat_fd(&next)?;

        // Component `position` sits at depth `position + 1`; the scan root's own components may be mounts.
        if position + 1 > root_depth && is_boundary(stat.reported_mount_root, stat.meta.dev, dev) {
            return Err(changed("a mount lies on the way"));
        }

        dev = stat.meta.dev;
        dir = next;
    }

    let name = CString::new(*last).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    Ok((dir, name))
}

/// Describes `name` below `parent` and compares it with the plan.
pub(super) fn check(parent: &OwnedFd, name: &CStr, expected: &ExpectedEntry) -> io::Result<Stat> {
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
    if let (Some(planned), Some(now)) = (expected.mount_id, stat.mount_id)
        && planned != now
    {
        return Err(changed("it is on another mount now"));
    }
    if is_boundary(stat.reported_mount_root, stat.meta.dev, parent_dev) {
        return Err(changed("it is a mount now"));
    }

    Ok(stat)
}

/// Removes everything inside the open folder `dir` (described by `this`). `path` is its path, extended
/// while descending, for error messages.
fn empty(dir: &OwnedFd, this: &Stat, path: &mut Vec<u8>, depth: usize, removed: &mut Removed) {
    if depth >= MAX_DEPTH {
        removed.note(path.clone(), "too deeply nested; left as it is".to_owned());
        return;
    }

    // Names are read and removed in batches, so a folder of millions of entries never has all its
    // names in memory. Removing while reading may make the listing skip entries, so the folder is read
    // again from the start until a pass removes nothing; what could not be removed is remembered and
    // not tried again.
    let mut left: HashSet<CString> = HashSet::new();
    let mut made_writable = false;
    for _ in 0..MAX_PASSES {
        if let Err(error) = seek(dir, SeekFrom::Start(0)) {
            removed.note(path.clone(), io::Error::from(error).to_string());
            break;
        }

        let mut buffer = Vec::with_capacity(32 * 1024);
        let mut raw = RawDir::new(dir.as_fd(), buffer.spare_capacity_mut());
        let mut progressed = false;
        let mut finished = false;

        while !finished {
            let mut batch = Vec::with_capacity(BATCH);
            while batch.len() < BATCH {
                match raw.next() {
                    None => {
                        finished = true;
                        break;
                    }
                    Some(Err(error)) => {
                        removed.note(path.clone(), io::Error::from(error).to_string());
                        finished = true;
                        break;
                    }
                    Some(Ok(entry)) => {
                        let name = entry.file_name();
                        if !matches!(name.to_bytes(), b"." | b"..") && !left.contains(name) {
                            batch.push(name.to_owned());
                        }
                    }
                }
            }

            for name in batch {
                let length = path.len();
                path.push(b'/');
                path.extend_from_slice(name.to_bytes());

                match remove_entry(dir, this, &name, path, depth, removed, &mut made_writable) {
                    Ok(Gone::Removed) => progressed = true,
                    Ok(Gone::Already) => {}
                    Ok(Gone::LeftAlone) => {
                        left.insert(name);
                    }
                    Err(error) => {
                        removed.note(path.clone(), error.to_string());
                        left.insert(name);
                    }
                }

                path.truncate(length);
            }
        }

        if !progressed {
            break;
        }
    }

    // The folder was made writable only to remove entries; give it its own mode back (it stays when
    // only its contents go, or when something in it could not be removed).
    if made_writable && let Err(error) = fchmod(dir, Mode::from_raw_mode(this.mode)) {
        removed.note(
            path.clone(),
            format!("its permissions could not be restored: {error}"),
        );
    }
}

/// What happened to one entry of a folder being emptied.
enum Gone {
    Removed,
    /// It had disappeared already.
    Already,
    /// Left on purpose (a folder of another mount).
    LeftAlone,
}

fn remove_entry(
    dir: &OwnedFd,
    this: &Stat,
    name: &CStr,
    path: &mut Vec<u8>,
    depth: usize,
    removed: &mut Removed,
    made_writable: &mut bool,
) -> io::Result<Gone> {
    let stat = match stat_at(dir, name) {
        Ok(stat) => stat,
        // Gone already: nothing to do.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Gone::Already),
        Err(error) => return Err(error),
    };

    let flags = if stat.meta.kind == EntryKind::Dir {
        let other_mount = match (this.mount_id, stat.mount_id) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        };
        if other_mount || is_boundary(stat.reported_mount_root, stat.meta.dev, this.meta.dev) {
            removed.skipped_mounts += 1;
            return Ok(Gone::LeftAlone);
        }

        let child = openat(dir, name, DIR_FLAGS, Mode::empty())?;
        let opened = stat_fd(&child)?;
        if !same(&opened, &stat) {
            return Err(changed("the folder was replaced while it was opened"));
        }
        empty(&child, &opened, path, depth + 1, removed);
        AtFlags::REMOVEDIR
    } else {
        AtFlags::empty()
    };

    match unlinkat(dir, name, flags) {
        Ok(()) => {}
        Err(rustix::io::Errno::ACCESS) if !*made_writable => {
            // A read-only folder of our own (Go's module cache): let its owner write and search it, for
            // as long as its entries are removed (`empty` restores the mode). Fails for someone else's.
            *made_writable = true;
            fchmod(dir, Mode::from_raw_mode(this.mode | 0o300))?;
            unlinkat(dir, name, flags)?;
        }
        Err(error) => return Err(error.into()),
    }

    removed.entries += 1;
    Ok(Gone::Removed)
}

fn same(a: &Stat, b: &Stat) -> bool {
    a.meta.dev == b.meta.dev && a.meta.ino == b.meta.ino
}

pub(super) fn changed(what: &str) -> io::Error {
    io::Error::other(format!("changed since the plan: {what}"))
}

pub(super) fn components(path: &[u8]) -> io::Result<Vec<&[u8]>> {
    let rest = path
        .strip_prefix(b"/")
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let rest = rest.strip_suffix(b"/").unwrap_or(rest);
    if rest.is_empty() {
        return Ok(Vec::new());
    }

    let parts: Vec<&[u8]> = rest.split(|&byte| byte == b'/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == b"." || *part == b"..")
    {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    Ok(parts)
}

/// Renames with `RENAME_NOREPLACE`, so nothing at the destination is ever overwritten.
pub(super) fn rename_no_replace(
    from: &OwnedFd,
    from_name: &CStr,
    to: &OwnedFd,
    to_name: &CStr,
) -> io::Result<()> {
    renameat_with(from, from_name, to, to_name, RenameFlags::NOREPLACE)?;
    Ok(())
}
