//! The budget of descriptors kept open as parents of pending children is one counter for the whole
//! process, shared by every scan in it (the macOS reader uses the same one as the Linux reader).
//!
//! This is its own test binary because the counter is process-wide: other scans running in the same
//! process at the same time would change the numbers.

#![cfg(target_os = "macos")]

use std::fs;

use platter_core::{DirListing, DirSource, EntryBudget, SourceOptions};
use platter_platform::macos::MacDirSource;
use platter_platform::macos::reader::{MacHandle, descriptors_held};

const BUDGET: usize = 3;

fn take_children(listing: &mut DirListing<MacHandle>) -> Vec<MacHandle> {
    listing
        .entries
        .iter_mut()
        .filter_map(|entry| entry.child.take())
        .collect()
}

#[test]
fn concurrent_scans_share_one_descriptor_budget() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c", "d"] {
        fs::create_dir_all(dir.path().join(name).join("x")).unwrap();
    }
    let source = MacDirSource::new().with_descriptor_budget(BUDGET);
    assert_eq!(descriptors_held(), 0);

    // Two scans of the same tree, as the desktop app can have several at once.
    let mut scans = Vec::new();
    for _ in 0..2 {
        let root = source.open_root(dir.path(), SourceOptions::default()).unwrap();
        let mut listing = source
            .read_dir(&root.handle, &mut EntryBudget::unlimited().lease())
            .unwrap();
        scans.push((root.handle, take_children(&mut listing)));
    }

    // Reading a directory keeps its descriptor open for the children it lists: while the listings are
    // alive the descriptors stay claimed. Read every first-level directory of both scans.
    let mut alive = Vec::new();
    let mut highest = 0;
    for (_, children) in &scans {
        for handle in children {
            alive.push(
                source
                    .read_dir(handle, &mut EntryBudget::unlimited().lease())
                    .unwrap(),
            );
            highest = highest.max(descriptors_held());
        }
    }

    assert_eq!(
        highest, BUDGET,
        "eight directories were read, only {BUDGET} may keep a descriptor, across both scans together"
    );
    assert_eq!(descriptors_held(), BUDGET);

    // The directories that got no slot still list fine and hand their children a way to be opened.
    for listing in &mut alive {
        for handle in take_children(listing) {
            assert_eq!(
                source
                    .read_dir(&handle, &mut EntryBudget::unlimited().lease())
                    .unwrap()
                    .entries
                    .len(),
                0
            );
        }
    }

    drop(alive);
    assert_eq!(descriptors_held(), 0, "every slot is given back");
}
