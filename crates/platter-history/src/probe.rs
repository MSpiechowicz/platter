//! What became of the entries a run moved to the trash: still there, or not.

use std::collections::HashMap;
use std::io::ErrorKind;

use platter_core::{EntryKind, EntryProbe};
use serde::Serialize;

use crate::store::{StoredCleanup, StoredItem};

/// How many entries one call looks at. A run with more is `notChecked` beyond it.
pub const CHECK_CAP: usize = 4096;

/// The state of a run, as shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// At least one entry is in the trash and can be put back.
    InTrash,
    /// Every entry was put back.
    Restored,
    /// Not put back, and no longer in the trash (it was emptied or moved).
    NoLongerInTrash,
    /// Nothing went to the trash: what the run did cannot be undone.
    Permanent,
    /// Too many entries to look at, or one could not be looked at. Restore is still offered.
    NotChecked,
}

/// A run's status and the bytes Restore would put back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunStatus {
    pub status: Status,
    pub restorable: u64,
}

/// What looking at the runs found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The status of every run, in the order given.
    pub statuses: Vec<RunStatus>,
    /// The stored ids of the entries found gone from the trash: nothing needs their paths any more. An
    /// entry is gone only when the trash folder itself (the `files` folder it was in) was there to look in:
    /// with that folder missing (an unmounted disk) the entry shows as no longer in the trash and is not
    /// named here, so its paths are kept.
    pub gone: Vec<i64>,
}

/// The status of every run, in the order given. `cap` entries are looked at in all, newest run first
/// as given, so a long history costs a bounded number of probes.
pub fn statuses(cleanups: &[StoredCleanup], probe: &impl EntryProbe, cap: usize) -> Vec<RunStatus> {
    check(cleanups, probe, cap).statuses
}

/// As [`statuses`], and says which entries it found gone (at most `cap`).
pub fn check(cleanups: &[StoredCleanup], probe: &impl EntryProbe, cap: usize) -> Checked {
    let mut left = cap;
    let mut gone = Vec::new();
    let mut folders = Folders::default();
    let statuses = cleanups
        .iter()
        .map(|cleanup| status_and_gone(cleanup, probe, &mut left, &mut gone, &mut folders))
        .collect();

    Checked { statuses, gone }
}

/// The status of one run from the entries it moved to the trash. Every entry that is looked at takes one
/// from `left`. The run holds the entries not put back (and not already known to be gone) that were
/// loaded: the ones it has more than that are not looked at.
pub fn status_of(cleanup: &StoredCleanup, probe: &impl EntryProbe, left: &mut usize) -> RunStatus {
    status_and_gone(cleanup, probe, left, &mut Vec::new(), &mut Folders::default())
}

/// As [`status_of`]; the ids of the entries found gone are added to `gone`.
fn status_and_gone(
    cleanup: &StoredCleanup,
    probe: &impl EntryProbe,
    left: &mut usize,
    gone: &mut Vec<i64>,
    folders: &mut Folders,
) -> RunStatus {
    if cleanup.item_count == 0 {
        return RunStatus {
            status: Status::Permanent,
            restorable: 0,
        };
    }
    if cleanup.unrestored == 0 {
        return RunStatus {
            status: Status::Restored,
            restorable: 0,
        };
    }

    let mut in_trash = 0;
    let mut restorable = 0u64;
    // Entries already known to be gone are not looked at again, and are not unknown.
    let waiting = cleanup.unrestored.saturating_sub(cleanup.gone);
    let mut unknown = waiting > cleanup.items.len() as u64;

    for item in &cleanup.items {
        if *left == 0 {
            unknown = true;
            continue;
        }
        *left -= 1;

        match look(probe, folders, item) {
            Looked::InTrash => {
                in_trash += 1;
                restorable = restorable.saturating_add(item.allocated);
            }
            Looked::Gone => gone.push(item.id),
            Looked::Missing => {}
            Looked::Unknown => unknown = true,
        }
    }

    let status = if in_trash > 0 {
        Status::InTrash
    } else if unknown {
        Status::NotChecked
    } else {
        Status::NoLongerInTrash
    };

    RunStatus { status, restorable }
}

enum Looked {
    InTrash,
    /// Not in the trash, which was there to look in: nothing needs its paths.
    Gone,
    /// Not found, but the trash folder is not there either (a disk that is not mounted), or found with the
    /// same inode under another device number (a disk plugged in again): it shows as no longer in the
    /// trash, and its paths are kept in case the trash comes back or the entry is still there.
    Missing,
    Unknown,
}

/// An item is in the trash while the entry with its identity is at its trash path. It is gone only when
/// the folder it was in is present and the entry is missing from it or is another one. The same inode
/// on another device number is not told apart from the same entry on a disk that came back differently.
fn look(probe: &impl EntryProbe, folders: &mut Folders, item: &StoredItem) -> Looked {
    match probe.probe(&item.trashed) {
        Ok(found) => match found.last() {
            Some(entry) if (entry.meta.dev, entry.meta.ino) == item.identity => Looked::InTrash,
            // The inode is the same on another device number: a disk plugged in again under a new
            // number, so the entry is probably still there. Shown as not in the trash, paths kept.
            Some(entry) if entry.meta.ino == item.identity.1 => Looked::Missing,
            _ => Looked::Gone,
        },
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            if folders.is_there(probe, &item.trashed) {
                Looked::Gone
            } else {
                Looked::Missing
            }
        }
        Err(_) => Looked::Unknown,
    }
}

/// The trash folders looked at so far, so entries in the same folder cost one look at it between them
/// (the number of looks stays bounded by the cap on entries, plus one per distinct folder).
#[derive(Default)]
struct Folders(HashMap<Vec<u8>, bool>);

impl Folders {
    /// Whether the folder that holds `path` is there, as a folder.
    fn is_there(&mut self, probe: &impl EntryProbe, path: &[u8]) -> bool {
        let Some(slash) = path.iter().rposition(|byte| *byte == b'/') else {
            return false;
        };
        // A path directly under `/` has no folder worth the name: the root is always there.
        if slash == 0 {
            return false;
        }

        let folder = &path[..slash];
        if let Some(known) = self.0.get(folder) {
            return *known;
        }
        let there = match probe.probe(folder) {
            Ok(found) => found
                .last()
                .is_some_and(|entry| entry.meta.kind == EntryKind::Dir),
            Err(_) => false,
        };
        self.0.insert(folder.to_vec(), there);

        there
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io;

    use platter_core::{EntryKind, EntryMeta, ProbedEntry};

    use super::*;

    /// A disk that is a table: the path and the identity of what is there.
    struct Disk(HashMap<Vec<u8>, io::Result<(u64, u64)>>);

    impl EntryProbe for Disk {
        fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
            match self.0.get(path) {
                None => Err(io::Error::from(io::ErrorKind::NotFound)),
                Some(Err(error)) => Err(io::Error::from(error.kind())),
                Some(Ok((dev, ino))) => Ok(vec![ProbedEntry {
                    meta: EntryMeta {
                        kind: if path == b"/trash" {
                            EntryKind::Dir
                        } else {
                            EntryKind::File
                        },
                        allocated: 0,
                        apparent: 0,
                        mtime: 0,
                        dev: *dev,
                        ino: *ino,
                        nlink: 1,
                        is_mount_root: false,
                    },
                    mount_id: None,
                    birth: None,
                }]),
            }
        }
    }

    fn item(name: &str, ino: u64, allocated: u64) -> StoredItem {
        StoredItem {
            id: ino as i64,
            original: format!("/home/u/{name}").into_bytes(),
            trashed: format!("/trash/{name}").into_bytes(),
            identity: (1, ino),
            is_dir: false,
            allocated,
        }
    }

    /// A run that has `items` not put back, and `put_back` more that are.
    fn run(items: Vec<StoredItem>, put_back: u64) -> StoredCleanup {
        StoredCleanup {
            id: 1,
            at: 0,
            freed: 0,
            trashed: 0,
            what: Vec::new(),
            methods: Vec::new(),
            item_count: items.len() as u64 + put_back,
            unrestored: items.len() as u64,
            gone: 0,
            items,
        }
    }

    /// A disk with a trash folder (`/trash`) that holds `entries`.
    fn disk(entries: &[(&str, io::Result<(u64, u64)>)]) -> Disk {
        let mut disk = disk_without_trash(entries);
        disk.0.insert(b"/trash".to_vec(), Ok((1, 1000)));

        disk
    }

    /// A disk where `/trash` is not there (its disk is not mounted), whatever `entries` say.
    fn disk_without_trash(entries: &[(&str, io::Result<(u64, u64)>)]) -> Disk {
        Disk(
            entries
                .iter()
                .map(|(name, found)| {
                    let found = match found {
                        Ok(identity) => Ok(*identity),
                        Err(error) => Err(io::Error::from(error.kind())),
                    };
                    (format!("/trash/{name}").into_bytes(), found)
                })
                .collect(),
        )
    }

    #[test]
    fn a_run_that_trashed_nothing_is_permanent() {
        let status = status_of(&run(Vec::new(), 0), &disk(&[]), &mut 10);

        assert_eq!(status.status, Status::Permanent);
    }

    #[test]
    fn an_entry_with_its_identity_in_the_trash_can_be_restored() {
        let run = run(vec![item("a", 7, 100), item("b", 8, 50)], 0);

        let status = status_of(&run, &disk(&[("a", Ok((1, 7))), ("b", Ok((1, 8)))]), &mut 10);

        assert_eq!((status.status, status.restorable), (Status::InTrash, 150));
    }

    #[test]
    fn an_entry_that_is_gone_or_replaced_is_no_longer_in_the_trash() {
        let run = run(vec![item("a", 7, 100), item("b", 8, 50)], 0);

        // "a" is not there, and another file took the name of "b".
        let status = status_of(&run, &disk(&[("b", Ok((1, 99)))]), &mut 10);

        assert_eq!((status.status, status.restorable), (Status::NoLongerInTrash, 0));
    }

    #[test]
    fn an_entry_with_the_same_inode_on_another_device_is_kept_not_named_gone() {
        let cleanups = [run(vec![item("a", 7, 100)], 0)];

        // A disk plugged in again under another device number: the entry is still in its trash.
        let checked = check(&cleanups, &disk(&[("a", Ok((2, 7)))]), 10);

        assert_eq!(
            checked.statuses[0].status,
            Status::NoLongerInTrash,
            "shown as before"
        );
        assert_eq!(checked.statuses[0].restorable, 0);
        assert!(checked.gone.is_empty(), "so its paths are kept");

        // Another inode on another device is another file: gone.
        let checked = check(&cleanups, &disk(&[("a", Ok((2, 8)))]), 10);

        assert_eq!(checked.gone, [7]);
    }

    #[test]
    fn one_entry_left_in_the_trash_is_enough_and_only_it_counts() {
        let run = run(vec![item("a", 7, 100), item("b", 8, 50)], 0);

        let status = status_of(&run, &disk(&[("b", Ok((1, 8)))]), &mut 10);

        assert_eq!((status.status, status.restorable), (Status::InTrash, 50));
    }

    #[test]
    fn a_run_with_every_entry_put_back_is_restored_and_costs_no_look() {
        let run = run(Vec::new(), 2);
        let mut left = 0;

        let status = status_of(&run, &disk(&[("a", Ok((1, 7)))]), &mut left);

        assert_eq!(status.status, Status::Restored);
    }

    #[test]
    fn entries_put_back_do_not_hide_the_ones_still_in_the_trash() {
        let run = run(vec![item("b", 8, 50)], 3);

        let status = status_of(&run, &disk(&[("b", Ok((1, 8)))]), &mut 10);

        assert_eq!((status.status, status.restorable), (Status::InTrash, 50));
    }

    #[test]
    fn beyond_the_cap_an_entry_is_not_checked() {
        let run = run(vec![item("a", 7, 100), item("b", 8, 50)], 0);
        let mut left = 1;

        // Only "a" is looked at, and it is gone: "b" is unknown.
        let status = status_of(&run, &disk(&[]), &mut left);

        assert_eq!(status.status, Status::NotChecked);
        assert_eq!(left, 0);

        // An entry that was looked at and is in the trash still shows the run as in the trash.
        let mut left = 1;
        let status = status_of(&run, &disk(&[("a", Ok((1, 7)))]), &mut left);
        assert_eq!((status.status, status.restorable), (Status::InTrash, 100));
    }

    #[test]
    fn entries_that_were_not_loaded_are_not_checked() {
        // Three entries wait, and the load had room for one: it is gone, and two are unknown.
        let mut run = run(vec![item("a", 7, 100)], 0);
        run.item_count = 3;
        run.unrestored = 3;

        let status = status_of(&run, &disk(&[]), &mut 10);

        assert_eq!(status.status, Status::NotChecked);
    }

    #[test]
    fn the_cap_is_shared_by_all_runs() {
        let cleanups = [run(vec![item("a", 1, 10)], 0), run(vec![item("b", 2, 10)], 0)];

        let all = statuses(&cleanups, &disk(&[("a", Ok((1, 1))), ("b", Ok((1, 2)))]), 1);

        assert_eq!(all[0].status, Status::InTrash);
        assert_eq!(all[1].status, Status::NotChecked);
    }

    #[test]
    fn a_probe_that_fails_for_another_reason_is_not_checked() {
        let run = run(vec![item("a", 7, 100)], 0);
        let failing = disk(&[("a", Err(io::Error::from(io::ErrorKind::PermissionDenied)))]);

        let status = status_of(&run, &failing, &mut 10);

        assert_eq!(status.status, Status::NotChecked);
    }

    #[test]
    fn the_entries_found_gone_are_named_and_the_ones_in_the_trash_are_not() {
        let cleanups = [run(
            vec![item("a", 7, 100), item("b", 8, 50), item("c", 9, 10)],
            0,
        )];
        // "a" is in the trash, "b" is not there, and another file took the name of "c".
        let on_disk = disk(&[("a", Ok((1, 7))), ("c", Ok((1, 99)))]);

        let checked = check(&cleanups, &on_disk, 10);

        assert_eq!(checked.gone, [8, 9]);
        assert_eq!(checked.statuses[0].status, Status::InTrash);
    }

    #[test]
    fn an_entry_whose_trash_folder_is_not_there_shows_as_gone_but_is_not_named_gone() {
        let cleanups = [run(vec![item("a", 7, 100)], 0)];

        // The disk with the trash is not mounted.
        let checked = check(&cleanups, &disk_without_trash(&[]), 10);

        assert_eq!(checked.statuses[0].status, Status::NoLongerInTrash);
        assert!(checked.gone.is_empty(), "so its paths are kept");

        // The folder is there and the entry is not.
        let checked = check(&cleanups, &disk(&[]), 10);

        assert_eq!(checked.statuses[0].status, Status::NoLongerInTrash);
        assert_eq!(checked.gone, [7]);
    }

    #[test]
    fn an_entry_that_could_not_be_looked_at_or_was_not_reached_is_not_named_gone() {
        let cleanups = [run(vec![item("a", 7, 100), item("b", 8, 50)], 0)];
        let failing = disk(&[("a", Err(io::Error::from(io::ErrorKind::PermissionDenied)))]);

        // "a" fails to be looked at, and the cap leaves no look for "b".
        let checked = check(&cleanups, &failing, 1);

        assert!(checked.gone.is_empty());
    }

    #[test]
    fn a_run_whose_entries_are_all_known_gone_is_no_longer_in_the_trash_and_costs_no_look() {
        let mut run = run(Vec::new(), 1);
        run.item_count = 3;
        run.unrestored = 2;
        run.gone = 2;
        let mut left = 0;

        let status = status_of(&run, &disk(&[]), &mut left);

        assert_eq!((status.status, status.restorable), (Status::NoLongerInTrash, 0));
    }

    #[test]
    fn entries_known_gone_do_not_hide_the_ones_not_loaded() {
        // Four wait, one is known gone and one is loaded: two are unknown.
        let mut run = run(vec![item("a", 7, 100)], 0);
        run.item_count = 4;
        run.unrestored = 4;
        run.gone = 1;

        let status = status_of(&run, &disk(&[]), &mut 10);

        assert_eq!(status.status, Status::NotChecked);
    }
}
