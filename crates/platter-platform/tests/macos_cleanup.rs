//! Cleanup on macOS: the probe and the remover (trash, put back, refusals) on real folders.
//!
//! Every test works in its own temporary folder, with a "home trash" of its own named `.Trash` next to a
//! work folder (made empty), so the same volume holds both. The user's real `~/.Trash` is never touched.
#![cfg(target_os = "macos")]

use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use platter_core::ports::ChildTrashed;
use platter_core::{EntryKind, EntryProbe, EntryRemover, ExpectedEntry, RemoveMode};
use platter_platform::macos::{MacEntryProbe, MacEntryRemover};

/// A temporary scan root holding `home/work` (what is cleaned) and `home/.Trash` (where it goes).
struct Sandbox {
    _dir: tempfile::TempDir,
    root: PathBuf,
    work: PathBuf,
    trash: PathBuf,
}

/// The folder is created under its real path (`/var` is a link to `/private/var` on macOS).
fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let work = root.join("home/work");
    let trash = root.join("home/.Trash");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir(&trash).unwrap();

    Sandbox {
        _dir: dir,
        root,
        work,
        trash,
    }
}

fn bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

impl Sandbox {
    /// What the plan would have recorded for `path`: its identity as the probe sees it now.
    fn expected(&self, path: &Path) -> ExpectedEntry {
        let found = MacEntryProbe.probe(&bytes(path)).unwrap();
        let last = found.last().unwrap();

        ExpectedEntry {
            dev: last.meta.dev,
            ino: last.meta.ino,
            mount_id: None,
            birth: last.birth,
            is_dir: last.meta.kind == EntryKind::Dir,
            root_depth: self.root.components().count() - 1,
            entered_mounts: Vec::new(),
        }
    }

    fn trash(&self, path: &Path) -> io::Result<PathBuf> {
        let expected = self.expected(path);

        self.trash_as(path, &expected)
    }

    fn trash_as(&self, path: &Path, expected: &ExpectedEntry) -> io::Result<PathBuf> {
        let to = MacEntryRemover.trash(&bytes(path), expected, &self.trash)?;

        Ok(PathBuf::from(String::from_utf8(to).unwrap()))
    }
}

#[test]
fn a_file_goes_to_the_trash_and_comes_back_by_restore() {
    let sandbox = sandbox();
    let file = sandbox.work.join("report.pdf");
    std::fs::write(&file, b"contents").unwrap();
    let expected = sandbox.expected(&file);

    let trashed = sandbox.trash_as(&file, &expected).unwrap();

    assert_eq!(trashed, sandbox.trash.join("report.pdf"));
    assert!(!file.exists());
    assert_eq!(std::fs::read(&trashed).unwrap(), b"contents");

    MacEntryRemover
        .restore_from(&bytes(&trashed), &expected, &bytes(&file), &sandbox.trash)
        .unwrap();

    assert_eq!(std::fs::read(&file).unwrap(), b"contents");
    assert!(names_in(&sandbox.trash).is_empty());
}

#[test]
fn a_folder_goes_to_the_trash_whole_and_comes_back() {
    let sandbox = sandbox();
    let folder = sandbox.work.join("target");
    std::fs::create_dir_all(folder.join("debug/deps")).unwrap();
    std::fs::write(folder.join("debug/deps/lib.rlib"), b"x").unwrap();
    let expected = sandbox.expected(&folder);

    let trashed = sandbox.trash_as(&folder, &expected).unwrap();

    assert!(!folder.exists());
    assert!(trashed.join("debug/deps/lib.rlib").is_file());

    MacEntryRemover
        .restore_from(&bytes(&trashed), &expected, &bytes(&folder), &sandbox.trash)
        .unwrap();

    assert!(folder.join("debug/deps/lib.rlib").is_file());
}

#[test]
fn the_trash_folder_is_made_when_it_is_missing() {
    let sandbox = sandbox();
    let file = sandbox.work.join("a");
    std::fs::write(&file, b"x").unwrap();
    std::fs::remove_dir(&sandbox.trash).unwrap();

    let trashed = sandbox.trash(&file).unwrap();

    assert!(trashed.is_file());
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&sandbox.trash).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0, "closed to others");
}

#[test]
fn a_name_that_is_taken_in_the_trash_gets_a_number_the_way_finder_does() {
    let sandbox = sandbox();
    let mut trashed = Vec::new();
    for round in 0..3 {
        let file = sandbox.work.join("a.txt");
        std::fs::write(&file, format!("round {round}")).unwrap();
        trashed.push(sandbox.trash(&file).unwrap());
    }
    for round in 0..2 {
        let folder = sandbox.work.join("build");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("marker"), format!("round {round}")).unwrap();
        trashed.push(sandbox.trash(&folder).unwrap());
    }

    let names: Vec<String> = trashed
        .iter()
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["a.txt", "a 2.txt", "a 3.txt", "build", "build 2"]);
    // Nothing was overwritten: each holds what it was given.
    assert_eq!(std::fs::read_to_string(&trashed[0]).unwrap(), "round 0");
    assert_eq!(std::fs::read_to_string(&trashed[1]).unwrap(), "round 1");
    assert_eq!(std::fs::read_to_string(&trashed[2]).unwrap(), "round 2");
    assert_eq!(
        std::fs::read_to_string(trashed[3].join("marker")).unwrap(),
        "round 0"
    );
    assert_eq!(
        std::fs::read_to_string(trashed[4].join("marker")).unwrap(),
        "round 1"
    );
}

#[test]
fn another_file_under_the_same_name_is_not_trashed() {
    let sandbox = sandbox();
    let file = sandbox.work.join("data");
    std::fs::write(&file, b"planned").unwrap();
    let expected = sandbox.expected(&file);

    std::fs::remove_file(&file).unwrap();
    std::fs::write(&file, b"newer").unwrap();
    let error = sandbox.trash_as(&file, &expected).unwrap_err();

    assert!(error.to_string().contains("changed since the plan"), "{error}");
    assert_eq!(std::fs::read(&file).unwrap(), b"newer");
    assert!(names_in(&sandbox.trash).is_empty());
}

#[test]
fn a_file_that_became_a_folder_is_not_trashed() {
    let sandbox = sandbox();
    let path = sandbox.work.join("thing");
    std::fs::write(&path, b"x").unwrap();
    let expected = sandbox.expected(&path);

    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let error = sandbox.trash_as(&path, &expected).unwrap_err();

    assert!(error.to_string().contains("changed since the plan"), "{error}");
    assert!(path.is_dir());
}

#[test]
fn a_missing_target_is_refused() {
    let sandbox = sandbox();
    let file = sandbox.work.join("gone");
    std::fs::write(&file, b"x").unwrap();
    let expected = sandbox.expected(&file);
    std::fs::remove_file(&file).unwrap();

    let error = sandbox.trash_as(&file, &expected).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[test]
fn a_link_at_the_target_is_not_followed() {
    let sandbox = sandbox();
    let real = sandbox.work.join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(real.join("inner"), b"x").unwrap();
    let link = sandbox.work.join("link");
    symlink(&real, &link).unwrap();

    // The plan of the link itself, and the plan of what it points to: both are refused for the link.
    let own = sandbox.expected(&link);
    let pointed = sandbox.expected(&real);
    for expected in [own, pointed] {
        let error = sandbox.trash_as(&link, &expected).unwrap_err();
        assert!(error.to_string().contains("changed since the plan"), "{error}");
    }

    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    assert!(real.join("inner").is_file());
    assert!(names_in(&sandbox.trash).is_empty());
}

#[test]
fn a_link_on_the_way_is_not_followed() {
    let sandbox = sandbox();
    let real = sandbox.work.join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(real.join("inner"), b"x").unwrap();
    let expected = sandbox.expected(&real.join("inner"));
    symlink(&real, sandbox.work.join("way")).unwrap();

    let error = sandbox
        .trash_as(&sandbox.work.join("way/inner"), &expected)
        .unwrap_err();

    assert!(error.to_string().contains("changed since the plan"), "{error}");
    assert!(real.join("inner").is_file());
    assert!(names_in(&sandbox.trash).is_empty());
}

#[test]
fn a_trash_folder_that_is_a_link_is_refused() {
    let sandbox = sandbox();
    let elsewhere = sandbox.root.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::remove_dir(&sandbox.trash).unwrap();
    symlink(&elsewhere, &sandbox.trash).unwrap();
    let file = sandbox.work.join("a");
    std::fs::write(&file, b"x").unwrap();

    let error = sandbox.trash(&file).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::NotADirectory);
    assert!(file.is_file());
    assert!(names_in(&elsewhere).is_empty());
}

#[test]
fn the_trash_folder_itself_is_never_moved_into_itself() {
    let sandbox = sandbox();
    std::fs::write(sandbox.trash.join("kept"), b"x").unwrap();

    let error = sandbox.trash(&sandbox.trash).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

    let inside = sandbox.trash.join("kept");
    let error = sandbox.trash(&inside).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(inside.is_file());
}

#[test]
fn deleting_for_good_is_refused_in_every_mode() {
    let sandbox = sandbox();
    let folder = sandbox.work.join("cache");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("entry"), b"x").unwrap();
    let file = sandbox.work.join("file");
    std::fs::write(&file, b"x").unwrap();

    for (path, mode) in [
        (&folder, RemoveMode::Whole),
        (&folder, RemoveMode::Contents),
        (&file, RemoveMode::Whole),
    ] {
        let expected = sandbox.expected(path);
        let error = MacEntryRemover.remove(&bytes(path), &expected, mode).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    assert!(folder.join("entry").is_file());
    assert!(file.is_file());
}

#[test]
fn an_occupied_old_name_is_not_overwritten_by_restore() {
    let sandbox = sandbox();
    let file = sandbox.work.join("note");
    std::fs::write(&file, b"old").unwrap();
    let expected = sandbox.expected(&file);
    let trashed = sandbox.trash_as(&file, &expected).unwrap();
    std::fs::write(&file, b"new").unwrap();

    let error = MacEntryRemover
        .restore_from(&bytes(&trashed), &expected, &bytes(&file), &sandbox.trash)
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&file).unwrap(), b"new");
    assert_eq!(std::fs::read(&trashed).unwrap(), b"old");
}

#[test]
fn restore_refuses_an_entry_that_is_not_the_one_trashed() {
    let sandbox = sandbox();
    let file = sandbox.work.join("note");
    std::fs::write(&file, b"old").unwrap();
    let expected = sandbox.expected(&file);
    let trashed = sandbox.trash_as(&file, &expected).unwrap();
    std::fs::remove_file(&trashed).unwrap();
    std::fs::write(&trashed, b"someone else's").unwrap();

    let error = MacEntryRemover
        .restore_from(&bytes(&trashed), &expected, &bytes(&file), &sandbox.trash)
        .unwrap_err();

    assert!(error.to_string().contains("changed since the plan"), "{error}");
    assert!(!file.exists());
    assert_eq!(std::fs::read(&trashed).unwrap(), b"someone else's");
}

#[test]
fn restore_needs_the_old_folder_and_takes_only_from_a_trash_folder() {
    let sandbox = sandbox();
    let folder = sandbox.work.join("sub");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("note");
    std::fs::write(&file, b"x").unwrap();
    let expected = sandbox.expected(&file);
    let trashed = sandbox.trash_as(&file, &expected).unwrap();
    std::fs::remove_dir(&folder).unwrap();

    let error = MacEntryRemover
        .restore_from(&bytes(&trashed), &expected, &bytes(&file), &sandbox.trash)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(trashed.is_file());

    // An entry outside a trash folder is not put anywhere, however it is described.
    let loose = sandbox.work.join("loose");
    std::fs::write(&loose, b"x").unwrap();
    let loose_expected = sandbox.expected(&loose);
    let error = MacEntryRemover
        .restore_from(
            &bytes(&loose),
            &loose_expected,
            &bytes(&sandbox.work.join("elsewhere")),
            &sandbox.trash,
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(loose.is_file());
}

#[test]
fn restore_takes_only_from_the_trash_of_the_users_own_home() {
    let sandbox = sandbox();
    let file = sandbox.work.join("note");
    std::fs::write(&file, b"x").unwrap();
    let expected = sandbox.expected(&file);
    let trashed = sandbox.trash_as(&file, &expected).unwrap();

    // A `.Trash` of some other folder is not the home trash, whatever its name.
    let error = MacEntryRemover
        .restore(&bytes(&trashed), &expected, &bytes(&file))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

    let other = sandbox.root.join("home/other/.Trash");
    let error = MacEntryRemover
        .restore_from(&bytes(&trashed), &expected, &bytes(&file), &other)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

    assert!(trashed.is_file());
    assert!(!file.exists());
}

#[test]
fn the_children_of_a_folder_go_to_the_trash_and_the_folder_stays() {
    let sandbox = sandbox();
    let folder = sandbox.work.join("cache");
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::write(folder.join("a"), b"a").unwrap();
    std::fs::write(folder.join("b"), b"b").unwrap();
    std::fs::write(folder.join("sub/c"), b"c").unwrap();
    symlink(sandbox.work.join("elsewhere"), folder.join("link")).unwrap();
    let expected = sandbox.expected(&folder);

    let done = MacEntryRemover
        .trash_children(&bytes(&folder), &expected, &sandbox.trash, 10)
        .unwrap();

    assert_eq!(done.more, 0);
    assert_eq!(done.children.len(), 4);
    let mut moved = Vec::new();
    for child in &done.children {
        let name = String::from_utf8(child.name.clone()).unwrap();
        match &child.outcome {
            ChildTrashed::To(to) => {
                assert!(Path::new(std::str::from_utf8(to).unwrap()).exists(), "{name}");
                moved.push(name);
            }
            ChildTrashed::Left(why) => {
                assert_eq!(name, "link");
                assert!(why.contains("link"), "{why}");
            }
            other => panic!("{name}: {other:?}"),
        }
    }
    moved.sort();
    assert_eq!(moved, ["a", "b", "sub"]);
    assert_eq!(names_in(&folder), ["link"]);
    assert_eq!(names_in(&sandbox.trash), ["a", "b", "sub"]);

    // A child comes back the way a single entry does.
    let a = done.children.iter().find(|child| child.name == b"a").unwrap();
    let ChildTrashed::To(to) = &a.outcome else {
        unreachable!()
    };
    let planned = ExpectedEntry {
        dev: a.dev,
        ino: a.ino,
        mount_id: None,
        birth: None,
        is_dir: a.is_dir,
        root_depth: sandbox.root.components().count() - 1,
        entered_mounts: Vec::new(),
    };
    MacEntryRemover
        .restore_from(to, &planned, &bytes(&folder.join("a")), &sandbox.trash)
        .unwrap();
    assert_eq!(std::fs::read(folder.join("a")).unwrap(), b"a");
}

#[test]
fn no_more_children_than_the_limit_are_moved() {
    let sandbox = sandbox();
    let folder = sandbox.work.join("cache");
    std::fs::create_dir(&folder).unwrap();
    for name in ["a", "b", "c", "d"] {
        std::fs::write(folder.join(name), name).unwrap();
    }
    let expected = sandbox.expected(&folder);

    let done = MacEntryRemover
        .trash_children(&bytes(&folder), &expected, &sandbox.trash, 2)
        .unwrap();

    assert_eq!(done.more, 2);
    assert_eq!(names_in(&sandbox.trash).len(), 2);
    assert_eq!(names_in(&folder).len(), 2);
}

#[test]
fn a_folder_that_is_not_the_planned_one_has_no_child_moved() {
    let sandbox = sandbox();
    let folder = sandbox.work.join("cache");
    std::fs::create_dir(&folder).unwrap();
    let expected = sandbox.expected(&folder);
    std::fs::remove_dir(&folder).unwrap();
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("a"), b"a").unwrap();

    let error = MacEntryRemover
        .trash_children(&bytes(&folder), &expected, &sandbox.trash, 10)
        .unwrap_err();

    assert!(error.to_string().contains("changed since the plan"), "{error}");
    assert!(folder.join("a").is_file());
    assert!(names_in(&sandbox.trash).is_empty());

    // A file cannot be emptied.
    let file = sandbox.work.join("file");
    std::fs::write(&file, b"x").unwrap();
    let planned = sandbox.expected(&file);
    let error = MacEntryRemover
        .trash_children(&bytes(&file), &planned, &sandbox.trash, 10)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn free_space_is_reported_for_a_folder() {
    let sandbox = sandbox();

    assert!(MacEntryRemover.free_bytes(&sandbox.work).unwrap() > 0);
}

#[test]
fn the_filesystem_root_is_never_moved() {
    let sandbox = sandbox();
    let expected = ExpectedEntry {
        dev: 0,
        ino: 0,
        mount_id: None,
        birth: None,
        is_dir: true,
        root_depth: 0,
        entered_mounts: Vec::new(),
    };

    let error = MacEntryRemover
        .trash(b"/", &expected, &sandbox.trash)
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}
