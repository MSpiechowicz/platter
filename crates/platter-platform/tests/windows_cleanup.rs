//! The Recycle Bin remover and the probe on a real Windows volume. Run in CI only (`windows-latest`): they
//! use the Recycle Bin of the runner's system drive and a tiny temporary tree. Every one that uses the bin
//! runs under a watchdog (see `watched`), so a shell that waits for an answer fails the test and not the job.
//!
//! The tests that change the Recycle Bin's settings in the registry (`NukeOnDelete`, `MaxCapacity`, the
//! policies) change the settings of whoever runs them, and a test that is killed would leave them changed
//! (a bin that deletes at once, a bin of 1 MB that may purge what is in it). So they run only when
//! `PLATTER_TEST_CHANGE_RECYCLE_BIN=1` is set (the CI jobs set it, on a runner that is thrown away), and
//! even then the values are written to a marker file first, which the next run restores from if this one
//! was killed (see `Marker`).

#![cfg(windows)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, mpsc};
use std::time::Duration;

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

/// How long a test that uses the Recycle Bin may take. The shell can wait for an answer nobody gives, and a
/// test that waits with it would hold the CI job for its whole time limit: this one fails instead.
const WATCHDOG: Duration = Duration::from_secs(60);

/// Runs `body` with the bin to itself, in its own thread, and fails when it has not ended within
/// [`WATCHDOG`] (the thread is left behind), or when it panicked.
fn watched(body: impl FnOnce() + Send + 'static) {
    let (done, ended) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _bin = bin();
        // With the bin to itself, what is in the marker was left by a run that was killed.
        restore_leftovers();
        body();
        let _ = done.send(());
    });

    match ended.recv_timeout(WATCHDOG) {
        Ok(()) => thread.join().unwrap(),
        Err(mpsc::RecvTimeoutError::Disconnected) => match thread.join() {
            Err(panic) => std::panic::resume_unwind(panic),
            Ok(()) => unreachable!("the test ended without saying so"),
        },
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // The stuck thread may never reach its own restore: the marker has what it changed.
            restore_leftovers();
            panic!(
                "the test did not end in {} s: the shell is probably waiting on a question nobody can answer",
                WATCHDOG.as_secs()
            )
        }
    }
}

/// Whether the tests that change the Recycle Bin's settings may run: only when asked for explicitly.
fn registry_tests_enabled() -> bool {
    let enabled = std::env::var("PLATTER_TEST_CHANGE_RECYCLE_BIN").is_ok_and(|value| value == "1");
    if !enabled {
        eprintln!(
            "skipped: this test changes the Recycle Bin's settings in the registry, set PLATTER_TEST_CHANGE_RECYCLE_BIN=1 to run it"
        );
        return false;
    }

    true
}

/// Where the settings that are changed are kept until they are put back.
fn marker_path() -> PathBuf {
    std::env::temp_dir().join("platter-test-recycle-bin-settings.txt")
}

/// The settings a test changed and has not put back yet, one per line: `key|name|value` (`value` empty
/// when there was none). Written before a setting is changed and rewritten when one is put back.
fn read_marker() -> Vec<(String, String, String)> {
    let text = fs::read_to_string(marker_path()).unwrap_or_default();

    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '|');
            Some((
                parts.next()?.to_owned(),
                parts.next()?.to_owned(),
                parts.next()?.to_owned(),
            ))
        })
        .collect()
}

fn write_marker(entries: &[(String, String, String)]) {
    if entries.is_empty() {
        let _ = fs::remove_file(marker_path());
        return;
    }

    let text: String = entries
        .iter()
        .map(|(key, name, value)| format!("{key}|{name}|{value}\n"))
        .collect();
    fs::write(marker_path(), text).expect("the marker of the changed settings could not be written");
}

/// Puts back every setting that is in the marker: what an earlier run that was killed (or timed out) left
/// changed. Also run by the watchdog before it fails a test, whose thread may never get to put its own back.
fn restore_leftovers() {
    // Only a run that may change the settings puts old ones back: a plain `cargo test` never writes the
    // registry, not even from a marker a killed run left.
    if std::env::var("PLATTER_TEST_CHANGE_RECYCLE_BIN").as_deref() != Ok("1") {
        return;
    }
    for (key, name, value) in read_marker() {
        put_back(&key, &name, (!value.is_empty()).then_some(value.as_str()));
    }
    let _ = fs::remove_file(marker_path());
}

fn put_back(key: &str, name: &str, before: Option<&str>) {
    let _ = match before {
        Some(value) => reg(&["add", key, "/v", name, "/t", "REG_DWORD", "/d", value, "/f"]),
        None => reg(&["delete", key, "/v", name, "/f"]),
    };
}

/// The key of the Recycle Bin's settings of the volume of `on`.
fn volume_key(on: &Path) -> String {
    let drive: String = on.to_string_lossy().chars().take(2).collect();

    format!(
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\BitBucket\\Volume\\{}",
        volume_guid(&drive)
    )
}

/// The key of the policies that Platter reads (`HKCU`; `HKLM` needs the administrator).
const POLICY_KEY: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\Explorer";

/// A value of the Recycle Bin's settings in the registry (per volume: `NukeOnDelete`, delete at once, and
/// `MaxCapacity`, the most the bin holds, in MB; or a policy: `NoRecycleFiles`, `RecycleBinSize`), set for a
/// test and put back as it was when dropped, also when the test fails.
struct BinSetting {
    key: String,
    name: &'static str,
    /// The value before, as `reg` writes it (`0x0`); `None` when there was none.
    before: Option<String>,
}

impl BinSetting {
    fn set(on: &Path, name: &'static str, value: u32) -> Self {
        Self::at(volume_key(on), name, value)
    }

    fn policy(name: &'static str, value: u32) -> Self {
        Self::at(POLICY_KEY.to_owned(), name, value)
    }

    fn at(key: String, name: &'static str, value: u32) -> Self {
        let before = reg(&["query", &key, "/v", name])
            .filter(|text| text.contains(name))
            .and_then(|text| text.split_whitespace().last().map(str::to_owned));

        // Written down before it is changed: a run that is killed from here on is put right by the next.
        let mut marker = read_marker();
        marker.push((key.clone(), name.to_owned(), before.clone().unwrap_or_default()));
        write_marker(&marker);

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
        put_back(&self.key, self.name, self.before.as_deref());

        let mut marker = read_marker();
        if let Some(at) = marker
            .iter()
            .position(|(key, name, _)| *key == self.key && name == self.name)
        {
            marker.remove(at);
        }
        write_marker(&marker);
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
    watched(|| {
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
    });
}

#[test]
fn a_folder_goes_whole_and_a_restore_refuses_a_taken_name() {
    watched(|| {
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
    });
}

#[test]
fn an_entry_that_was_swapped_since_the_plan_is_left_alone() {
    watched(|| {
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
    });
}

#[test]
fn a_link_at_the_target_or_on_the_way_is_not_followed_and_not_acted_on() {
    watched(|| {
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
    });
}

#[test]
fn the_emptied_folder_stays_and_what_is_inside_can_each_be_put_back() {
    watched(|| {
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
    });
}

#[test]
fn only_so_many_children_are_acted_on_and_the_rest_are_counted() {
    watched(|| {
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
    });
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
    watched(|| {
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
    });
}

/// What a refusal before the shell looks like: the pre-check's text, an error that is not a recycle that
/// did not happen (the item was never touched), and `needle` as the reason.
fn assert_refused_before_the_shell(result: io::Result<Vec<u8>>, needle: &str) {
    let error = result.expect_err("the bin would not take it, so it must be refused");

    assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{error}");
    assert!(
        error.to_string().contains("would not keep it"),
        "not the pre-check's refusal: {error}"
    );
    assert!(error.to_string().contains(needle), "{error}");
    assert!(
        not_recycled(&error).is_none(),
        "it was not taken away, so it is a refusal, not a recycle that did not happen"
    );
}

#[test]
fn what_would_pass_max_path_in_the_bin_is_not_asked_and_stays() {
    watched(|| {
        let (_guard, root) = scratch();

        // Short where it is (under 260), but in the bin the folder is named `C:\$Recycle.Bin\<SID>\$R...`
        // and what is below keeps its path: 100 + 4 + 8 levels of 21 is over 260.
        let deep = root.join("deep");
        let mut inner = deep.clone();
        for _ in 0..8 {
            inner = inner.join("a".repeat(20));
        }
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join("f.txt"), b"inside").unwrap();
        let seen = expected(&deep, &root);

        let refused = WindowsEntryRemover.trash(&bytes(&deep), &seen, Path::new(""));

        assert_refused_before_the_shell(refused, "characters");
        assert_eq!(fs::read(inner.join("f.txt")).unwrap(), b"inside");

        // A file whose own name is too long for the bin.
        let long = root.join("n".repeat(170));
        fs::write(&long, b"long").unwrap();
        let seen = expected(&long, &root);

        let refused = WindowsEntryRemover.trash(&bytes(&long), &seen, Path::new(""));

        assert_refused_before_the_shell(refused, "characters");
        assert_eq!(fs::read(&long).unwrap(), b"long");
    });
}

#[test]
fn a_bin_set_to_delete_at_once_is_not_asked_and_the_item_stays() {
    if !registry_tests_enabled() {
        return;
    }
    watched(|| {
        let (_guard, root) = scratch();
        let file = root.join("keep.txt");
        fs::write(&file, b"precious").unwrap();
        let folder = root.join("keep-dir");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("f.txt"), b"inside").unwrap();
        let seen_file = expected(&file, &root);
        let seen_folder = expected(&folder, &root);

        // The volume's bin is set to delete at once: the shell must never be asked.
        let _setting = BinSetting::set(&root, "NukeOnDelete", 1);
        let refused_file = WindowsEntryRemover.trash(&bytes(&file), &seen_file, Path::new(""));
        let refused_folder = WindowsEntryRemover.trash(&bytes(&folder), &seen_folder, Path::new(""));

        assert_refused_before_the_shell(refused_file, "delete files at once");
        assert_refused_before_the_shell(refused_folder, "delete files at once");
        assert_eq!(fs::read(&file).unwrap(), b"precious", "the file stays");
        assert_eq!(
            fs::read(folder.join("f.txt")).unwrap(),
            b"inside",
            "so does the folder"
        );
    });
}

#[test]
fn an_item_larger_than_the_bin_is_not_asked_and_stays_where_it_is() {
    if !registry_tests_enabled() {
        return;
    }
    watched(|| {
        let (_guard, root) = scratch();
        let file = root.join("big.bin");
        fs::write(&file, vec![7u8; 3 * 1024 * 1024]).unwrap();
        let folder = root.join("big-dir");
        fs::create_dir_all(folder.join("inner")).unwrap();
        fs::write(folder.join("inner").join("big.bin"), vec![7u8; 3 * 1024 * 1024]).unwrap();
        let seen_file = expected(&file, &root);
        let seen_folder = expected(&folder, &root);

        // The bin may hold 1 MB at most: the file, and what the folder holds, are larger.
        let _setting = BinSetting::set(&root, "MaxCapacity", 1);
        let refused_file = WindowsEntryRemover.trash(&bytes(&file), &seen_file, Path::new(""));
        let refused_folder = WindowsEntryRemover.trash(&bytes(&folder), &seen_folder, Path::new(""));

        assert_refused_before_the_shell(refused_file, "holds 1 MB");
        assert_refused_before_the_shell(refused_folder, "holds 1 MB");
        assert_eq!(fs::metadata(&file).unwrap().len(), 3 * 1024 * 1024, "it stays");
        assert!(
            folder.join("inner").join("big.bin").exists(),
            "so does the folder"
        );
    });
}

#[test]
fn an_item_below_the_bins_size_is_still_moved() {
    if !registry_tests_enabled() {
        return;
    }
    watched(|| {
        let (_guard, root) = scratch();
        let file = root.join("small.bin");
        fs::write(&file, vec![7u8; 10 * 1024]).unwrap();
        let seen = expected(&file, &root);

        // The pre-check refuses what does not fit, not everything.
        let _setting = BinSetting::set(&root, "MaxCapacity", 1);
        let trashed = WindowsEntryRemover
            .trash(&bytes(&file), &seen, Path::new(""))
            .unwrap();

        assert!(!file.exists());
        WindowsEntryRemover
            .restore(&trashed, &seen, &bytes(&file))
            .unwrap();
        assert_eq!(fs::metadata(&file).unwrap().len(), 10 * 1024);
    });
}

#[test]
fn a_policy_against_the_bin_is_not_asked_and_the_item_stays() {
    if !registry_tests_enabled() {
        return;
    }
    watched(|| {
        let (_guard, root) = scratch();
        let file = root.join("keep.txt");
        fs::write(&file, b"precious").unwrap();
        let seen = expected(&file, &root);

        {
            let _policy = BinSetting::policy("NoRecycleFiles", 1);
            let refused = WindowsEntryRemover.trash(&bytes(&file), &seen, Path::new(""));
            assert_refused_before_the_shell(refused, "policy");
        }
        {
            let _policy = BinSetting::policy("RecycleBinSize", 10);
            let refused = WindowsEntryRemover.trash(&bytes(&file), &seen, Path::new(""));
            assert_refused_before_the_shell(refused, "policy");
        }

        assert_eq!(fs::read(&file).unwrap(), b"precious", "the file stays");
    });
}

#[test]
fn children_the_bin_will_not_take_are_left_in_place_and_said_so() {
    if !registry_tests_enabled() {
        return;
    }
    watched(|| {
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
            match &child.outcome {
                ChildTrashed::Left(why) => {
                    assert!(why.contains("would not keep it"), "{why}");
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(fs::read(folder.join("a.tmp")).unwrap(), b"a");
        assert_eq!(fs::read(folder.join("sub").join("b.tmp")).unwrap(), b"b");
    });
}

#[test]
fn a_child_larger_than_the_bin_is_left_and_the_others_are_moved() {
    if !registry_tests_enabled() {
        return;
    }
    watched(|| {
        let (_guard, root) = scratch();
        let folder = root.join("Temp");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("big.tmp"), vec![7u8; 3 * 1024 * 1024]).unwrap();
        fs::write(folder.join("small.tmp"), b"s").unwrap();
        let seen = expected(&folder, &root);

        let _setting = BinSetting::set(&root, "MaxCapacity", 1);
        let done = WindowsEntryRemover
            .trash_children(&bytes(&folder), &seen, Path::new(""), 100)
            .unwrap();

        assert_eq!(done.children.len(), 2);
        for child in &done.children {
            let name = String::from_utf8(child.name.clone()).unwrap();
            match (&child.outcome, name.as_str()) {
                (ChildTrashed::Left(why), "big.tmp") => assert!(why.contains("holds 1 MB"), "{why}"),
                (ChildTrashed::To(_), "small.tmp") => {}
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(
            fs::metadata(folder.join("big.tmp")).unwrap().len(),
            3 * 1024 * 1024
        );
        assert!(!folder.join("small.tmp").exists());
    });
}

#[test]
fn what_is_left_alone_does_not_use_up_the_limit() {
    watched(|| {
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
    });
}
