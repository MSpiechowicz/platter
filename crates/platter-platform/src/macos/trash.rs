//! Platter's own trash on macOS (package M2a): an identity-checked rename into `~/.Trash` (home volume) or
//! `<mount>/.Trashes/<uid>` (other volumes), and the move back. Where a path goes is decided by
//! [`crate::mac_pure::trash_place`].
//!
//! - A target on the device of the home trash (`trash_dir` as the callers pass it, `~/.Trash`) goes there.
//!   The folder is made (mode 0700) when it is missing, and must belong to the user.
//! - Any other target goes to `<mount point>/.Trashes/<uid>`, where the mount point is found by walking up
//!   while the device stays the same. That folder is never made here: it must exist (Finder makes it the
//!   first time something is trashed on the volume), be a real folder and not a link, belong to the user
//!   and be closed to others, and be on the target's device. Otherwise the move is refused, untouched.
//! - The move is `renameat` with `RENAME_EXCL`, which does not replace. A name that is taken becomes
//!   `name 2`, `name 3`, ... (before the extension of a file, as Finder does it). The entry that arrived is
//!   described again and must be the planned one; when it is not (a swap in the instants after the check)
//!   it is moved back when that is possible, and reported.
//! - A rename that would cross a volume (`EXDEV`) is refused: nothing is ever copied and deleted. A
//!   refusal by the system (`EPERM`, `EACCES`: the item is locked, or privacy protection of `~/.Trash`
//!   wants Full Disk Access) is `PermissionDenied`, and nothing was touched.
//!
//! Finder's "Put Back" information is not written, so Finder cannot put these items back. [`restore`] does.

use std::ffi::CStr;
use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;

use platter_core::ports::{ChildTrashed, NotRecycled, TrashedChild, TrashedChildren, Unconfirmed};
use platter_core::{EntryKind, ExpectedEntry};
use rustix::fs::{CWD, Dir, Mode, mkdirat, openat};
use rustix::io::Errno;
use rustix::process::getuid;

use super::probe::{DIR_FLAGS, Stat, Volumes, c_name, components, stat_at, stat_fd, walk};
use super::remove::{changed, check, is_same_entry, open_parent, rename_error, rename_no_replace};
use crate::mac_pure::trash_place::{
    MAX_NAMES, mount_point, numbered_name, trash_dir, trash_entry_kind, trash_folder_is_ours,
};

/// Children read (and so looked at) per child that may be moved, at most: the ones left alone do not count
/// against the limit, but a folder full of links cannot keep one action looking at them for ever.
const LOOKED_AT_PER_MOVE: usize = 4;

/// The user's id, as the text of a folder name below `.Trashes` has it.
fn uid() -> u32 {
    getuid().as_raw()
}

/// One trash folder, open and checked, ready to take entries from the device `dev`.
struct Place {
    dir: OwnedFd,
    /// Identity of the trash folder itself: what is moved into it is never it, and never its parent.
    dev: u64,
    ino: u64,
    /// The trash folder's path, for the answer.
    location: Vec<u8>,
}

impl Place {
    /// The trash for an entry of the device `dev` whose path is `path`.
    fn for_device(path: &[u8], dev: u64, home_trash: &Path) -> io::Result<Self> {
        let home = home_trash.as_os_str().as_encoded_bytes();
        let location = trash_dir(home, uid(), true, b"/").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the trash folder's path is not a plain path",
            )
        })?;

        // The devices are compared before the home trash is opened (and made when missing): a target on
        // another volume goes to that volume's folder and never needs the home trash.
        if home_device(&location)? == dev {
            let dir = open_home_trash(&location)?;
            let stat = stat_fd(&dir)?;
            if stat.uid != uid() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "the trash folder belongs to someone else",
                ));
            }
            if stat.meta.dev == dev {
                return Ok(Self {
                    dir,
                    dev: stat.meta.dev,
                    ino: stat.meta.ino,
                    location,
                });
            }
        }

        // Another volume: its own folder for the user, which must already be there.
        let mount = mount_of(path)?;
        let location = trash_dir(home, uid(), false, &mount).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the volume's mount point is not a plain path",
            )
        })?;
        let dir = open_existing_dir(&location).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => io::Error::new(
                io::ErrorKind::Unsupported,
                "this volume has no Trash folder for you (Finder makes it the first time something is trashed there), so nothing was moved",
            ),
            _ => error,
        })?;

        let stat = stat_fd(&dir)?;
        if stat.uid != uid() || stat.mode & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the volume's Trash folder belongs to someone else or is open to others",
            ));
        }
        if stat.meta.dev != dev {
            return Err(io::Error::new(
                io::ErrorKind::CrossesDevices,
                "the volume's Trash folder is on another device than the entry",
            ));
        }

        Ok(Self {
            dir,
            dev: stat.meta.dev,
            ino: stat.meta.ino,
            location,
        })
    }

    /// Moves `name`, an entry of the open folder `parent` that `expected` describes, into this trash and
    /// returns its path there.
    fn put(&self, parent: &OwnedFd, name: &CStr, expected: &Stat) -> io::Result<Vec<u8>> {
        let is_dir = expected.meta.kind == EntryKind::Dir;
        if (expected.meta.dev, expected.meta.ino) == (self.dev, self.ino) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "it is the Trash folder itself",
            ));
        }
        let holder = stat_fd(parent)?;
        if (holder.meta.dev, holder.meta.ino) == (self.dev, self.ino) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "it is in the Trash already",
            ));
        }

        for number in 1..=MAX_NAMES {
            let candidate = c_name(&numbered_name(name.to_bytes(), number, is_dir))?;

            match rename_no_replace(parent, name, &self.dir, &candidate) {
                Ok(()) => return self.arrived(parent, name, &candidate, expected),
                Err(Errno::EXIST) => {}
                Err(error) => return Err(rename_error(error)),
            }
        }

        Err(io::Error::other("no free name in the Trash"))
    }

    /// The entry just moved to `candidate` must be the planned one. When it is not, it is moved back to
    /// its old name if that is still free, and the move is reported as failed.
    fn arrived(
        &self,
        parent: &OwnedFd,
        name: &CStr,
        candidate: &CStr,
        expected: &Stat,
    ) -> io::Result<Vec<u8>> {
        let mut to = self.location.clone();
        to.push(b'/');
        to.extend_from_slice(candidate.to_bytes());

        let now = match stat_at(&self.dir, candidate) {
            Ok(now) => now,
            Err(error) => {
                return Err(Unconfirmed(format!(
                    "it was moved to the Trash, but could not be looked at there afterwards ({error})"
                ))
                .into_error());
            }
        };
        if is_same_entry(expected, &now) {
            return Ok(to);
        }

        match rename_no_replace(&self.dir, candidate, parent, name) {
            Ok(()) => Err(changed(
                "another entry had its name when it was moved, so it was put back where it was",
            )),
            Err(error) => Err(NotRecycled(format!(
                "another entry had its name when it was moved, and it could not be put back ({error}): it is in the Trash at {}",
                String::from_utf8_lossy(&to)
            ))
            .into_error()),
        }
    }
}

/// Moves the entry at `path` (which `expected` describes) into the trash. Returns where it went.
pub(super) fn move_to_trash(path: &[u8], expected: &ExpectedEntry, home_trash: &Path) -> io::Result<Vec<u8>> {
    let volumes = Volumes::read()?;
    let (parent, name) = open_parent(path, expected.root_depth, &volumes)?;
    let stat = check(&parent, &name, expected, &volumes)?;

    let place = Place::for_device(path, stat.meta.dev, home_trash)?;
    place.put(&parent, &name, &stat)
}

/// Moves every direct child of the folder at `path` into the trash, at most `limit` of them, and leaves the
/// folder. A child that is a link, is on another device, or cannot be moved stays and is reported.
pub(super) fn trash_children(
    path: &[u8],
    expected: &ExpectedEntry,
    home_trash: &Path,
    limit: usize,
) -> io::Result<TrashedChildren> {
    if !expected.is_dir {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only a folder can be emptied",
        ));
    }

    let volumes = Volumes::read()?;
    let (parent, name) = open_parent(path, expected.root_depth, &volumes)?;
    let stat = check(&parent, &name, expected, &volumes)?;

    // The folder is held open: it cannot be swapped for another while its children are moved.
    let folder = openat(&parent, &name, DIR_FLAGS, Mode::empty())?;
    let opened = stat_fd(&folder)?;
    if (opened.meta.dev, opened.meta.ino) != (stat.meta.dev, stat.meta.ino) {
        return Err(changed("the folder was replaced while it was opened"));
    }

    let place = Place::for_device(path, opened.meta.dev, home_trash)?;
    if (opened.meta.dev, opened.meta.ino) == (place.dev, place.ino) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the Trash folder itself is not emptied by Platter",
        ));
    }

    // Names are read first: moving while reading may make a listing skip entries.
    let ceiling = limit.saturating_mul(LOOKED_AT_PER_MOVE);
    let mut names = Vec::new();
    let mut more = 0u64;
    let mut listing = Dir::read_from(&folder)?;
    while let Some(entry) = listing.read() {
        let entry = entry?;
        let child = entry.file_name();
        if matches!(child.to_bytes(), b"." | b"..") {
            continue;
        }

        if names.len() < ceiling {
            names.push(child.to_owned());
        } else {
            more += 1;
        }
    }

    let mut children = Vec::with_capacity(names.len());
    let mut moved = 0usize;
    for child in names {
        if moved >= limit {
            more += 1;
            continue;
        }

        let outcome = trash_child(&place, &folder, &opened, &child);
        if matches!(
            outcome.outcome,
            ChildTrashed::To(_) | ChildTrashed::NotRecycled(_) | ChildTrashed::Unconfirmed(_)
        ) {
            moved += 1;
        }
        children.push(outcome);
    }

    Ok(TrashedChildren { children, more })
}

/// Looks at one child of the open folder and moves it into the trash when it is a plain file or folder on
/// the folder's own device.
fn trash_child(place: &Place, folder: &OwnedFd, holder: &Stat, name: &CStr) -> TrashedChild {
    let reported = |stat: Option<&Stat>, outcome: ChildTrashed| TrashedChild {
        name: name.to_bytes().to_vec(),
        dev: stat.map_or(0, |stat| stat.meta.dev),
        ino: stat.map_or(0, |stat| stat.meta.ino),
        is_dir: stat.is_some_and(|stat| stat.meta.kind == EntryKind::Dir),
        allocated: stat
            .filter(|stat| stat.meta.kind == EntryKind::File)
            .map_or(0, |stat| stat.meta.apparent),
        outcome,
    };

    let stat = match stat_at(folder, name) {
        Ok(stat) => stat,
        Err(error) => {
            return reported(
                None,
                ChildTrashed::Left(format!("it could not be looked at ({error})")),
            );
        }
    };

    match stat.meta.kind {
        EntryKind::Dir | EntryKind::File => {}
        EntryKind::Symlink => {
            return reported(
                Some(&stat),
                ChildTrashed::Left("it is a link, which is left alone".to_owned()),
            );
        }
        EntryKind::Other => {
            return reported(
                Some(&stat),
                ChildTrashed::Left("it is neither a file nor a folder".to_owned()),
            );
        }
    }
    if stat.meta.dev != holder.meta.dev {
        return reported(
            Some(&stat),
            ChildTrashed::Left("it is on another volume".to_owned()),
        );
    }

    let outcome = match place.put(folder, name, &stat) {
        Ok(to) => ChildTrashed::To(to),
        Err(error) => match (
            platter_core::ports::not_recycled(&error),
            platter_core::ports::unconfirmed(&error),
        ) {
            (Some(why), _) => ChildTrashed::NotRecycled(why.to_owned()),
            (None, Some(why)) => ChildTrashed::Unconfirmed(why.to_owned()),
            (None, None) => ChildTrashed::Left(error.to_string()),
        },
    };

    reported(Some(&stat), outcome)
}

/// Moves the entry at `trashed` (`<trash>/<name>`) back to `original`, when it is still the planned one
/// (`expected`) and lies directly in `home_trash` (the user's `~/.Trash`, which the environment names) or in
/// the user's `.Trashes/<uid>` of a volume, and the folder it lies in belongs to the user. Nothing is overwritten: a taken old name is `AlreadyExists`. The folder it goes back into
/// must exist, with no link on the way. Mounts on the way do not matter here (the trash and the original
/// folder may be on any mount); a rename that crosses a device is refused, never a copy.
pub(super) fn restore(
    trashed: &[u8],
    expected: &ExpectedEntry,
    original: &[u8],
    home_trash: Option<&Path>,
) -> io::Result<()> {
    let home_trash = home_trash.map(|path| path.as_os_str().as_encoded_bytes());
    let Some(kind) = trash_entry_kind(trashed, uid(), home_trash) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not an entry of a trash folder",
        ));
    };

    let volumes = Volumes::read()?;
    let (files, entry) = open_parent(trashed, usize::MAX, &volumes)?;

    // The folder that is held is the one that is looked at: it is the user's own, and a volume's is closed
    // to others, or another user could have put an entry of their own in it.
    let holder = stat_fd(&files)?;
    if !trash_folder_is_ours(kind, holder.uid, holder.mode, uid()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the Trash folder belongs to someone else or is open to others",
        ));
    }

    let stat = check(&files, &entry, expected, &volumes)?;
    let (folder, original_name) = open_parent(original, usize::MAX, &volumes)?;

    match rename_no_replace(&files, &entry, &folder, &original_name) {
        Ok(()) => {}
        Err(Errno::EXIST) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "something else has taken its old name",
            ));
        }
        Err(error) => return Err(rename_error(error)),
    }

    // What arrived is the planned entry. When it is not, it goes back where it was found, if it can.
    let now = stat_at(&folder, &original_name).map_err(|error| {
        Unconfirmed(format!(
            "it was put back, but could not be looked at afterwards ({error})"
        ))
        .into_error()
    })?;
    if is_same_entry(&stat, &now) {
        return Ok(());
    }

    match rename_no_replace(&folder, &original_name, &files, &entry) {
        Ok(()) => Err(changed(
            "another entry had its name when it was put back, so it was left in the Trash",
        )),
        Err(error) => Err(io::Error::other(format!(
            "changed since the plan: another entry had its name when it was put back, and it could not be moved back ({error})"
        ))),
    }
}

/// The mount point of the volume that holds `path`: the deepest folder above it that starts a mount.
fn mount_of(path: &[u8]) -> io::Result<Vec<u8>> {
    let parts = components(path)?;
    let found = walk(path)?;

    let above = found.len().saturating_sub(1);
    let ancestors: Vec<(&[u8], bool)> = parts
        .iter()
        .zip(&found)
        .take(above)
        .map(|(name, entry)| (*name, entry.meta.is_mount_root))
        .collect();

    Ok(mount_point(&ancestors))
}

/// The device of the folder that holds the home trash (the home folder), looked at without creating
/// anything. The trash folder is made in it, so it lies on this device unless it is a mount of its own.
fn home_device(trash: &[u8]) -> io::Result<u64> {
    let parts = components(trash)?;
    let Some((_, above)) = parts.split_last() else {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    };

    let mut dir = openat(CWD, c"/", DIR_FLAGS, Mode::empty())?;
    for part in above {
        let name = c_name(part)?;
        dir = openat(&dir, &name, DIR_FLAGS, Mode::empty()).map_err(denied)?;
    }

    Ok(stat_fd(&dir)?.meta.dev)
}

/// Opens an existing folder without following links and without creating anything.
fn open_existing_dir(path: &[u8]) -> io::Result<OwnedFd> {
    let mut dir = openat(CWD, c"/", DIR_FLAGS, Mode::empty())?;
    for part in components(path)? {
        let name = c_name(part)?;
        dir = openat(&dir, &name, DIR_FLAGS, Mode::empty()).map_err(denied)?;
    }

    Ok(dir)
}

/// Opens the home trash (`path` is clean and absolute): every folder above it exists, and the last name
/// is made (mode 0700) when it is missing. No link is followed.
fn open_home_trash(path: &[u8]) -> io::Result<OwnedFd> {
    let parts = components(path)?;
    let Some((last, above)) = parts.split_last() else {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    };

    let mut dir = openat(CWD, c"/", DIR_FLAGS, Mode::empty())?;
    for part in above {
        let name = c_name(part)?;
        dir = openat(&dir, &name, DIR_FLAGS, Mode::empty()).map_err(denied)?;
    }

    let name = c_name(last)?;
    match openat(&dir, &name, DIR_FLAGS, Mode::empty()) {
        Ok(trash) => return Ok(trash),
        Err(Errno::NOENT) => {}
        Err(error) => return Err(denied(error)),
    }

    // Missing: make it, as Finder does for a user who has never trashed anything. A rival that made it
    // meanwhile is fine, it is opened below like any other (and its owner is checked by the caller).
    match mkdirat(&dir, &name, Mode::RWXU) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(error) => return Err(denied(error)),
    }

    openat(&dir, &name, DIR_FLAGS, Mode::empty()).map_err(denied)
}

/// An error of opening a folder of the trash. Privacy protection answers `EPERM` for `~/.Trash` of an app
/// without Full Disk Access.
fn denied(errno: Errno) -> io::Error {
    match errno {
        Errno::PERM | Errno::ACCESS => io::Error::new(
            io::ErrorKind::PermissionDenied,
            "macOS did not allow Platter into the Trash folder (it may need Full Disk Access)",
        ),
        Errno::LOOP | Errno::NOTDIR => io::Error::from(io::ErrorKind::NotADirectory),
        other => io::Error::from(other),
    }
}
