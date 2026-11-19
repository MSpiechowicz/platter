use std::fs;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use platter_core::dto::{ApiErrorCode, ScanEvent, ScanOptions};
use platter_core::testing::{FakeFs, scan_fake};
use platter_core::{EntryProbe, ProbedEntry};
use serde_json::json;
use std::sync::Mutex;

use super::*;
use crate::commands::history::overview;
use crate::commands::scan::start_pinned;

fn sample_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("photos/2024")).unwrap();
    fs::write(dir.path().join("photos/2024/a.jpg"), vec![1u8; 20_000]).unwrap();
    fs::write(dir.path().join("notes.txt"), b"hello").unwrap();

    dir
}

fn options(root: &std::path::Path) -> ScanOptions {
    ScanOptions {
        root: root.to_string_lossy().into_owned(),
        one_file_system: false,
        excluded: Vec::new(),
    }
}

/// A scan to its terminal event, as the web view starts one: through `allowed_root`.
fn scan_to_the_end(state: &AppState, options: ScanOptions, pinned: Option<(u64, u64)>) -> ScanEvent {
    scan_then(state, options, pinned, || {})
}

/// [`scan_to_the_end`], and `on_finished` runs when the scan's last event is sent (on the scan's thread, after
/// the save of the scan began).
fn scan_then(
    state: &AppState,
    options: ScanOptions,
    pinned: Option<(u64, u64)>,
    on_finished: impl Fn() + Send + Sync + 'static,
) -> ScanEvent {
    let (sender, receiver) = mpsc::channel();
    tauri::async_runtime::block_on(start_pinned(
        state,
        options,
        pinned,
        Arc::new(move |event| {
            if matches!(event, ScanEvent::Finished { .. }) {
                on_finished();
            }
            let _ = sender.send(event);
        }),
    ))
    .expect("the scan starts");

    loop {
        let event = receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("a terminal event");
        if !matches!(event, ScanEvent::Progress { .. }) {
            return event;
        }
    }
}

/// Waits for the background save of a finished scan to have written both files.
fn wait_for_the_save(state: &AppState) {
    let deadline = Instant::now() + Duration::from_secs(30);

    while state.last_scan.load(u64::MAX).is_none() {
        assert!(Instant::now() < deadline, "the scan was not saved");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn open_now(state: &AppState) -> CommandResult<Option<LastScanDto>> {
    tauri::async_runtime::block_on(open(state))
}

/// A scan of `dir` that finished and was saved, in a state whose data folder is `data`.
fn saved_scan(data: &std::path::Path, dir: &std::path::Path) -> (AppState, ScanEvent) {
    let state = AppState::for_test().with_last_scan_dir(data);
    let finished = scan_to_the_end(&state, options(dir), None);
    wait_for_the_save(&state);

    (state, finished)
}

#[test]
fn the_last_scan_is_opened_after_a_restart_as_a_finished_scan_with_what_the_finished_event_said() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    let (_, finished) = saved_scan(data.path(), dir.path());
    let ScanEvent::Finished {
        root_id,
        totals,
        elapsed_ms,
        warnings,
        ..
    } = finished
    else {
        panic!("the scan did not finish: {finished:?}");
    };

    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    let opened = open_now(&restarted).unwrap().expect("the scan is kept");

    assert_eq!(
        (opened.root_id, opened.totals, opened.elapsed_ms),
        (root_id, totals, elapsed_ms)
    );
    assert_eq!(opened.warnings, warnings);
    assert_eq!(opened.options, options(dir.path()));
    assert!(!opened.outdated);
    assert_eq!(
        {
            let tree = restarted.scans.tree(opened.scan_id).unwrap();
            tree.scanned_at() + tree.elapsed().as_secs() as i64
        },
        opened.scanned_at,
        "the time the scan finished"
    );
    assert!(!restarted.scans.is_outdated(opened.scan_id));
    assert_eq!(restarted.scans.tree(opened.scan_id).unwrap().totals().files, 2);
}

#[test]
fn the_dto_is_camel_case_and_leaves_out_empty_warnings() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    saved_scan(data.path(), dir.path());

    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    let opened = open_now(&restarted).unwrap().unwrap();

    let value = serde_json::to_value(&opened).unwrap();
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "elapsedMs",
            "options",
            "outdated",
            "rootId",
            "scanId",
            "scannedAt",
            "totals"
        ]
    );
    assert_eq!(
        value["options"],
        json!({ "root": options(dir.path()).root, "oneFileSystem": false, "excluded": [] })
    );
    assert_eq!(value["rootId"], 0);
    assert!(value["scannedAt"].is_i64());
}

#[test]
fn a_cleanup_after_the_scan_is_seen_as_outdated_after_a_restart() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    let (state, _) = saved_scan(data.path(), dir.path());

    state.scans.disk_changed();

    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    let opened = open_now(&restarted).unwrap().unwrap();
    assert!(opened.outdated);
    assert!(restarted.scans.is_outdated(opened.scan_id));
}

#[test]
fn a_cleanup_after_opening_marks_the_kept_scan_outdated_too() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    saved_scan(data.path(), dir.path());
    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    assert!(!open_now(&restarted).unwrap().unwrap().outdated);

    restarted.scans.disk_changed();

    let again = AppState::for_test().with_last_scan_dir(data.path());
    assert!(open_now(&again).unwrap().unwrap().outdated);
}

#[test]
fn a_scan_that_follows_the_last_one_replaces_it() {
    let (data, first) = (tempfile::tempdir().unwrap(), sample_dir());
    let (state, _) = saved_scan(data.path(), first.path());
    let second = tempfile::tempdir().unwrap();
    fs::write(second.path().join("only.txt"), b"x").unwrap();

    scan_to_the_end(&state, options(second.path()), None);
    let deadline = Instant::now() + Duration::from_secs(30);
    let opened = loop {
        let restarted = AppState::for_test().with_last_scan_dir(data.path());
        match open_now(&restarted).unwrap() {
            Some(dto) if dto.options.root == options(second.path()).root => break dto,
            _ => assert!(Instant::now() < deadline, "the second scan was not saved"),
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    assert_eq!(opened.totals.files, 1);
}

#[test]
fn opening_allows_the_folder_again_so_that_a_new_scan_with_the_same_options_starts() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    saved_scan(data.path(), dir.path());
    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    assert_eq!(
        restarted.allowed_root(&options(dir.path()).root),
        None,
        "not allowed before"
    );

    let opened = open_now(&restarted).unwrap().unwrap();

    let pinned = restarted.allowed_root(&opened.options.root).expect("allowed now");
    let tree = restarted.scans.tree(opened.scan_id).unwrap();
    let scanned = tree.root_identity();
    assert_eq!(pinned, Some((scanned.dev, scanned.ino)), "pinned to the folder");
    let rescanned = scan_to_the_end(&restarted, opened.options.clone(), pinned);
    assert!(matches!(rescanned, ScanEvent::Finished { .. }), "{rescanned:?}");
}

#[test]
fn opening_records_nothing_in_the_history() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    let (scanned, _) = saved_scan(data.path(), dir.path());
    let history = |state: &AppState| tauri::async_runtime::block_on(overview(state, 30)).unwrap();
    // Linux records a scan of a temporary folder under its volume; elsewhere the test runner's folder may not
    // be on a volume the history knows. What matters below holds everywhere.
    if cfg!(target_os = "linux") {
        assert!(history(&scanned).volume.is_some(), "a scan that ran is on record");
    }

    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    open_now(&restarted).unwrap().unwrap();

    assert!(
        history(&restarted).volume.is_none(),
        "a scan that was opened is not"
    );
}

#[test]
fn nothing_is_opened_when_nothing_was_kept_or_it_cannot_be_used() {
    let data = tempfile::tempdir().unwrap();
    let none = AppState::for_test();
    let empty = AppState::for_test().with_last_scan_dir(data.path());

    assert_eq!(open_now(&none).unwrap(), None, "no data folder");
    assert_eq!(open_now(&empty).unwrap(), None, "nothing saved");

    let dir = sample_dir();
    saved_scan(data.path(), dir.path());
    fs::write(data.path().join("last-scan.bin"), b"junk").unwrap();
    let damaged = AppState::for_test().with_last_scan_dir(data.path());
    assert_eq!(open_now(&damaged).unwrap(), None, "a damaged tree");
    assert!(
        damaged.allowed_root(&options(dir.path()).root).is_none(),
        "and no folder is allowed for it"
    );
}

#[test]
fn nothing_is_opened_when_the_folder_is_gone_or_replaced_by_another() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    saved_scan(data.path(), dir.path());
    let root = dir.path().to_path_buf();
    drop(dir);
    assert!(!root.exists());

    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    assert_eq!(open_now(&restarted).unwrap(), None, "the folder is gone");

    // A folder at the same path is not the same folder as the one scanned only when it is on another
    // path: a moved scan (the sidecar names a path the tree was not scanned at) is not opened either.
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    let other = sample_dir();
    saved_scan(data.path(), dir.path());
    let sidecar_path = data.path().join("last-scan.json");
    let mut sidecar: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    sidecar["options"]["root"] = json!(other.path().to_string_lossy());
    fs::write(&sidecar_path, serde_json::to_vec(&sidecar).unwrap()).unwrap();
    let restarted = AppState::for_test().with_last_scan_dir(data.path());

    assert_eq!(
        open_now(&restarted).unwrap(),
        None,
        "the files do not belong together"
    );
}

#[test]
fn nothing_is_opened_while_a_scan_runs() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    saved_scan(data.path(), dir.path());
    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    let (running, _) = restarted.scans.begin().unwrap();

    assert_eq!(open_now(&restarted).unwrap(), None);
    assert!(
        restarted.scans.restore_pool(None).is_none(),
        "no ticket while a scan runs"
    );

    restarted.scans.abandon(running);
    assert!(open_now(&restarted).unwrap().is_some());
}

#[test]
fn a_scan_started_while_the_last_one_is_read_is_not_evicted_by_it() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    saved_scan(data.path(), dir.path());
    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    let scans = Arc::clone(&restarted.scans);
    let started_meanwhile = Arc::new(Mutex::new(None));

    // The user starts a scan and it finishes while the tree is being read.
    let record = Arc::clone(&started_meanwhile);
    let opened = tauri::async_runtime::block_on(open_with(
        &restarted,
        Arc::new(move |_| {
            let (id, _) = scans.begin().unwrap();
            scans.finish(id, scan_fake(&FakeFs::new(), false)).unwrap();
            *record.lock().unwrap() = Some(id);
            true
        }),
    ))
    .unwrap();

    assert_eq!(opened, None);
    let id = started_meanwhile.lock().unwrap().expect("the scan ran");
    assert!(restarted.scans.tree(id).is_ok(), "the scan the user made stays");
    assert!(
        restarted.allowed_root(&options(dir.path()).root).is_none(),
        "and the folder is not allowed for a scan that was not opened"
    );
}

#[test]
fn a_restore_is_refused_for_a_ticket_older_than_a_scan() {
    let registry = crate::state::ScanRegistry::default();
    let ticket = registry.restore_pool(None).unwrap();
    let (id, _) = registry.begin().unwrap();
    registry.abandon(id);

    let restored = registry.restore(scan_fake(&FakeFs::new(), false), false, None, &ticket);

    assert_eq!(restored.unwrap(), None);
    let fresh = registry.restore_pool(None).unwrap();
    assert!(
        registry
            .restore(scan_fake(&FakeFs::new(), false), false, None, &fresh)
            .unwrap()
            .is_some()
    );
}

#[test]
fn opening_evicts_the_finished_scan_that_was_there_and_keeps_the_memory_budget() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    saved_scan(data.path(), dir.path());
    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    let (earlier, _) = restarted.scans.begin().unwrap();
    restarted
        .scans
        .finish(earlier, scan_fake(&FakeFs::new(), false))
        .unwrap();

    let opened = open_now(&restarted).unwrap().unwrap();

    assert_ne!(opened.scan_id, earlier);
    assert_eq!(
        restarted.scans.tree(earlier).unwrap_err().code(),
        ApiErrorCode::NotFound
    );

    let tree = restarted.scans.tree(opened.scan_id).unwrap();
    let held = tree.memory_bytes() as u64;
    let ticket = restarted.scans.restore_pool(Some(8 << 30)).unwrap();
    assert_eq!(
        ticket.pool,
        platter_platform::memory::memory_budget_for((8 << 30) + held)
    );
    let mut big = FakeFs::new();
    for index in 0..500 {
        big.file(&format!("/many/file-{index}"), 1);
    }
    let too_large = restarted
        .scans
        .restore(scan_fake(&big, false), false, Some(0), &ticket)
        .unwrap_err();
    assert_eq!(too_large.code(), ApiErrorCode::Invalid);
    assert!(
        restarted.scans.tree(opened.scan_id).is_ok(),
        "a refused restore evicts nothing"
    );
}

/// The sidecar as it is on disk.
fn kept(data: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(data.join("last-scan.json")).unwrap()).unwrap()
}

#[test]
fn a_scan_whose_mounts_are_not_the_ones_it_saw_is_opened_outdated_and_the_file_is_not_rewritten() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    saved_scan(data.path(), dir.path());
    let restarted = AppState::for_test().with_last_scan_dir(data.path());

    let opened = tauri::async_runtime::block_on(open_with(&restarted, Arc::new(|_| false)))
        .unwrap()
        .unwrap();

    assert!(opened.outdated);
    assert!(restarted.scans.is_outdated(opened.scan_id));
    assert_eq!(
        kept(data.path())["outdated"],
        false,
        "what is kept is not changed by looking"
    );
    assert_eq!(opened.totals.files, 2, "the scan is there to look at");
}

/// A probe that gives every mount another id, as a restart does. Linux only, like the test that uses it: only
/// Linux reports mount ids.
#[cfg(target_os = "linux")]
struct Remounted<'a>(&'a platter_platform::Platform);

#[cfg(target_os = "linux")]
impl EntryProbe for Remounted<'_> {
    fn probe(&self, path: &[u8]) -> std::io::Result<Vec<ProbedEntry>> {
        let mut found = self.0.entry_probe.probe(path)?;
        for entry in &mut found {
            entry.mount_id = entry.mount_id.map(|id| id + 1000);
        }

        Ok(found)
    }
}

#[cfg(target_os = "linux")]
#[test]
fn the_mounts_of_a_scan_are_the_same_until_a_mount_id_changes() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    let (state, _) = saved_scan(data.path(), dir.path());
    let tree = state.last_scan.load(u64::MAX).unwrap().tree;
    assert!(tree.root_identity().mount_id.is_some(), "the scan saw a mount id");

    assert!(mounts_unchanged(&state.platform.entry_probe, &tree));
    assert!(
        !mounts_unchanged(&Remounted(&state.platform), &tree),
        "after a restart"
    );
}

/// A probe that answers from a list of `(path, entry)`, nothing for `/`, and fails for any other path.
struct Listed(Vec<(Vec<u8>, ProbedEntry)>);

impl EntryProbe for Listed {
    fn probe(&self, path: &[u8]) -> std::io::Result<Vec<ProbedEntry>> {
        if path == b"/" {
            return Ok(Vec::new());
        }

        match self.0.iter().find(|(known, _)| known == path) {
            Some((_, entry)) => Ok(vec![*entry]),
            None => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
        }
    }
}

#[test]
fn an_entered_mount_that_is_not_the_one_seen_makes_the_scan_outdated() {
    let mut fs = FakeFs::new();
    fs.same_filesystem_mount("/mnt/sub").file("/mnt/sub/a", 5);
    let tree = scan_fake(&fs, false);
    let (id, mount) = tree.entered_mounts().next().expect("a mount was entered");
    let path = tree.path_bytes(id);
    let entry = |ino| ProbedEntry {
        meta: platter_core::EntryMeta {
            dev: mount.dev,
            ino,
            ..fs_meta()
        },
        mount_id: None,
        birth: None,
    };

    assert!(mounts_unchanged(
        &Listed(vec![(path.clone(), entry(mount.ino))]),
        &tree
    ));
    assert!(
        !mounts_unchanged(&Listed(vec![(path.clone(), entry(mount.ino + 1))]), &tree),
        "another folder"
    );
    assert!(
        !mounts_unchanged(&Listed(Vec::new()), &tree),
        "gone, or not to be looked at"
    );
}

fn fs_meta() -> platter_core::EntryMeta {
    platter_core::EntryMeta {
        kind: platter_core::EntryKind::Dir,
        allocated: 0,
        apparent: 0,
        mtime: 0,
        dev: 0,
        ino: 0,
        nlink: 1,
        is_mount_root: true,
    }
}

#[test]
fn a_folder_that_was_replaced_by_another_is_not_opened_and_not_allowed() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    saved_scan(data.path(), dir.path());
    let moved = tempfile::tempdir().unwrap();
    fs::rename(dir.path(), moved.path().join("old")).unwrap();
    fs::create_dir(dir.path()).unwrap();
    let restarted = AppState::for_test().with_last_scan_dir(data.path());

    assert_eq!(open_now(&restarted).unwrap(), None);
    assert!(restarted.allowed_root(&options(dir.path()).root).is_none());
}

#[test]
fn a_folder_that_was_allowed_already_keeps_what_it_was_allowed_with() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    saved_scan(data.path(), dir.path());
    let root = options(dir.path()).root;

    for earlier in [None, Some((7, 8))] {
        let restarted = AppState::for_test().with_last_scan_dir(data.path());
        restarted.allow_picked(root.clone(), earlier);

        assert!(open_now(&restarted).unwrap().is_some());

        assert_eq!(restarted.allowed_root(&root), Some(earlier));
    }
}

#[test]
fn a_cleanup_that_lands_while_the_scan_is_being_kept_leaves_it_outdated() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    let state = AppState::for_test().with_last_scan_dir(data.path());
    let scans = Arc::clone(&state.scans);

    // The cleanup finishes the moment the scan has: after the save began, before it was written.
    scan_then(&state, options(dir.path()), None, move || scans.disk_changed());
    wait_for_the_save(&state);

    let restarted = AppState::for_test().with_last_scan_dir(data.path());
    assert!(open_now(&restarted).unwrap().unwrap().outdated);
}

#[test]
fn the_kept_scan_is_outdated_as_soon_as_a_cleanup_or_an_undo_is_about_to_change_the_disk() {
    let (data, dir) = (tempfile::tempdir().unwrap(), sample_dir());
    let (state, _) = saved_scan(data.path(), dir.path());
    assert_eq!(kept(data.path())["outdated"], false);

    state.scans.disk_will_change();

    assert_eq!(kept(data.path())["outdated"], true);
}

#[cfg(target_os = "linux")]
#[test]
fn a_cleanup_marks_the_kept_scan_before_it_moves_anything() {
    use crate::commands::cleanup::tests::{Approve, plan_of, project, scanned};
    use crate::commands::cleanup::{CleanupEventDto, start};

    let data = tempfile::tempdir().unwrap();
    let state = AppState::for_test().with_last_scan_dir(data.path());
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    wait_for_the_save(&state);
    assert_eq!(kept(data.path())["outdated"], false);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();

    // What the sidecar says when the cleanup reports its first event, which is before it is over.
    let (sender, receiver) = mpsc::channel();
    let dir = data.path().to_path_buf();
    tauri::async_runtime::block_on(start(
        &state,
        plan.plan_id,
        Arc::new(move |event| {
            let sidecar: serde_json::Value =
                serde_json::from_slice(&fs::read(dir.join("last-scan.json")).unwrap()).unwrap();
            let _ = sender.send((
                matches!(event, CleanupEventDto::Finished { .. }),
                sidecar["outdated"].clone(),
            ));
        }),
    ))
    .unwrap();

    let (_, first) = receiver.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(first, true);
}

struct Answer(bool);

impl crate::commands::cleanup::Confirm for Answer {
    fn confirm(&self, question: &Question) -> bool {
        assert_eq!(question.accept, "Forget");
        self.0
    }
}

fn forget_now(state: &AppState) -> CommandResult<()> {
    tauri::async_runtime::block_on(forget(state))
}

#[test]
fn forgetting_deletes_the_kept_scan_once_the_user_confirms_and_the_next_scan_is_kept_again() {
    let data = tempfile::tempdir().unwrap();
    let dir = sample_dir();
    let (state, _) = saved_scan(data.path(), dir.path());
    state.set_confirm(Arc::new(Answer(true)));

    forget_now(&state).expect("forgotten");

    assert!(!data.path().join("last-scan.bin").exists());
    assert!(!data.path().join("last-scan.json").exists());
    assert!(state.last_scan.load(u64::MAX).is_none());
    assert_eq!(open_now(&state).unwrap(), None);

    // The next scan is kept as before.
    scan_to_the_end(&state, options(dir.path()), None);
    wait_for_the_save(&state);
}

#[test]
fn forgetting_keeps_the_scan_when_the_user_says_no() {
    let data = tempfile::tempdir().unwrap();
    let dir = sample_dir();
    let (state, _) = saved_scan(data.path(), dir.path());
    state.set_confirm(Arc::new(Answer(false)));

    let refused = forget_now(&state).unwrap_err();

    assert_eq!(refused.code(), ApiErrorCode::Cancelled);
    assert!(state.last_scan.load(u64::MAX).is_some());
}

#[test]
fn a_save_that_began_before_forgetting_does_not_bring_the_scan_back() {
    let data = tempfile::tempdir().unwrap();
    let dir = sample_dir();
    let state = AppState::for_test().with_last_scan_dir(data.path());
    let store = Arc::clone(&state.last_scan);

    // Forgotten right as the scan's save begins: the save is dropped, or what it wrote is deleted.
    scan_then(&state, options(dir.path()), None, move || store.forget().unwrap());
    std::thread::sleep(Duration::from_millis(500));

    assert!(state.last_scan.load(u64::MAX).is_none());
    assert!(!data.path().join("last-scan.json").exists());
}

#[test]
fn forgetting_with_nothing_kept_is_fine() {
    let data = tempfile::tempdir().unwrap();
    let state = AppState::for_test().with_last_scan_dir(data.path());
    state.set_confirm(Arc::new(Answer(true)));

    forget_now(&state).expect("nothing to forget is not an error");
}
