//! A `DirSource` on plain `std::fs`. Works on every OS and is the reference the fast readers are
//! tested against.
//!
//! It never follows symlinks below the root (the root path itself is resolved). On unix it reports real
//! block counts, device/inode/link counts; elsewhere allocated falls back to the length and hardlinks
//! are not detected. A directory on another device than its parent is flagged as a mount root, which is
//! the best a portable reader can do (the Linux reader uses the kernel's mount-root flag instead).
//!
//! `one_file_system` is therefore the classic `st_dev` comparison: every directory on another device
//! than its parent is left alone. On btrfs that stops at every subvolume (each has its own `st_dev`),
//! so scanning `/` of a machine with `/home` on a separate subvolume leaves `/home` out. The Linux
//! reader knows better (it compares superblocks from the mount table); this one cannot without
//! OS-specific knowledge, and it is only used where no faster reader exists.
//!
//! # Races
//!
//! This reader works on joined paths, so a directory replaced by a symlink between the listing of its
//! parent and the listing of the directory itself would be followed by `fs::read_dir`. Right before a
//! listing the entry is therefore checked with `symlink_metadata` and refused unless it is still a real
//! directory (reported as an error). The check and the listing are two syscalls: a swap in that gap, or
//! one of an ancestor, is still possible. Closing it needs descriptor-relative opens (`openat`), which is
//! what the Linux reader does; a portable `std` cannot.

pub(crate) mod meta;

// The Windows exclusion rules are plain data and name matching: their tests run on every OS.
#[cfg(all(test, not(windows)))]
#[path = "../windows/exclusions.rs"]
mod windows_exclusions;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use platter_core::{
    DirListing, DirSource, EntryKind, Lease, MountFacts, RawEntry, RootInfo, SkipReason, SourceOptions,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct GenericDirSource;

/// Path of a directory plus the device it lives on (to spot mount boundaries below it).
#[derive(Debug)]
pub struct GenericHandle {
    path: PathBuf,
    dev: u64,
    /// The scan root, whose path may legitimately be a symlink; everything below must not be.
    is_root: bool,
    one_file_system: bool,
}

impl DirSource for GenericDirSource {
    type Handle = GenericHandle;

    fn open_root(&self, path: &Path, options: SourceOptions) -> io::Result<RootInfo<GenericHandle>> {
        let meta = meta::entry_meta(&fs::metadata(path)?);

        Ok(RootInfo {
            handle: GenericHandle {
                path: path.to_path_buf(),
                dev: meta.dev,
                is_root: true,
                one_file_system: options.one_file_system,
            },
            meta,
            mount_id: None,
            warnings: Vec::new(),
        })
    }

    fn read_dir(&self, dir: &GenericHandle, lease: &mut Lease<'_>) -> io::Result<DirListing<GenericHandle>> {
        if !dir.is_root {
            ensure_real_directory(&dir.path)?;
        }

        let mut listing = DirListing::default();

        for entry in fs::read_dir(&dir.path)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    listing.errors += 1;
                    continue;
                }
            };

            // `DirEntry::metadata` does not traverse symlinks (lstat semantics).
            let mut meta = match entry.metadata() {
                Ok(metadata) => meta::entry_meta(&metadata),
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => {
                    listing.errors += 1;
                    continue;
                }
            };

            let file_name = entry.file_name();

            // Room is reserved before the entry is materialised: a refusal ends the listing, because
            // the scan is over (it has hit a limit).
            if !lease.admit(file_name.len(), meta.kind == EntryKind::Dir) {
                break;
            }

            let mut child = None;
            let mut skipped = None;
            if meta.kind == EntryKind::Dir {
                meta.is_mount_root = meta.dev != dir.dev;
                if !(meta.is_mount_root && dir.one_file_system) {
                    child = Some(GenericHandle {
                        path: dir.path.join(&file_name),
                        dev: meta.dev,
                        is_root: false,
                        one_file_system: dir.one_file_system,
                    });
                } else {
                    // Another device than the parent is all this reader can tell: another disk.
                    skipped = Some(SkipReason::OtherDisk);
                }
            }

            listing.entries.push(RawEntry {
                name: file_name.as_encoded_bytes().to_vec(),
                meta,
                child,
                skipped,
                mount: MountFacts::default(),
            });
        }

        Ok(listing)
    }
}

/// Fails unless `path` is, right now, a directory and not a symlink to one (`fs::read_dir` follows it).
fn ensure_real_directory(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_dir() {
        return Ok(());
    }

    Err(io::Error::other(
        "the directory was replaced while the scan was running",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{MetadataExt, symlink};

    use platter_core::EntryBudget;

    use super::*;

    fn names<H>(listing: &DirListing<H>) -> Vec<String> {
        let mut names: Vec<String> = listing
            .entries
            .iter()
            .map(|entry| String::from_utf8_lossy(&entry.name).into_owned())
            .collect();
        names.sort();
        names
    }

    /// Lists the child `sub` of a fresh tree after `sub` was replaced by a symlink to `elsewhere`.
    fn list_child_after_swap() -> (io::Result<DirListing<GenericHandle>>, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/inside.txt"), b"x").unwrap();
        fs::create_dir_all(dir.path().join("elsewhere")).unwrap();
        fs::write(dir.path().join("elsewhere/secret.txt"), b"y").unwrap();

        let source = GenericDirSource;
        let root = source.open_root(dir.path(), SourceOptions::default()).unwrap();
        let mut listing = source
            .read_dir(&root.handle, &mut EntryBudget::unlimited().lease())
            .unwrap();
        let sub = listing
            .entries
            .iter_mut()
            .find(|entry| entry.name == b"sub")
            .and_then(|entry| entry.child.take())
            .expect("sub is listed as a directory");

        fs::remove_dir_all(dir.path().join("sub")).unwrap();
        symlink(dir.path().join("elsewhere"), dir.path().join("sub")).unwrap();

        let swapped = fs::read_dir(dir.path().join("sub"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        (
            source.read_dir(&sub, &mut EntryBudget::unlimited().lease()),
            swapped,
        )
    }

    #[test]
    fn std_read_dir_follows_a_symlink_in_the_last_component() {
        let (_, followed) = list_child_after_swap();

        assert_eq!(
            followed,
            ["secret.txt"],
            "the premise: a plain read_dir would walk out of the tree"
        );
    }

    #[test]
    fn a_directory_swapped_for_a_symlink_is_refused_not_followed() {
        let (result, _) = list_child_after_swap();

        let error = result.expect_err("the symlink must not be listed");
        assert!(error.to_string().contains("replaced"), "{error}");
    }

    #[test]
    fn the_scan_root_may_be_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        fs::write(dir.path().join("real/f"), b"x").unwrap();
        symlink(dir.path().join("real"), dir.path().join("link")).unwrap();

        let source = GenericDirSource;
        let root = source
            .open_root(&dir.path().join("link"), SourceOptions::default())
            .unwrap();

        assert_eq!(
            names(
                &source
                    .read_dir(&root.handle, &mut EntryBudget::unlimited().lease())
                    .unwrap()
            ),
            ["f"]
        );
    }

    #[test]
    fn one_file_system_withholds_directories_on_another_device() {
        let Ok(proc) = fs::metadata("/proc") else {
            return;
        };
        if fs::metadata("/").map(|root| root.dev()).ok() == Some(proc.dev()) {
            return;
        }

        let source = GenericDirSource;
        for (one_file_system, entered) in [(true, false), (false, true)] {
            let root = source
                .open_root(Path::new("/"), SourceOptions { one_file_system })
                .unwrap();
            let listing = source
                .read_dir(&root.handle, &mut EntryBudget::unlimited().lease())
                .unwrap();
            let entry = listing
                .entries
                .iter()
                .find(|entry| entry.name == b"proc")
                .unwrap();

            assert!(entry.meta.is_mount_root);
            assert_eq!(
                entry.child.is_some(),
                entered,
                "one_file_system = {one_file_system}"
            );
        }
    }
}
