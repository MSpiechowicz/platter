//! The Recycle Bin remover and the probe on a real Windows volume. Run in CI only (`windows-latest`): they
//! use the Recycle Bin of the runner's system drive and a tiny temporary tree.

#![cfg(windows)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use platter_core::ports::{ChildTrashed, not_recycled};
use platter_core::{EntryKind, EntryProbe, EntryRemover, ExpectedEntry};
use platter_platform::windows::{WindowsEntryProbe, WindowsEntryRemover};

/// A temporary folder named the way the program names it (long names, never `\\?\`).
fn scratch() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dunce::canonicalize(dir.path()).unwrap();

    (dir, path)
}

fn bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}

/// What a plan made now would have seen at `path`, for a scan root `root`.
fn expected(path: &Path, root: &Path) -> ExpectedEntry {
    let found = WindowsEntryProbe.probe(&bytes(path)).unwrap();
    let last = found.last().unwrap();

    ExpectedEntry {
        dev: last.meta.dev,
        ino: last.meta.ino,
        mount_id: None,
        birth: last.birth,
        is_dir: last.meta.kind == EntryKind::Dir,
        root_depth: WindowsEntryProbe.probe(&bytes(root)).unwrap().len(),
    }
}

static BIN: Mutex<()> = Mutex::new(());

/// Every test that uses the Recycle Bin holds this, so that one that changes the bin's settings is alone
/// with it (the settings are the user's, per volume, and apply to every process).
fn bin() -> MutexGuard<'static, ()> {
    BIN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A value of the Recycle Bin's per-volume settings in the registry (`NukeOnDelete`: delete at once,
/// `MaxCapacity`: the most the bin holds, in MB), set for a test and put back as it was when dropped, also
/// when the test fails.
struct BinSetting {
    key: String,
    name: &'static str,
    /// The value before, as `reg` writes it (`0x0`); `None` when there was none.
    before: Option<String>,
}

impl BinSetting {
    fn set(on: &Path, name: &'static str, value: u32) -> Self {
        let drive: String = on.to_string_lossy().chars().take(2).collect();
        let key = format!(
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\BitBucket\\Volume\\{}",
            volume_guid(&drive)
        );
        let before = reg(&["query", &key, "/v", name])
            .filter(|text| text.contains(name))
            .and_then(|text| text.split_whitespace().last().map(str::to_owned));

        let done = reg(&[
            "add",
            &key,
            "/v",
            name,
            "/t",
            "REG_DWORD",
            "/d",
            &value.to_string(),
            "/f",
        ]);
        assert!(done.is_some(), "reg add {key} {name}");

        Self { key, name, before }
    }
}

impl Drop for BinSetting {
    fn drop(&mut self) {
        let _ = match &self.before {
            Some(value) => reg(&[
                "add",
                &self.key,
                "/v",
                self.name,
                "/t",
                "REG_DWORD",
                "/d",
                value,
                "/f",
            ]),
            None => reg(&["delete", &self.key, "/v", self.name, "/f"]),
        };
    }
}

/// What `reg` printed, when it succeeded.
fn reg(args: &[&str]) -> Option<String> {
    let done = Command::new("reg").args(args).output().unwrap();

    done.status
        .success()
        .then(|| String::from_utf8_lossy(&done.stdout).into_owned())
}

/// `{GUID}` of the volume of `drive` (`C:`), from `mountvol C:\ /L`.
fn volume_guid(drive: &str) -> String {
    let done = Command::new("mountvol")
        .arg(format!("{drive}\\"))
        .arg("/L")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&done.stdout).into_owned();
    let open = text.find('{').unwrap_or_else(|| panic!("mountvol: {text}"));
    let close = text[open..].find('}').unwrap() + open;

    text[open..=close].to_owned()
}

fn junction(link: &Path, target: &Path) {
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap()
        .status;
    assert!(status.success(), "mklink /J");
}

#[test]
fn a_file_goes_to_the_recycle_bin_and_comes_back_the_same() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let file = root.join("a.txt");
    fs::write(&file, b"precious").unwrap();
    let seen = expected(&file, &root);

    let trashed = WindowsEntryRemover
        .trash(&bytes(&file), &seen, Path::new(""))
        .unwrap();

    assert!(!file.exists(), "it left its place");
    let trashed_path = PathBuf::from(String::from_utf8(trashed.clone()).unwrap());
    assert!(
        trashed_path.to_string_lossy().contains("$Recycle.Bin"),
        "{trashed_path:?}"
    );
    assert_eq!(fs::read(&trashed_path).unwrap(), b"precious");
    let in_bin = WindowsEntryProbe.probe(&trashed).unwrap();
    assert_eq!(
        (in_bin.last().unwrap().meta.dev, in_bin.last().unwrap().meta.ino),
        (seen.dev, seen.ino)
    );

    WindowsEntryRemover
        .restore(&trashed, &seen, &bytes(&file))
        .unwrap();

    assert_eq!(fs::read(&file).unwrap(), b"precious");
    assert!(!trashed_path.exists());
}

#[test]
fn a_folder_goes_whole_and_a_restore_refuses_a_taken_name() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let folder = root.join("dir");
    fs::create_dir_all(folder.join("inner")).unwrap();
    fs::write(folder.join("inner").join("f.txt"), b"x").unwrap();
    let seen = expected(&folder, &root);

    let trashed = WindowsEntryRemover
        .trash(&bytes(&folder), &seen, Path::new(""))
        .unwrap();
    assert!(!folder.exists());

    // Something else took the old name meanwhile.
    fs::write(&folder, b"another").unwrap();
    let refused = WindowsEntryRemover
        .restore(&trashed, &seen, &bytes(&folder))
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        fs::read(&folder).unwrap(),
        b"another",
        "the taken name was not touched"
    );

    fs::remove_file(&folder).unwrap();
    WindowsEntryRemover
        .restore(&trashed, &seen, &bytes(&folder))
        .unwrap();
    assert_eq!(fs::read(folder.join("inner").join("f.txt")).unwrap(), b"x");
}

#[test]
fn an_entry_that_was_swapped_since_the_plan_is_left_alone() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let file = root.join("a.txt");
    fs::write(&file, b"planned").unwrap();
    let seen = expected(&file, &root);

    fs::remove_file(&file).unwrap();
    fs::write(&file, b"another file with the same name").unwrap();
    let refused = WindowsEntryRemover
        .trash(&bytes(&file), &seen, Path::new(""))
        .unwrap_err();

    assert!(refused.to_string().contains("another entry"), "{refused}");
    assert_eq!(fs::read(&file).unwrap(), b"another file with the same name");
}

#[test]
fn a_link_at_the_target_or_on_the_way_is_not_followed_and_not_acted_on() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let real = root.join("real");
    fs::create_dir_all(real.join("inner")).unwrap();
    fs::write(real.join("inner").join("f.txt"), b"keep me").unwrap();
    let link = root.join("link");
    junction(&link, &real);

    // On the way: the probe refuses, and so does a trash that is given the entry's identity.
    let through = bytes(&link.join("inner").join("f.txt"));
    let probed = WindowsEntryProbe.probe(&through).unwrap_err();
    assert_eq!(probed.kind(), io::ErrorKind::NotADirectory);
    let seen = expected(&real.join("inner").join("f.txt"), &root);
    let refused = WindowsEntryRemover
        .trash(&through, &seen, Path::new(""))
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::NotADirectory);
    assert!(real.join("inner").join("f.txt").exists());

    // At the last name: described as a link, never entered, and never moved.
    let found = WindowsEntryProbe.probe(&bytes(&link)).unwrap();
    assert_eq!(found.last().unwrap().meta.kind, EntryKind::Symlink);
    let as_folder = ExpectedEntry {
        is_dir: true,
        ..expected(&real, &root)
    };
    let refused = WindowsEntryRemover
        .trash(&bytes(&link), &as_folder, Path::new(""))
        .unwrap_err();
    assert!(
        refused.to_string().contains("changed since the plan"),
        "{refused}"
    );
    assert!(link.exists() && real.join("inner").join("f.txt").exists());
}

#[test]
fn the_emptied_folder_stays_and_what_is_inside_can_each_be_put_back() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let folder = root.join("Temp");
    fs::create_dir_all(folder.join("sub")).unwrap();
    fs::write(folder.join("a.tmp"), b"a").unwrap();
    fs::write(folder.join("sub").join("b.tmp"), b"b").unwrap();
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("precious"), b"p").unwrap();
    junction(&folder.join("link"), &outside);
    let seen = expected(&folder, &root);

    let done = WindowsEntryRemover
        .trash_children(&bytes(&folder), &seen, Path::new(""), 100)
        .unwrap();

    assert!(folder.is_dir(), "the folder itself stays");
    assert!(!folder.join("a.tmp").exists() && !folder.join("sub").exists());
    assert!(folder.join("link").exists(), "a link is left where it is");
    assert!(
        outside.join("precious").exists(),
        "and what it leads to is untouched"
    );
    assert_eq!(done.children.len(), 3);
    assert_eq!(done.more, 0);
    for child in &done.children {
        let name = String::from_utf8(child.name.clone()).unwrap();
        match (&child.outcome, name.as_str()) {
            (ChildTrashed::To(to), "a.tmp" | "sub") => {
                let original = folder.join(&name);
                let wanted = ExpectedEntry {
                    dev: child.dev,
                    ino: child.ino,
                    is_dir: child.is_dir,
                    ..seen
                };
                WindowsEntryRemover
                    .restore(to, &wanted, &bytes(&original))
                    .unwrap();
                assert!(original.exists());
            }
            (ChildTrashed::Left(_), "link") => {}
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(fs::read(folder.join("sub").join("b.tmp")).unwrap(), b"b");
}

#[test]
fn only_so_many_children_are_acted_on_and_the_rest_are_counted() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let folder = root.join("Temp");
    fs::create_dir(&folder).unwrap();
    for name in ["1", "2", "3", "4"] {
        fs::write(folder.join(name), name).unwrap();
    }
    let seen = expected(&folder, &root);

    let done = WindowsEntryRemover
        .trash_children(&bytes(&folder), &seen, Path::new(""), 3)
        .unwrap();

    assert_eq!(done.children.len(), 3);
    assert_eq!(done.more, 1);
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
}

#[test]
fn deleting_for_good_is_not_something_this_remover_does() {
    let (_guard, root) = scratch();
    let file = root.join("a.txt");
    fs::write(&file, b"x").unwrap();
    let seen = expected(&file, &root);

    for mode in [
        platter_core::RemoveMode::Whole,
        platter_core::RemoveMode::Contents,
    ] {
        let refused = WindowsEntryRemover
            .remove(&bytes(&file), &seen, mode)
            .unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::Unsupported);
    }
    assert!(file.exists());
}

#[test]
fn paths_that_are_not_plainly_drive_absolute_are_not_looked_at() {
    for bad in [
        "C:/Windows",
        "\\\\?\\C:\\Windows",
        "\\\\localhost\\C$\\Windows",
        "Windows",
        "C:\\Windows\\..\\Windows",
        "C:\\NUL",
        "C:\\Windows.",
    ] {
        let error = WindowsEntryProbe.probe(bad.as_bytes()).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{bad}");
    }
}

#[test]
fn a_missing_entry_is_not_found_and_an_error_is_never_a_recycle_that_did_not_happen() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let file = root.join("gone.txt");
    fs::write(&file, b"x").unwrap();
    let seen = expected(&file, &root);
    fs::remove_file(&file).unwrap();

    let error = WindowsEntryRemover
        .trash(&bytes(&file), &seen, Path::new(""))
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(not_recycled(&error).is_none());
}

#[test]
fn what_the_bin_will_not_take_is_refused_and_stays_where_it_is() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let file = root.join("keep.txt");
    fs::write(&file, b"precious").unwrap();
    let folder = root.join("keep-dir");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("f.txt"), b"inside").unwrap();
    let seen_file = expected(&file, &root);
    let seen_folder = expected(&folder, &root);

    // The volume's bin is set to delete at once: the shell must say no, not delete.
    let _setting = BinSetting::set(&root, "NukeOnDelete", 1);
    let refused_file = WindowsEntryRemover.trash(&bytes(&file), &seen_file, Path::new(""));
    let refused_folder = WindowsEntryRemover.trash(&bytes(&folder), &seen_folder, Path::new(""));

    assert!(
        refused_file.is_err(),
        "the shell deleted at once: {refused_file:?}"
    );
    assert!(
        refused_folder.is_err(),
        "the shell deleted at once: {refused_folder:?}"
    );
    assert!(
        not_recycled(&refused_file.unwrap_err()).is_none(),
        "it was not taken away, so it is a refusal, not a recycle that did not happen"
    );
    assert_eq!(fs::read(&file).unwrap(), b"precious", "the file stays");
    assert_eq!(
        fs::read(folder.join("f.txt")).unwrap(),
        b"inside",
        "so does the folder"
    );
}

#[test]
fn an_item_too_large_for_the_bin_is_refused_and_stays_where_it_is() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let file = root.join("big.bin");
    fs::write(&file, vec![7u8; 3 * 1024 * 1024]).unwrap();
    let seen = expected(&file, &root);

    // The bin may hold 1 MB at most: the file is larger.
    let _setting = BinSetting::set(&root, "MaxCapacity", 1);
    let refused = WindowsEntryRemover.trash(&bytes(&file), &seen, Path::new(""));

    assert!(refused.is_err(), "the shell took it: {refused:?}");
    assert_eq!(fs::metadata(&file).unwrap().len(), 3 * 1024 * 1024, "it stays");
}

#[test]
fn children_the_bin_will_not_take_are_left_in_place_and_said_so() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let folder = root.join("Temp");
    fs::create_dir_all(folder.join("sub")).unwrap();
    fs::write(folder.join("a.tmp"), b"a").unwrap();
    fs::write(folder.join("sub").join("b.tmp"), b"b").unwrap();
    let seen = expected(&folder, &root);

    let _setting = BinSetting::set(&root, "NukeOnDelete", 1);
    let done = WindowsEntryRemover
        .trash_children(&bytes(&folder), &seen, Path::new(""), 100)
        .unwrap();

    assert_eq!(done.children.len(), 2);
    for child in &done.children {
        assert!(
            matches!(child.outcome, ChildTrashed::Left(_)),
            "{:?}",
            child.outcome
        );
    }
    assert_eq!(fs::read(folder.join("a.tmp")).unwrap(), b"a");
    assert_eq!(fs::read(folder.join("sub").join("b.tmp")).unwrap(), b"b");
}

#[test]
fn what_is_left_alone_does_not_use_up_the_limit() {
    let _bin = bin();
    let (_guard, root) = scratch();
    let folder = root.join("Temp");
    fs::create_dir(&folder).unwrap();
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    // Names that sort first (the listing of NTFS is sorted), so the links are looked at before the files.
    junction(&folder.join("a-link"), &outside);
    junction(&folder.join("b-link"), &outside);
    for name in ["x1", "x2", "x3"] {
        fs::write(folder.join(name), name).unwrap();
    }
    let seen = expected(&folder, &root);

    let done = WindowsEntryRemover
        .trash_children(&bytes(&folder), &seen, Path::new(""), 2)
        .unwrap();

    let trashed = done
        .children
        .iter()
        .filter(|child| matches!(child.outcome, ChildTrashed::To(_)))
        .count();
    let left = done
        .children
        .iter()
        .filter(|child| matches!(child.outcome, ChildTrashed::Left(_)))
        .count();
    assert_eq!((trashed, left, done.more), (2, 2, 1), "{:?}", done.children);
    assert!(folder.join("x3").exists(), "the third file is for the next run");
}
