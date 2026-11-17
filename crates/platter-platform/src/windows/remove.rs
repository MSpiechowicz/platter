//! The entry remover on Windows (W2).
//!
//! [`WindowsEntryRemover`] moves a target to the Recycle Bin and nothing else: `remove` (deleting for
//! good) is `Unsupported`, because Windows has no way to delete by handle without `unsafe`, and a move to
//! the Recycle Bin that loses a race can be put back, a delete cannot.
//!
//! To move one entry it walks the path again (see [`probe::walk`](super::probe)), every folder on the way
//! opened as a reparse point and held open without `FILE_SHARE_DELETE` for the whole action, compares what
//! it found with what the plan saw (identity, kind, creation time, no volume change and no link below the
//! scan root), lets go of its own handle on the target, and calls `trash::delete`. It then looks the item
//! up in the Recycle Bin by its old path and deletion time, and accepts it only when the file `$R...` that
//! the shell names has the identity that was planned. That path is the answer, and what `restore` is given
//! later.
//!
//! What is not closed: the last name can be swapped between the walk and the shell's move (the shell opens
//! it by name). The identity of the item in the bin is checked after, so a lost race is told, and the item
//! is in the Recycle Bin where the user can put it back. An entry the shell took away but that is not in
//! the bin afterwards (deleted for good, say) is reported as [`NotRecycled`], never as trashed. So is one
//! that cannot be looked at after the shell ran (it may be in the bin): neither failed nor restorable.
//!
//! The shell is called with `FOF_ALLOWUNDO | FOF_WANTNUKEWARNING | FOF_NO_UI`. An item the Recycle Bin
//! will not take (too large for it, a drive set to delete at once) is skipped by the shell and reported
//! by `trash` (the crate's progress sink) as an error, and stays where it is: the CI tests in
//! `tests/windows_cleanup.rs` set the volume's bin to delete immediately, and to a tiny size, and check
//! that the item is still there. Platter does not read those settings first (no safe API for them
//! without a registry crate, which the project does not have).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use platter_core::ports::{ChildTrashed, NotRecycled, TrashedChild, TrashedChildren};
use platter_core::{EntryKind, EntryRemover, ExpectedEntry, RemoveMode, Removed};

use super::identity::identity_of_path;
use super::probe::{Hold, Walk, is_plain_name, is_short_enough, join, parse, timed, walk};
use super::recycle::{self, Wanted};

/// [`EntryRemover`] on the Recycle Bin.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsEntryRemover;

impl EntryRemover for WindowsEntryRemover {
    fn remove(&self, _path: &[u8], _expected: &ExpectedEntry, _mode: RemoveMode) -> io::Result<Removed> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Platter only moves to the Recycle Bin on Windows",
        ))
    }

    fn trash(&self, path: &[u8], expected: &ExpectedEntry, _trash_dir: &Path) -> io::Result<Vec<u8>> {
        let parts = parse(path)?;
        let original = join(&parts, parts.len());
        if !is_short_enough(&original) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the path is too long",
            ));
        }

        let looked = held_walk(path, Hold::Ancestors)?;
        check(&looked, expected)?;
        recycle::ensure_recyclable(&parts[0])?;

        let wanted = wanted(expected);
        // The folders above stay held until the action is over; the target itself must not be.
        let Walk {
            held: _ancestors,
            target,
            ..
        } = looked;
        drop(target);

        let since = now_seconds();
        let moved = trash::delete(&original);

        // Still there as planned: it was not moved, whatever the shell said. When that cannot be told
        // the shell has already run, so it is not reported as a failure: it may be in the Recycle Bin.
        match recycle_state(&original, &wanted) {
            Ok(State::Still) => {
                return Err(match moved {
                    Err(error) => io::Error::other(error),
                    Ok(()) => io::Error::other("the shell did not move it"),
                });
            }
            Ok(State::Gone) => {}
            Err(error) => return Err(NotRecycled(unknown_after_move(&error)).into_error()),
        }

        let why = match &moved {
            Err(error) => format!("the shell reported: {error}"),
            Ok(()) => "the shell moved it".to_owned(),
        };
        let items = recycle::items().map_err(|error| {
            NotRecycled(format!(
                "it is gone, but the Recycle Bin could not be read to find it ({error})"
            ))
            .into_error()
        })?;

        match recycle::find(&items, &original, since, &wanted) {
            Some(item) => Ok(item.into_os_string().into_encoded_bytes()),
            None => Err(NotRecycled(format!(
                "it is gone but not in the Recycle Bin, so it cannot be put back ({why})"
            ))
            .into_error()),
        }
    }

    fn trash_children(
        &self,
        path: &[u8],
        expected: &ExpectedEntry,
        _trash_dir: &Path,
        limit: usize,
    ) -> io::Result<TrashedChildren> {
        let parts = parse(path)?;
        let folder = join(&parts, parts.len());
        if !expected.is_dir {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "only a folder can be emptied",
            ));
        }

        // The folder is held, last name too, so it cannot be swapped while its children are acted on.
        let looked = held_walk(path, Hold::All)?;
        check(&looked, expected)?;
        recycle::ensure_recyclable(&parts[0])?;
        let volume = looked.identities.last().map(|identity| identity.volume_serial);

        let (mut left, eligible, more) = list_children(&folder, volume, limit)?;

        let since = now_seconds();
        let paths: Vec<&Path> = eligible.iter().map(|child| child.path.as_path()).collect();
        let moved = if paths.is_empty() {
            Ok(())
        } else {
            trash::delete_all(&paths)
        };

        // Each child is looked at again: still there as it was, or gone.
        let mut gone: Vec<&Child> = Vec::new();
        let mut unknown: Vec<TrashedChild> = Vec::new();
        for child in &eligible {
            let wanted = child.wanted();
            match recycle_state(&child.path, &wanted) {
                Ok(State::Gone) => gone.push(child),
                Ok(State::Still) => left.push(child.reported(ChildTrashed::Left(
                    "it could not be moved to the Recycle Bin (it may be in use by a program)".to_owned(),
                ))),
                Err(error) => {
                    unknown.push(child.reported(ChildTrashed::NotRecycled(unknown_after_move(&error))));
                }
            }
        }

        let mut children = left;
        children.append(&mut unknown);
        if !gone.is_empty() {
            let items = recycle::items();
            let note = match &moved {
                Err(error) => format!("the shell reported: {error}"),
                Ok(()) => "the shell moved it".to_owned(),
            };

            for child in gone {
                let outcome = match &items {
                    Ok(items) => match recycle::find(items, &child.path, since, &child.wanted()) {
                        Some(item) => ChildTrashed::To(item.into_os_string().into_encoded_bytes()),
                        None => ChildTrashed::NotRecycled(format!(
                            "it is gone but not in the Recycle Bin, so it cannot be put back ({note})"
                        )),
                    },
                    Err(error) => ChildTrashed::NotRecycled(format!(
                        "it is gone, but the Recycle Bin could not be read to find it ({error})"
                    )),
                };
                children.push(child.reported(outcome));
            }
        }

        Ok(TrashedChildren { children, more })
    }

    fn restore(&self, trashed: &[u8], expected: &ExpectedEntry, original: &[u8]) -> io::Result<()> {
        let trashed = std::str::from_utf8(trashed)
            .map(PathBuf::from)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let parts = parse(original)?;
        if parts.len() < 2 {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let place = join(&parts, parts.len());
        let folder = join(&parts, parts.len() - 1);

        // The folder it goes back into is a plain folder with no link on its way, and stays so.
        let above = folder.clone().into_os_string().into_encoded_bytes();
        let looked = held_walk(&above, Hold::All)?;
        match looked.entries.last() {
            Some(entry) if entry.meta.kind == EntryKind::Dir => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "the folder it came from is gone, or is a link now",
                ));
            }
        }

        recycle::restore(&trashed, &place, &wanted(expected))
    }

    fn free_bytes(&self, path: &Path) -> io::Result<u64> {
        crate::free_space(path)
    }
}

/// One direct child that may be moved.
struct Child {
    name: Vec<u8>,
    path: PathBuf,
    volume_serial: u64,
    file_index: u64,
    is_dir: bool,
    size: u64,
}

impl Child {
    fn wanted(&self) -> Wanted {
        Wanted {
            volume_serial: self.volume_serial,
            file_index: self.file_index,
            is_dir: self.is_dir,
        }
    }

    fn reported(&self, outcome: ChildTrashed) -> TrashedChild {
        TrashedChild {
            name: self.name.clone(),
            dev: self.volume_serial,
            ino: self.file_index,
            is_dir: self.is_dir,
            allocated: self.size,
            outcome,
        }
    }
}

/// How many children are looked at per child that may be moved, at most: the ones left alone do not count
/// against the limit, but a folder full of links cannot keep one action looking at them for ever.
const LOOKED_AT_PER_MOVE: usize = 4;

/// Looks at the direct children of `folder` until `limit` of them may be moved (or [`LOOKED_AT_PER_MOVE`]
/// times as many are looked at; the rest are counted): those that may be moved, and the ones left alone
/// with the reason. A link, a name Windows would read as another, a child on another volume and one that
/// cannot be looked at are left alone, and do not use up the limit.
fn list_children(
    folder: &Path,
    volume: Option<u64>,
    limit: usize,
) -> io::Result<(Vec<TrashedChild>, Vec<Child>, u64)> {
    let mut left = Vec::new();
    let mut eligible = Vec::new();
    let mut seen = 0usize;
    let mut more = 0u64;
    let ceiling = limit.saturating_mul(LOOKED_AT_PER_MOVE);

    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        if eligible.len() >= limit || seen >= ceiling {
            more += 1;
            continue;
        }
        seen += 1;

        let name = entry.file_name();
        let bytes = name.as_encoded_bytes().to_vec();
        let unplain = |what: &str| TrashedChild {
            name: bytes.clone(),
            dev: 0,
            ino: 0,
            is_dir: false,
            allocated: 0,
            outcome: ChildTrashed::Left(what.to_owned()),
        };

        let Some(text) = name.to_str().filter(|text| is_plain_name(text)) else {
            left.push(unplain("its name is one that Windows would read as another"));
            continue;
        };
        let path = folder.join(text);
        let identity = match identity_of_path(&path) {
            Ok(identity) => identity,
            Err(error) => {
                left.push(unplain(&format!("it could not be looked at ({error})")));
                continue;
            }
        };

        let reported = |what: &str| TrashedChild {
            name: bytes.clone(),
            dev: identity.volume_serial,
            ino: identity.file_index,
            is_dir: identity.is_dir(),
            allocated: 0,
            outcome: ChildTrashed::Left(what.to_owned()),
        };
        if identity.is_reparse_point() {
            left.push(reported("it is a link, which is left alone"));
            continue;
        }
        if volume.is_some_and(|serial| serial != identity.volume_serial) {
            left.push(reported("it is on another volume"));
            continue;
        }

        eligible.push(Child {
            name: bytes.clone(),
            path,
            volume_serial: identity.volume_serial,
            file_index: identity.file_index,
            is_dir: identity.is_dir(),
            size: if identity.is_dir() { 0 } else { identity.size },
        });
    }

    Ok((left, eligible, more))
}

/// What is reported for an entry that the shell was asked to move and that could not be looked at
/// afterwards: neither failed nor trashed, and never offered for restoring.
fn unknown_after_move(error: &io::Error) -> String {
    format!(
        "it may have been moved to the Recycle Bin, look there before cleaning again (it could not be checked afterwards: {error})"
    )
}

/// Whether the entry planned is still at `path`.
enum State {
    Still,
    Gone,
}

/// Looks at `path` again after the shell was asked to move it. Still the planned entry: not moved. Not
/// there, or another entry now: moved (what is there now is not ours). Anything else cannot be told.
fn recycle_state(path: &Path, wanted: &Wanted) -> io::Result<State> {
    match identity_of_path(path) {
        Ok(identity) if wanted.is(&identity) => Ok(State::Still),
        Ok(_) => Ok(State::Gone),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(State::Gone),
        Err(error) => Err(error),
    }
}

fn wanted(expected: &ExpectedEntry) -> Wanted {
    Wanted {
        volume_serial: expected.dev,
        file_index: expected.ino,
        is_dir: expected.is_dir,
    }
}

/// [`walk`] in its own thread under the probe's time limit, handing the held handles back.
fn held_walk(path: &[u8], hold: Hold) -> io::Result<Walk> {
    let path = path.to_vec();

    timed(move || walk(&path, hold))
}

/// What the plan saw against what is there now. Nothing is touched when they differ.
fn check(looked: &Walk, expected: &ExpectedEntry) -> io::Result<()> {
    let entries = &looked.entries;
    let Some(now) = entries.last() else {
        return Err(changed("the path is empty"));
    };

    // A drive root, or a scan root or something above it, is never acted on.
    if entries.len() <= 1 || entries.len() <= expected.root_depth {
        return Err(changed("it is not below the scan root"));
    }
    if entries
        .get(expected.root_depth..)
        .is_none_or(|below| below.iter().any(|entry| entry.meta.is_mount_root))
    {
        return Err(changed("a mount lies on the way"));
    }

    match (now.meta.kind, expected.is_dir) {
        (EntryKind::Dir, true) | (EntryKind::File, false) => {}
        (EntryKind::Symlink, _) => return Err(changed("it is a link now")),
        (EntryKind::Dir, false) => return Err(changed("it is now a folder")),
        (EntryKind::File, true) => return Err(changed("it is now a file")),
        (EntryKind::Other, _) => return Err(changed("it is neither a file nor a folder")),
    }
    if (now.meta.dev, now.meta.ino) != (expected.dev, expected.ino) {
        return Err(changed("another entry now has its name"));
    }
    if matches!((now.birth, expected.birth), (Some(now), Some(planned)) if now != planned) {
        return Err(changed("another entry now has its name"));
    }

    Ok(())
}

fn changed(what: &str) -> io::Error {
    io::Error::other(format!("changed since the plan: {what}"))
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}
