use std::fs::{self, File};
use std::path::PathBuf;
use std::time::SystemTime;

use platter_core::testing::FakeFs;
use platter_core::{CancelToken, NodeId, ScanConfig, scan};
use serde_json::json;

use super::*;

fn tree_at(now: i64, file: &str) -> Tree {
    let mut fs = FakeFs::new();
    fs.file("/photos/a.jpg", 20_000)
        .file(file, 5)
        .file("/notes.txt", 7);
    let config = ScanConfig {
        now: Some(now),
        ..ScanConfig::default()
    };

    scan(&fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).unwrap()
}

fn options() -> ScanOptions {
    ScanOptions {
        root: "/".to_owned(),
        one_file_system: true,
        excluded: vec!["/proc".to_owned()],
    }
}

/// Everything a person could see of a tree.
fn describe(tree: &Tree) -> Vec<(PathBuf, u64, u64, i64)> {
    (0..tree.len() as u32)
        .map(|index| {
            let id = NodeId(index);
            (
                tree.path_of(id),
                tree.allocated(id),
                tree.apparent(id),
                tree.mtime(id),
            )
        })
        .collect()
}

fn store_in(dir: &Path) -> Arc<LastScanStore> {
    Arc::new(LastScanStore::new(Some(dir.to_path_buf())))
}

fn save_and_wait(store: &Arc<LastScanStore>, tree: Tree, outdated: bool) {
    store
        .save(Arc::new(tree), options(), || outdated)
        .expect("a thread was started")
        .join()
        .unwrap();
}

fn sidecar_json(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(dir.join(SIDECAR_FILE)).unwrap()).unwrap()
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();

    names
}

#[test]
fn a_saved_scan_is_found_again_by_a_new_store_with_everything_it_had() {
    let dir = tempfile::tempdir().unwrap();
    let tree = tree_at(1_700_000_000, "/photos/b.jpg");
    save_and_wait(
        &store_in(dir.path()),
        tree_at(1_700_000_000, "/photos/b.jpg"),
        false,
    );

    let found = store_in(dir.path()).load(u64::MAX).expect("the scan was kept");

    assert_eq!(describe(&found.tree), describe(&tree));
    assert_eq!(found.tree.totals(), tree.totals());
    assert_eq!(found.tree.scanned_at(), 1_700_000_000);
    assert_eq!(found.options, options());
    assert!(!found.outdated);
}

#[test]
fn the_scan_is_two_files_in_the_folder_the_tree_compressed_and_the_sidecar_in_json() {
    let dir = tempfile::tempdir().unwrap();

    save_and_wait(&store_in(dir.path()), tree_at(1_700_000_000, "/x"), false);

    assert_eq!(files_in(dir.path()), ["last-scan.bin", "last-scan.json"]);
    let bin = fs::read(dir.path().join("last-scan.bin")).unwrap();
    assert_eq!(bin[..4], [0x28, 0xB5, 0x2F, 0xFD], "a zstd frame");
    assert_eq!(
        sidecar_json(dir.path()),
        json!({
            "version": 1,
            "options": { "root": "/", "oneFileSystem": true, "excluded": ["/proc"] },
            "scannedAt": 1_700_000_000,
            "outdated": false,
        })
    );
}

#[test]
fn only_the_newest_scan_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());

    save_and_wait(&store, tree_at(1_000, "/first"), false);
    save_and_wait(&store, tree_at(2_000, "/second"), false);

    let found = store.load(u64::MAX).unwrap();
    assert_eq!(found.tree.scanned_at(), 2_000);
    assert_eq!(files_in(dir.path()), ["last-scan.bin", "last-scan.json"]);
}

#[test]
fn a_save_that_a_newer_scan_has_passed_does_not_overwrite_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    // The first save has begun (generation 1) when the second scan finishes and is saved (generation 2).
    let older = tree_at(1_000, "/first");
    store.lock().generation = 1;

    save_and_wait(&store, tree_at(2_000, "/second"), false);
    store.write(dir.path(), 1, &older, &options()).unwrap();

    assert_eq!(store.load(u64::MAX).unwrap().tree.scanned_at(), 2_000);
    assert_eq!(
        files_in(dir.path()),
        ["last-scan.bin", "last-scan.json"],
        "no temporary file is left"
    );
}

#[test]
fn a_cleanup_marks_the_kept_scan_outdated_and_only_the_sidecar_is_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    save_and_wait(&store, tree_at(1_700_000_000, "/x"), false);
    let bin_before = fs::read(dir.path().join(TREE_FILE)).unwrap();

    store.mark_outdated();

    assert_eq!(sidecar_json(dir.path())["outdated"], true);
    assert_eq!(fs::read(dir.path().join(TREE_FILE)).unwrap(), bin_before);
    assert!(
        store_in(dir.path()).load(u64::MAX).unwrap().outdated,
        "after a restart too"
    );
    assert_eq!(files_in(dir.path()), ["last-scan.bin", "last-scan.json"]);
}

#[test]
fn a_cleanup_while_the_scan_is_being_written_leaves_it_outdated() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());

    let writing = store
        .save(Arc::new(tree_at(1_700_000_000, "/x")), options(), || false)
        .unwrap();
    store.mark_outdated();
    writing.join().unwrap();

    assert!(store.load(u64::MAX).unwrap().outdated);
}

#[test]
fn a_scan_that_finished_outdated_is_kept_outdated() {
    let dir = tempfile::tempdir().unwrap();

    save_and_wait(&store_in(dir.path()), tree_at(1_700_000_000, "/x"), true);

    assert!(store_in(dir.path()).load(u64::MAX).unwrap().outdated);
}

#[test]
fn nothing_is_found_when_nothing_was_kept() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());

    assert!(store.load(u64::MAX).is_none(), "an empty folder");
    assert!(
        LastScanStore::new(Some(dir.path().join("not-made")))
            .load(u64::MAX)
            .is_none(),
        "no folder"
    );
    assert!(
        LastScanStore::new(None).load(u64::MAX).is_none(),
        "no data folder"
    );
    store.mark_outdated();
    assert!(
        files_in(dir.path()).is_empty(),
        "a cleanup makes no file by itself"
    );
}

#[test]
fn a_store_without_a_folder_saves_nothing() {
    let store = Arc::new(LastScanStore::new(None));

    assert!(
        store
            .save(Arc::new(tree_at(1, "/x")), options(), || false)
            .is_none()
    );
}

/// A change to the files in a data folder.
type Damage = Box<dyn Fn(&Path)>;

#[test]
fn files_that_cannot_be_used_are_the_same_as_no_scan() {
    let damage: Vec<(&str, Damage)> = vec![
        (
            "no sidecar",
            Box::new(|dir| fs::remove_file(dir.join(SIDECAR_FILE)).unwrap()),
        ),
        (
            "no tree",
            Box::new(|dir| fs::remove_file(dir.join(TREE_FILE)).unwrap()),
        ),
        (
            "an empty tree file",
            Box::new(|dir| fs::write(dir.join(TREE_FILE), b"").unwrap()),
        ),
        (
            "a tree that is not zstd",
            Box::new(|dir| fs::write(dir.join(TREE_FILE), b"hello there").unwrap()),
        ),
        (
            "a cut tree",
            Box::new(|dir| {
                let bytes = fs::read(dir.join(TREE_FILE)).unwrap();
                fs::write(dir.join(TREE_FILE), &bytes[..bytes.len() / 2]).unwrap();
            }),
        ),
        (
            "a damaged tree",
            Box::new(|dir| {
                let mut bytes = fs::read(dir.join(TREE_FILE)).unwrap();
                let middle = bytes.len() / 2;
                bytes[middle] ^= 0xFF;
                fs::write(dir.join(TREE_FILE), bytes).unwrap();
            }),
        ),
        (
            "a sidecar that is not json",
            Box::new(|dir| fs::write(dir.join(SIDECAR_FILE), b"{").unwrap()),
        ),
        (
            "an empty sidecar",
            Box::new(|dir| fs::write(dir.join(SIDECAR_FILE), b"").unwrap()),
        ),
        (
            "another version",
            Box::new(|dir| rewrite_sidecar(dir, |sidecar| sidecar["version"] = json!(2))),
        ),
        (
            "a sidecar of another scan",
            Box::new(|dir| rewrite_sidecar(dir, |sidecar| sidecar["scannedAt"] = json!(5))),
        ),
        (
            "a sidecar with a missing field",
            Box::new(|dir| {
                rewrite_sidecar(dir, |sidecar| {
                    sidecar.as_object_mut().unwrap().remove("outdated");
                })
            }),
        ),
        (
            "a root that is not absolute",
            Box::new(|dir| rewrite_sidecar(dir, |sidecar| sidecar["options"]["root"] = json!("home/me"))),
        ),
        (
            "folders to leave out that are not absolute",
            Box::new(|dir| rewrite_sidecar(dir, |sidecar| sidecar["options"]["excluded"] = json!(["tmp"]))),
        ),
    ];

    for (what, damage) in damage {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        save_and_wait(&store, tree_at(1_700_000_000, "/x"), false);
        assert!(store.load(u64::MAX).is_some(), "{what}: the pair is fine before");

        damage(dir.path());

        assert!(store.load(u64::MAX).is_none(), "{what}");
    }
}

fn rewrite_sidecar(dir: &Path, change: impl FnOnce(&mut serde_json::Value)) {
    let mut sidecar = sidecar_json(dir);
    change(&mut sidecar);
    fs::write(dir.join(SIDECAR_FILE), serde_json::to_vec(&sidecar).unwrap()).unwrap();
}

#[test]
fn a_tree_larger_than_the_memory_that_may_be_used_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    save_and_wait(&store, tree_at(1_700_000_000, "/x"), false);
    let held = store.load(u64::MAX).unwrap().tree.memory_bytes() as u64;

    assert!(store.load(held).is_some());
    assert!(store.load(held - 1).is_none());
}

#[test]
fn a_crash_between_the_two_renames_leaves_nothing_to_open() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    save_and_wait(&store, tree_at(1_000, "/first"), false);

    // What a stop right after the new tree replaced the old one looks like: the new tree, no sidecar.
    let newer = Arc::new(tree_at(2_000, "/second"));
    let temp = dir.path().join(temp_name("bin", "9"));
    write_tree(&temp, &newer).unwrap();
    fs::remove_file(dir.path().join(SIDECAR_FILE)).unwrap();
    fs::rename(&temp, dir.path().join(TREE_FILE)).unwrap();

    assert!(store.load(u64::MAX).is_none());
}

fn age(path: &Path, by: Duration) {
    let old = SystemTime::now() - by;
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(old)
        .unwrap();
}

#[test]
fn only_an_old_temporary_file_is_swept_by_a_save_and_never_one_that_may_be_another_copys() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("last-scan.bin.77-1.tmp");
    let fresh = dir.path().join("last-scan.bin.88-2.tmp");
    fs::write(&old, b"half a tree").unwrap();
    fs::write(&fresh, b"half a tree of another copy of the app").unwrap();
    fs::write(dir.path().join("other.tmp"), b"not ours").unwrap();
    age(&old, STALE_AFTER + Duration::from_secs(60));

    save_and_wait(&store_in(dir.path()), tree_at(1_700_000_000, "/x"), false);

    assert_eq!(
        files_in(dir.path()),
        [
            "last-scan.bin",
            "last-scan.bin.88-2.tmp",
            "last-scan.json",
            "other.tmp"
        ]
    );
}

#[test]
fn temporary_files_are_new_files_with_names_of_their_own() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (unique_token(), unique_token());

    assert_ne!(first, second);
    assert!(first.starts_with(&format!("{}-", std::process::id())));

    let path = dir.path().join(temp_name("bin", &first));
    drop(create_private_new(&path).unwrap());
    assert_eq!(
        create_private_new(&path).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists,
        "a file that is there is never written into"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
    }
}

#[test]
fn a_tree_whose_frame_does_not_end_cleanly_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    save_and_wait(&store, tree_at(1_700_000_000, "/x"), false);
    let path = dir.path().join(TREE_FILE);
    let good = fs::read(&path).unwrap();

    // The frame's checksum is its last four bytes: the tree is read whole before it is looked at.
    let mut bad_checksum = good.clone();
    *bad_checksum.last_mut().unwrap() ^= 0xFF;
    fs::write(&path, &bad_checksum).unwrap();
    assert!(store.load(u64::MAX).is_none(), "a damaged checksum");

    // Anything after the tree is more than was written.
    let mut with_more = good.clone();
    with_more.extend(zstd::encode_all(&b"more"[..], 1).unwrap());
    fs::write(&path, &with_more).unwrap();
    assert!(store.load(u64::MAX).is_none(), "a second frame");

    fs::write(&path, &good).unwrap();
    assert!(store.load(u64::MAX).is_some(), "the undamaged file opens");
}

#[cfg(unix)]
#[test]
fn a_link_in_place_of_either_file_is_not_followed() {
    for name in [TREE_FILE, SIDECAR_FILE] {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        save_and_wait(&store, tree_at(1_700_000_000, "/x"), false);
        let target = dir.path().join("elsewhere");
        fs::rename(dir.path().join(name), &target).unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(name)).unwrap();

        assert!(store.load(u64::MAX).is_none(), "{name}");
        store.mark_outdated();
        assert!(
            fs::symlink_metadata(dir.path().join(name)).unwrap().is_symlink(),
            "{name} is left alone"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_folder_instead_of_a_file_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    save_and_wait(&store, tree_at(1_700_000_000, "/x"), false);
    fs::remove_file(dir.path().join(TREE_FILE)).unwrap();
    fs::create_dir(dir.path().join(TREE_FILE)).unwrap();

    assert!(store.load(u64::MAX).is_none());
}

#[test]
fn the_store_is_not_locked_while_a_tree_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path());
    save_and_wait(&store, tree_at(1_700_000_000, "/x"), false);

    // A tree being read from the open file does not keep a save or a cleanup waiting: the file is open
    // and the lock is let go.
    let (sidecar, file) = {
        let _files = store.lock();
        open_pair(dir.path()).unwrap()
    };
    store.mark_outdated();
    save_and_wait(&store, tree_at(2_000_000_000, "/y"), false);

    let mut decoder = zstd::Decoder::new(file).unwrap();
    let tree = Tree::read_snapshot(&mut decoder).unwrap();
    assert_eq!(
        tree.scanned_at(),
        sidecar.scanned_at,
        "the file that was open is the one that was read"
    );
    assert_eq!(store.load(u64::MAX).unwrap().tree.scanned_at(), 2_000_000_000);
}

#[test]
fn a_folder_that_cannot_be_written_drops_the_save_without_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("a-file");
    fs::write(&blocked, b"").unwrap();
    let store = Arc::new(LastScanStore::new(Some(blocked.join("inside"))));

    let handle = store
        .save(Arc::new(tree_at(1, "/x")), options(), || false)
        .unwrap();

    assert!(handle.join().is_ok());
    assert!(store.load(u64::MAX).is_none());
}
