//! Finding and restoring items in the Recycle Bin (W2), through `trash::os_limited`.
//!
//! What the crate gives for an item (`trash` 5.4, read from its source): `id` is the shell's desktop
//! absolute parsing name, which for an item of an NTFS volume is the path of the real file `$R...` inside
//! `$Recycle.Bin\<sid>`; `original_parent` and `name` are where it was and what it was called (the name
//! with its extension), and `time_deleted` the whole seconds of the deletion time. Nothing is taken from
//! those on trust: an item is only ever named by opening its `id` and comparing the file's identity (volume
//! serial and file index, which a move within a volume keeps) with what was planned.
//!
//! `trash::delete` moves with `FOF_ALLOWUNDO` and `FOF_WANTNUKEWARNING` without a user interface. For an
//! item the bin will not take (too large for it, a drive set to delete at once) the shell raises a
//! question that nobody sees and waits for it, so that is never left to the shell: `bin_guard` reads the
//! bin's settings first and refuses such an item, and the shell call itself has a time limit
//! (`time_limit`). Should the shell delete one anyway, the item is not found here and the caller reports
//! it as gone but not recycled.
//!
//! `restore_all` refuses an item whose old path exists (`RestoreCollision`), but that check and the move
//! that follows it are two steps of the shell, and the move (`MoveItem` with `FOF_NO_UI`) has no
//! rename-on-collision or no-replace flag. So the guarantee is only this: [`restore`] looks at the old name
//! first and refuses when it is taken, and so does the crate, right before it moves. A name that is taken
//! in the instants between that check and the move is not protected (Linux has `RENAME_NOREPLACE` for
//! this, the shell has nothing like it). The identity at the old place is compared after.

use std::io;
use std::path::{Path, PathBuf};

use sysinfo::Disks;
use trash::TrashItem;

use super::identity::{FileIdentity, identity_of_path};

/// How far before the start of an action an item's deletion time may lie and still be that action's:
/// the shell's time has whole seconds, and the clocks involved are the same machine's.
const DELETED_SLACK_SECONDS: i64 = 120;

/// What an item has to be to be the one that was planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Wanted {
    pub volume_serial: u64,
    pub file_index: u64,
    pub is_dir: bool,
}

impl Wanted {
    pub fn is(&self, identity: &FileIdentity) -> bool {
        identity.volume_serial == self.volume_serial
            && identity.file_index == self.file_index
            && identity.is_dir() == self.is_dir
            && !identity.is_reparse_point()
    }
}

/// Refuses a volume the Recycle Bin is not known to work on: only a fixed drive with NTFS or ReFS.
/// Anything else (removable, network, FAT, a drive `sysinfo` does not list) is `Unsupported`, so the
/// shell is never asked to "recycle" where it would delete for good. The answer is the volume's total size
/// in bytes, which the default size of the bin is a share of.
pub(super) fn ensure_recyclable(drive_root: &str) -> io::Result<u64> {
    let refuse = || {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "the Recycle Bin is only used on fixed drives with NTFS or ReFS",
        )
    };

    let disks = Disks::new_with_refreshed_list().map_err(io::Error::other)?;
    let disk = disks
        .iter()
        .find(|disk| disk.mount_point().as_os_str().eq_ignore_ascii_case(drive_root))
        .ok_or_else(refuse)?;

    let file_system = disk.file_system().to_string_lossy();
    let known = file_system.eq_ignore_ascii_case("NTFS") || file_system.eq_ignore_ascii_case("ReFS");
    if disk.is_removable() || !known {
        return Err(refuse());
    }

    Ok(disk.total_space())
}

/// Every item in the Recycle Bin. It can be a long list: ask once for a whole run.
pub(super) fn items() -> io::Result<Vec<TrashItem>> {
    trash::os_limited::list().map_err(io::Error::other)
}

/// The path of the item among `items` that was `original` and now is the planned entry (`wanted`),
/// deleted not long before `since` (unix seconds) or later. `None` when there is none: it was deleted for
/// good, or the shell put it somewhere that cannot be told.
pub(super) fn find(items: &[TrashItem], original: &Path, since: i64, wanted: &Wanted) -> Option<PathBuf> {
    items
        .iter()
        .filter(|item| item.time_deleted.saturating_add(DELETED_SLACK_SECONDS) >= since)
        .filter(|item| same_path(&item.original_path(), original))
        .find(|item| is_wanted(Path::new(&item.id), wanted))
        .map(|item| PathBuf::from(&item.id))
}

/// Whether the entry at `path` is the planned one.
fn is_wanted(path: &Path, wanted: &Wanted) -> bool {
    identity_of_path(path).is_ok_and(|identity| wanted.is(&identity))
}

/// Windows file names are not case sensitive: two spellings of a path are the same place. NTFS compares
/// by an upcase table that maps one UTF-16 unit to one, so each character is compared by its Unicode
/// uppercase when that is a single character (`ß`, whose uppercase is two, stands for itself). The
/// standard library's Unicode data is not the volume's table, so a near spelling can match here that the
/// volume tells apart: every match is checked by identity afterwards, so that costs nothing. Text that
/// is not UTF-8 is compared with ASCII case folded only.
pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    let (Some(a_text), Some(b_text)) = (a.to_str(), b.to_str()) else {
        return a.as_os_str().eq_ignore_ascii_case(b.as_os_str());
    };

    a_text.chars().map(upcase).eq(b_text.chars().map(upcase))
}

/// The upcase of one character: its uppercase when that is one character, else itself.
fn upcase(c: char) -> char {
    let mut upper = c.to_uppercase();

    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

/// Puts the item at `trashed` back at `original`, when it is the entry that was planned (`wanted`) and
/// nothing has taken its old name: a name that is taken when this looks (and when the shell looks, right
/// before it moves) is `AlreadyExists`. The check is not atomic with the move, see the module's text.
/// The caller has checked that the way to `original` has no link on it and keeps it from changing meanwhile.
pub(super) fn restore(trashed: &Path, original: &Path, wanted: &Wanted) -> io::Result<()> {
    if !is_wanted(trashed, wanted) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the Recycle Bin no longer holds the entry that was moved there",
        ));
    }
    if original.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "something else has taken its old name",
        ));
    }

    // The item as the shell lists it: found by its path, then by what it is, never by name alone.
    let item = items()?
        .into_iter()
        .find(|item| same_path(Path::new(&item.id), trashed))
        .filter(|item| same_path(&item.original_path(), original))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "the Recycle Bin does not list it where it was moved",
            )
        })?;

    match trash::os_limited::restore_all([item]) {
        Ok(()) => {}
        Err(trash::Error::RestoreCollision { .. }) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "something else has taken its old name",
            ));
        }
        Err(error) => return Err(io::Error::other(error)),
    }

    // What is at the old place is what was planned.
    match identity_of_path(original) {
        Ok(identity) if wanted.is(&identity) => Ok(()),
        Ok(_) => Err(io::Error::other(
            "what was put back is not the entry that was moved",
        )),
        Err(error) => Err(error),
    }
}
