// These tests build the Linux layout (a home with `/` paths, the freedesktop trash, symbolic links), so the
// tests themselves run on Unix; what is Windows-specific is tested in `cleanup/wording.rs`, in `platter-cleanup` and in
// `platter-platform` (the Recycle Bin, on Windows only).
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::fs;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use std::sync::atomic::AtomicUsize;

use platter_cleanup::CleanupPlan;
use platter_core::dto::ApiErrorCode;
use platter_core::{EntryProbe, EntryRemover, ExpectedEntry};
use platter_history::MethodKind;

use super::*;
use crate::commands::cleanup::tests::{Approve, Dialog, plan_of, project, scanned, start_of, until_finished};
use crate::commands::cleanup::{CleanupEventDto, Confirm, undo_run};

fn result(rule_id: &'static str, path: &str, allocated: u64, outcome: TargetOutcome) -> TargetResult {
    TargetResult {
        rule_id,
        path: path.to_owned(),
        exact: path.as_bytes().to_vec(),
        identity: (1, 2),
        is_dir: false,
        allocated,
        outcome,
    }
}

fn removed(rule_id: &'static str, allocated: u64) -> TargetResult {
    result(
        rule_id,
        "/x/target",
        allocated,
        TargetOutcome::Removed(Default::default()),
    )
}

fn trashed_result(rule_id: &'static str, path: &str, allocated: u64) -> TargetResult {
    result(
        rule_id,
        path,
        allocated,
        TargetOutcome::Trashed(b"/t/files/x".to_vec()),
    )
}

fn report(freed: Option<i128>) -> CleanupReport {
    CleanupReport {
        freed,
        ..CleanupReport::default()
    }
}

#[cfg(unix)]
#[test]
fn a_run_that_touched_nothing_is_not_recorded() {
    let results = [
        result(
            "rust-target",
            "/x/a",
            5,
            TargetOutcome::Skipped("changed".to_owned()),
        ),
        result("rust-target", "/x/b", 5, TargetOutcome::Failed("busy".to_owned())),
    ];

    let described = describe_run(&[("rust-target", CleanupAction::Delete)], &results, None, 1);

    assert_eq!(described, None);
}

#[cfg(unix)]
#[test]
fn what_was_freed_is_the_measured_space_floored_at_zero_else_what_the_scan_counted() {
    let groups = [("rust-target", CleanupAction::Delete)];
    let results = [removed("rust-target", 300), removed("rust-target", 200)];

    let measured = describe_run(&groups, &results, Some(&report(Some(450))), 1).unwrap();
    let negative = describe_run(&groups, &results, Some(&report(Some(-90))), 1).unwrap();
    let unmeasured = describe_run(&groups, &results, Some(&report(None)), 1).unwrap();
    let panicked = describe_run(&groups, &results, None, 1).unwrap();

    assert_eq!((measured.freed, measured.freed_measured), (450, Some(450)));
    assert_eq!((negative.freed, negative.freed_measured), (0, Some(-90)));
    assert_eq!((unmeasured.freed, unmeasured.freed_measured), (500, None));
    assert_eq!(panicked.freed, 500);
}

#[cfg(unix)]
#[test]
fn a_run_that_only_trashed_frees_nothing_but_records_what_it_moved() {
    let groups = [("old-downloads", CleanupAction::Trash)];
    let results = [trashed_result("old-downloads", "/home/u/a.iso", 700)];

    // Free space moved by something else meanwhile does not count as freed by the trash.
    let described = describe_run(&groups, &results, Some(&report(Some(4096))), 1).unwrap();

    assert_eq!((described.freed, described.trashed), (0, 700));
    assert_eq!(described.items.len(), 1);
    assert_eq!(described.items[0].original, b"/home/u/a.iso");
    assert_eq!(described.items[0].trashed, b"/t/files/x");
    assert_eq!(
        described
            .methods
            .iter()
            .map(|method| method.kind)
            .collect::<Vec<_>>(),
        [MethodKind::Trash]
    );
    assert_eq!(described.what, ["Old downloads"]);
}

#[test]
fn an_entry_that_is_gone_without_a_restorable_item_is_recorded_as_deleted_never_as_trashed() {
    let groups = [("win-temp", CleanupAction::TrashChildren)];
    let only_lost = [result(
        "win-temp",
        "C:\\Users\\u\\AppData\\Local\\Temp\\huge.bin",
        900,
        TargetOutcome::NotRecycled("it is gone but not in the Recycle Bin".to_owned()),
    )];

    let described = describe_run(&groups, &only_lost, None, 1).unwrap();

    assert!(described.items.is_empty(), "there is nothing to put back");
    assert_eq!(described.trashed, 0);
    assert_eq!(
        described
            .methods
            .iter()
            .map(|method| method.kind)
            .collect::<Vec<_>>(),
        [MethodKind::Deleted]
    );

    // Beside a child that did reach the Recycle Bin, both are told, and only that child can be restored.
    let mixed = [
        trashed_result("win-temp", "C:\\Users\\u\\AppData\\Local\\Temp\\a.tmp", 100),
        only_lost[0].clone(),
    ];
    let described = describe_run(&groups, &mixed, None, 1).unwrap();

    assert_eq!(described.items.len(), 1);
    assert_eq!(
        described
            .methods
            .iter()
            .map(|method| method.kind)
            .collect::<Vec<_>>(),
        [MethodKind::Trash, MethodKind::Deleted]
    );
}

#[test]
fn an_entry_whose_fate_is_unknown_is_not_recorded_nor_counted_as_freed() {
    let groups = [("win-temp", CleanupAction::TrashChildren)];
    let unknown = result(
        "win-temp",
        "C:\\Users\\u\\AppData\\Local\\Temp\\huge.bin",
        900,
        TargetOutcome::Unconfirmed("it may be waiting on a prompt".to_owned()),
    );

    assert!(
        describe_run(&groups, std::slice::from_ref(&unknown), None, 1).is_none(),
        "nothing is known to have happened"
    );

    let mixed = [removed("win-temp", 100), unknown];
    let described = describe_run(&groups, &mixed, None, 1).unwrap();

    assert!(described.items.is_empty());
    assert_eq!(described.freed, 100, "only what is known to be gone counts");
}

#[cfg(unix)]
#[test]
fn a_run_lists_each_group_that_did_something_once() {
    let groups = [
        ("rust-target", CleanupAction::Delete),
        ("old-downloads", CleanupAction::Trash),
        ("npm-cache", CleanupAction::EmptyFolder),
        ("trash", CleanupAction::EmptyTrash),
    ];
    let results = [
        removed("rust-target", 10),
        trashed_result("old-downloads", "/home/u/a", 20),
        // Skipped: its group did nothing and is not named.
        result(
            "npm-cache",
            "/home/u/.npm",
            30,
            TargetOutcome::Skipped("changed".to_owned()),
        ),
        removed("trash", 40),
    ];

    let described = describe_run(&groups, &results, Some(&report(Some(50))), 7).unwrap();

    assert_eq!(described.what, ["Rust build output", "Old downloads", "Trash"]);
    assert_eq!(
        described
            .methods
            .iter()
            .map(|method| method.kind)
            .collect::<Vec<_>>(),
        [MethodKind::Deleted, MethodKind::Trash, MethodKind::EmptiedTrash]
    );
    assert!(described.emptied_trash);
    assert_eq!((described.at, described.freed, described.trashed), (7, 50, 20));
}

#[cfg(unix)]
#[test]
fn marked_items_are_named_by_their_folder_names_and_the_rest_counted() {
    let paths: Vec<String> = (1..=8)
        .map(|number| format!("/home/u/Videos/clip{number}/"))
        .collect();
    let results: Vec<TargetResult> = paths.iter().map(|path| trashed_result(MARKED, path, 1)).collect();

    let described = describe_run(&[(MARKED, CleanupAction::Trash)], &results, None, 1).unwrap();

    assert_eq!(
        described.what,
        ["clip1", "clip2", "clip3", "clip4", "clip5", "3 more"]
    );
}

// ---- The service ----

/// Two disks: `/` and `/mnt/data`.
fn two_disks() -> FixedVolumes {
    FixedVolumes(vec![
        volume("/", "Root", 1000, 400),
        volume("/mnt/data", "Data", 5000, 1000),
    ])
}

#[cfg(unix)]
#[test]
fn a_path_belongs_to_the_disk_with_the_longest_mount_point_holding_it() {
    let history = HistoryService::in_memory(two_disks());

    let on = |path: &str| history.volume_of(Path::new(path)).map(|volume| volume.label);

    assert_eq!(on("/home/u").as_deref(), Some("Root"));
    assert_eq!(on("/mnt/data/photos").as_deref(), Some("Data"));
    assert_eq!(on("/mnt/data").as_deref(), Some("Data"));
    assert_eq!(
        on("/mnt/database").as_deref(),
        Some("Root"),
        "a prefix of the name is not a parent"
    );
    assert!(
        HistoryService::in_memory(FixedVolumes::default())
            .volume_of(Path::new("/x"))
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn a_finished_scan_records_its_disk_and_the_first_usage_of_the_day() {
    let state = AppState::for_test();
    let (_dir, root) = project();

    scanned(&state, &root);
    scanned(&state, &root);
    let overview = tauri::async_runtime::block_on(overview(&state, 90)).unwrap();

    assert_eq!(
        overview.volume,
        Some(HistoryVolumeDto {
            id: "/".to_owned(),
            label: "Root".to_owned()
        })
    );
    assert_eq!(overview.samples.len(), 1, "one sample a day however many scans");
    assert_eq!((overview.samples[0].used, overview.samples[0].total), (600, 1000));
    assert_eq!(overview.samples[0].day, overview.today);
    assert_eq!(overview.without, [600]);
    assert!(overview.thin && overview.problem.is_none());
    assert!(overview.growth_per_day.is_none() && overview.days_until_full.is_none());
}

#[cfg(unix)]
#[test]
fn the_sampler_samples_the_stored_disks_that_are_mounted_and_keeps_the_first_of_the_day() {
    let state = AppState::for_test();
    let history = HistoryService::in_memory(two_disks());
    let (_dir, root) = project();
    // Only a disk the app scanned is stored.
    history.record_scan(&root, &scanned_totals());
    history.record_scan(Path::new("/mnt/data/x"), &scanned_totals());

    history.sample_all();
    let overview = history.overview(30, &state.platform.entry_probe);

    // The latest scan was on /mnt/data.
    assert_eq!(
        overview.volume.map(|volume| volume.label).as_deref(),
        Some("Data")
    );
    assert_eq!(overview.samples.len(), 1);
    assert_eq!(
        (overview.samples[0].used, overview.samples[0].total),
        (4000, 5000)
    );
}

fn scanned_totals() -> Totals {
    Totals::default()
}

#[cfg(unix)]
#[test]
fn days_outside_one_to_366_are_refused() {
    let state = AppState::for_test();

    for days in [-1, 0, 367, 1_000_000] {
        let error = tauri::async_runtime::block_on(overview(&state, days)).unwrap_err();
        assert_eq!(error.code(), ApiErrorCode::Invalid, "{days}");
    }
    assert!(tauri::async_runtime::block_on(overview(&state, 1)).is_ok());
    assert!(tauri::async_runtime::block_on(overview(&state, 366)).is_ok());
}

#[cfg(unix)]
#[test]
fn an_empty_history_is_an_overview_with_nothing_in_it() {
    let state = AppState::for_test();

    let overview = tauri::async_runtime::block_on(overview(&state, 90)).unwrap();
    let wire = serde_json::to_value(&overview).unwrap();

    assert!(overview.volume.is_none() && overview.thin);
    assert_eq!(wire["samples"], serde_json::json!([]));
    assert_eq!(wire["cleanupCount"], 0);
    for absent in ["volume", "growthPerDay", "daysUntilFull", "problem"] {
        assert!(wire.get(absent).is_none(), "{absent}");
    }
}

// ---- Cleanups recorded by the worker ----

#[cfg(unix)]
#[test]
fn a_cleanup_that_deleted_something_is_recorded_when_it_ends() {
    let state = AppState::for_test();
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();
    let (started, receiver) = start_of(&state, plan.plan_id);
    started.unwrap();
    until_finished(&receiver);

    let overview = tauri::async_runtime::block_on(overview(&state, 90)).unwrap();

    assert_eq!(overview.cleanup_count, 1);
    let row = &overview.cleanups[0];
    assert_eq!(row.what, ["Rust build output"]);
    assert_eq!(
        row.methods.iter().map(|method| method.kind).collect::<Vec<_>>(),
        [MethodKind::Deleted]
    );
    assert_eq!((row.status, row.restorable), (Status::Permanent, 0));
    assert_eq!(row.day, overview.today);
    assert_eq!(overview.freed, row.freed);

    let wire = serde_json::to_value(row).unwrap();
    assert_eq!(wire["status"], "permanent");
    assert_eq!(wire["methods"][0], serde_json::json!({ "kind": "deleted" }));
}

#[cfg(unix)]
#[test]
fn a_history_that_cannot_be_used_stops_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let garbage = b"definitely not a database ".repeat(200);
    fs::write(dir.path().join("history.sqlite"), &garbage).unwrap();
    let mut state = AppState::for_test();
    state.history = Arc::new(HistoryService::in_dir(Some(dir.path()), FixedVolumes::root()));
    state.set_confirm(Arc::new(Approve));
    let (_project_dir, root) = project();

    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();
    let (started, receiver) = start_of(&state, plan.plan_id);
    started.unwrap();
    until_finished(&receiver);

    assert!(!root.join("p/target").exists(), "the cleanup ran");
    let overview = tauri::async_runtime::block_on(overview(&state, 90)).unwrap();
    assert_eq!(overview.problem.as_deref(), Some("the history file is damaged"));
    assert!(overview.thin && overview.samples.is_empty() && overview.cleanups.is_empty());
    let restore = tauri::async_runtime::block_on(restore_run(&state, 1)).unwrap_err();
    assert_eq!(restore.code(), ApiErrorCode::Invalid);
    assert_eq!(
        fs::read(dir.path().join("history.sqlite")).unwrap(),
        garbage,
        "left as it was"
    );
}

#[cfg(unix)]
#[test]
fn the_history_file_is_private_and_a_link_at_its_name_is_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let place = dir.path().join("data/platter");

    let history = HistoryService::in_dir(Some(&place), FixedVolumes::root());

    assert_eq!(history.problem(), None);
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&place), 0o700);
    assert_eq!(mode(&place.join("history.sqlite")), 0o600);

    let other = dir.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("real.sqlite"), b"").unwrap();
    std::os::unix::fs::symlink(other.join("real.sqlite"), other.join("history.sqlite")).unwrap();
    let linked = HistoryService::in_dir(Some(&other), FixedVolumes::root());
    assert!(linked.problem().is_some_and(|problem| problem.contains("link")));
    assert!(
        HistoryService::in_dir(None, FixedVolumes::root())
            .problem()
            .is_some()
    );
}

// ---- Putting things back ----

/// A trash in a temp folder, with files moved into it by the real remover. Never the user's trash.
struct Trashed {
    _dir: tempfile::TempDir,
    home: PathBuf,
    results: Vec<TargetResult>,
}

fn trash_files(state: &AppState, names: &[&str]) -> Trashed {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let home = base.join("home");
    let trash = base.join("trash");
    fs::create_dir(&home).unwrap();

    let mut results = Vec::new();
    for name in names {
        let path = home.join(name);
        fs::write(&path, vec![1u8; 10_000]).unwrap();
        let exact = path.as_os_str().as_encoded_bytes().to_vec();

        let found = state.platform.entry_probe.probe(&exact).unwrap();
        let meta = found.last().unwrap().meta;
        let expected = ExpectedEntry {
            dev: meta.dev,
            ino: meta.ino,
            mount_id: None,
            birth: None,
            is_dir: false,
            // The folders above the temp folder (a mount of its own, often) are not the trash's business.
            root_depth: base
                .components()
                .filter(|part| matches!(part, std::path::Component::Normal(_)))
                .count(),
        };
        let to = state
            .platform
            .entry_remover
            .trash(&exact, &expected, &trash)
            .unwrap();

        results.push(TargetResult {
            rule_id: "old-downloads",
            path: path.to_string_lossy().into_owned(),
            exact,
            identity: (meta.dev, meta.ino),
            is_dir: false,
            allocated: meta.allocated,
            outcome: TargetOutcome::Trashed(to),
        });
    }

    Trashed {
        _dir: dir,
        home,
        results,
    }
}

/// A state whose user approves every question.
fn approving() -> AppState {
    let state = AppState::for_test();
    state.set_confirm(Arc::new(Approve));

    state
}

/// A state whose user answers as `dialog` does.
fn asking(dialog: &Arc<Dialog>) -> AppState {
    let state = AppState::for_test();
    state.set_confirm(Arc::clone(dialog) as Arc<dyn Confirm>);

    state
}

/// Records `trashed` as a finished cleanup, in the history and in the registry (as the worker does).
fn recorded(state: &AppState, trashed: &Trashed) -> i64 {
    let groups = [("old-downloads", CleanupAction::Trash)];
    let id = state
        .history
        .record_run(
            &trashed.home,
            &groups,
            &trashed.results,
            Some(&report(Some(0))),
            |_| {},
        )
        .expect("recorded");
    state.cleanups.record(7, Some(id), trashed.results.clone(), false);
    // The page loaded: only what an overview showed can be put back.
    overview_now(state);

    id
}

fn overview_now(state: &AppState) -> HistoryOverviewDto {
    tauri::async_runtime::block_on(overview(state, 30)).unwrap()
}

#[cfg(unix)]
#[test]
fn a_cleanup_that_trashed_is_in_the_trash_until_it_is_restored() {
    let state = approving();
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    let size: u64 = trashed.results.iter().map(|result| result.allocated).sum();
    let id = recorded(&state, &trashed);

    let before = overview_now(&state);

    let row = &before.cleanups[0];
    assert_eq!((row.id, row.status, row.restorable), (id, Status::InTrash, size));
    assert_eq!(row.freed, size, "a trash-only run shows its size");
    assert_eq!((before.freed, before.restorable), (0, size), "and frees nothing");
    assert_eq!(
        before.without,
        [] as [u64; 0],
        "no sample was taken, so no line either"
    );

    let restored = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap();

    assert_eq!(restored.restored, 2);
    assert!(
        restored
            .items
            .iter()
            .all(|item| item.restored && item.error.is_none())
    );
    assert!(trashed.home.join("a.iso").exists() && trashed.home.join("b.iso").exists());
    let after = overview_now(&state);
    assert_eq!(after.cleanups[0].status, Status::Restored);
    assert_eq!(
        (after.restorable, after.freed),
        (0, 0),
        "a restore changes no freed total"
    );
}

#[cfg(unix)]
#[test]
fn an_item_is_restored_once_and_what_came_back_is_not_offered_again() {
    let state = approving();
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    let id = recorded(&state, &trashed);
    // One of them went back by hand: the other is still in the trash.
    let first = &trashed.results[0];
    let TargetOutcome::Trashed(to) = &first.outcome else {
        unreachable!()
    };
    state
        .platform
        .entry_remover
        .restore(
            to,
            &ExpectedEntry {
                dev: first.identity.0,
                ino: first.identity.1,
                mount_id: None,
                birth: None,
                is_dir: false,
                root_depth: 0,
            },
            &first.exact,
        )
        .unwrap();

    let restored = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap();
    let again = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap();

    assert_eq!(restored.items.len(), 2);
    assert_eq!(
        restored.items.iter().filter(|item| item.restored).count(),
        1,
        "the one that was back is refused"
    );
    assert!(
        restored
            .items
            .iter()
            .any(|item| !item.restored && item.error.is_some())
    );
    assert_eq!(restored.restored, 1);
    assert_eq!(
        again.items.len(),
        1,
        "what failed is offered again, what came back is not"
    );
    assert_eq!(again.restored, 0);
}

#[cfg(unix)]
#[test]
fn restoring_an_unknown_cleanup_is_not_found() {
    let state = AppState::for_test();

    let error = tauri::async_runtime::block_on(restore_run(&state, 99)).unwrap_err();

    assert_eq!(error.code(), ApiErrorCode::NotFound);
}

#[cfg(unix)]
#[test]
fn a_restore_from_the_history_takes_the_run_away_from_undo() {
    let state = approving();
    let trashed = trash_files(&state, &["a.iso"]);
    let id = recorded(&state, &trashed);

    tauri::async_runtime::block_on(restore_run(&state, id)).unwrap();

    let undo = tauri::async_runtime::block_on(undo_run(&state, 7)).unwrap_err();
    assert_eq!(undo.code(), ApiErrorCode::NotFound, "nothing is left for undo");
    assert!(trashed.home.join("a.iso").exists());
}

#[cfg(unix)]
#[test]
fn what_a_restore_could_not_put_back_stays_for_undo() {
    let state = approving();
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    let id = recorded(&state, &trashed);
    // Something took the old name of the first one: it is never overwritten.
    fs::write(trashed.home.join("a.iso"), b"new").unwrap();

    let restored = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap();

    assert_eq!(restored.restored, 1);
    assert_eq!(fs::read(trashed.home.join("a.iso")).unwrap(), b"new");
    assert!(trashed.home.join("b.iso").exists());
    fs::remove_file(trashed.home.join("a.iso")).unwrap();
    let undone = tauri::async_runtime::block_on(undo_run(&state, 7)).unwrap();
    assert_eq!(undone.len(), 1, "only the entry that did not come back");
    assert!(undone[0].restored);
    assert!(trashed.home.join("a.iso").exists());
}

#[cfg(unix)]
#[test]
fn an_undo_marks_what_it_put_back_so_the_history_does_not_restore_it_again() {
    let state = approving();
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    let id = recorded(&state, &trashed);

    let undone = tauri::async_runtime::block_on(undo_run(&state, 7)).unwrap();

    assert!(undone.iter().all(|item| item.restored));
    assert_eq!(overview_now(&state).cleanups[0].status, Status::Restored);
    let restored = tauri::async_runtime::block_on(restore_run(&state, id));
    assert_eq!(
        restored.unwrap_err().code(),
        ApiErrorCode::NotFound,
        "nothing is left to put back"
    );
}

#[cfg(unix)]
#[test]
fn nothing_is_put_back_while_a_cleanup_runs_or_something_else_is_being_put_back() {
    let state = approving();
    let trashed = trash_files(&state, &["a.iso"]);
    let id = recorded(&state, &trashed);

    let plan = state.cleanups.store(1, CleanupPlan::default());
    let running = state.cleanups.take_and_reserve(plan, &|| false).unwrap();
    let during_cleanup = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();
    assert_eq!(during_cleanup.code(), ApiErrorCode::Invalid);
    assert!(
        !state.history.is_restoring(),
        "the slot is free again after a refusal"
    );
    drop(running);

    let other = state.history.begin_restore().expect("free");
    let during_restore = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();
    let during_undo = tauri::async_runtime::block_on(undo_run(&state, 7)).unwrap_err();
    assert_eq!(during_restore.code(), ApiErrorCode::Invalid);
    assert_eq!(during_undo.code(), ApiErrorCode::Invalid);
    assert!(state.history.begin_restore().is_none());
    drop(other);

    assert!(!trashed.home.join("a.iso").exists(), "nothing was put back");
    assert_eq!(
        tauri::async_runtime::block_on(restore_run(&state, id))
            .unwrap()
            .restored,
        1
    );
}

#[cfg(unix)]
#[test]
fn a_cleanup_does_not_start_while_things_are_being_put_back() {
    let state = AppState::for_test();
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();

    let restoring = state.history.begin_restore().unwrap();
    let (refused, _events) = start_of(&state, plan.plan_id);
    drop(restoring);

    assert_eq!(refused.unwrap_err().code(), ApiErrorCode::Invalid);
    assert!(root.join("p/target").exists());
    assert!(
        state.cleanups.has_plan(plan.plan_id),
        "the plan waits for the next try"
    );
    let (started, receiver) = start_of(&state, plan.plan_id);
    started.unwrap();
    until_finished(&receiver);
    assert!(!root.join("p/target").exists());
}

// ---- Asking first ----

#[cfg(unix)]
#[test]
fn the_dialog_lists_the_places_that_go_back_with_a_count_and_the_size() {
    let dialog = Dialog::answering(true);
    let state = asking(&dialog);
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    let id = recorded(&state, &trashed);

    tauri::async_runtime::block_on(restore_run(&state, id)).unwrap();

    let asked = dialog.asked();
    assert_eq!(asked.len(), 1);
    let question = &asked[0];
    assert_eq!(
        (question.title.as_str(), question.accept),
        ("Put back?", "Put back")
    );
    for name in ["a.iso", "b.iso"] {
        let original = trashed.home.join(name);
        assert!(
            question.text.contains(&*original.to_string_lossy()),
            "{}",
            question.text
        );
    }
    assert!(
        question.text.contains("• Put back: 2 items, "),
        "{}",
        question.text
    );
    assert!(!question.text.contains(';'), "{}", question.text);
}

#[cfg(unix)]
#[test]
fn an_undo_asks_the_same_question_and_goes_ahead_when_approved() {
    let dialog = Dialog::answering(true);
    let state = asking(&dialog);
    let trashed = trash_files(&state, &["a.iso"]);
    recorded(&state, &trashed);

    let undone = tauri::async_runtime::block_on(undo_run(&state, 7)).unwrap();

    assert!(undone[0].restored);
    assert!(trashed.home.join("a.iso").exists());
    let asked = dialog.asked();
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0]
            .text
            .contains(&*trashed.home.join("a.iso").to_string_lossy()),
        "{}",
        asked[0].text
    );
    assert!(
        asked[0].text.contains("• Put back: 1 item, "),
        "{}",
        asked[0].text
    );
}

#[cfg(unix)]
#[test]
fn a_declined_undo_moves_nothing_and_leaves_the_run_to_undo_and_to_the_history() {
    let dialog = Dialog::answering(false);
    let state = asking(&dialog);
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    let id = recorded(&state, &trashed);

    let declined = tauri::async_runtime::block_on(undo_run(&state, 7)).unwrap_err();

    assert_eq!(declined.code(), ApiErrorCode::Cancelled);
    assert!(!trashed.home.join("a.iso").exists() && !trashed.home.join("b.iso").exists());
    assert!(!state.history.is_restoring(), "the slot is free");
    assert_eq!(overview_now(&state).cleanups[0].status, Status::InTrash);

    dialog.answer(true);
    let undone = tauri::async_runtime::block_on(undo_run(&state, 7)).unwrap();
    assert_eq!(undone.len(), 2, "the run was still there");
    assert!(undone.iter().all(|item| item.restored));
    assert_eq!(overview_now(&state).cleanups[0].status, Status::Restored);
    assert_eq!(id, overview_now(&state).cleanups[0].id);
}

#[cfg(unix)]
#[test]
fn a_declined_restore_from_the_history_moves_nothing_and_leaves_undo_and_the_run_as_they_were() {
    let dialog = Dialog::answering(false);
    let state = asking(&dialog);
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    let id = recorded(&state, &trashed);

    let declined = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();

    assert_eq!(declined.code(), ApiErrorCode::Cancelled);
    assert!(!trashed.home.join("a.iso").exists());
    assert!(!state.history.is_restoring(), "the slot is free");
    assert_eq!(overview_now(&state).cleanups[0].status, Status::InTrash);

    // Undo still has the run, and the history can still offer it.
    dialog.answer(true);
    let again = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap();
    assert_eq!(again.restored, 2);

    let other = Dialog::answering(false);
    let second = asking(&other);
    let more = trash_files(&second, &["c.iso"]);
    recorded(&second, &more);
    tauri::async_runtime::block_on(restore_run(&second, overview_now(&second).cleanups[0].id)).unwrap_err();
    other.answer(true);
    let undone = tauri::async_runtime::block_on(undo_run(&second, 7)).unwrap();
    assert!(
        undone[0].restored,
        "undo still had the run after the history was declined"
    );
}

#[cfg(unix)]
#[test]
fn a_request_that_is_refused_shows_no_dialog() {
    let dialog = Dialog::answering(true);
    let state = asking(&dialog);
    let trashed = trash_files(&state, &["a.iso"]);
    let id = recorded(&state, &trashed);

    let unknown_run = tauri::async_runtime::block_on(restore_run(&state, id + 50)).unwrap_err();
    let unknown_undo = tauri::async_runtime::block_on(undo_run(&state, 99)).unwrap_err();
    let slot = state.history.begin_restore().unwrap();
    let busy_restore = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();
    let busy_undo = tauri::async_runtime::block_on(undo_run(&state, 7)).unwrap_err();
    drop(slot);

    assert_eq!(unknown_run.code(), ApiErrorCode::NotFound);
    assert_eq!(unknown_undo.code(), ApiErrorCode::NotFound);
    assert_eq!(busy_restore.code(), ApiErrorCode::Invalid);
    assert_eq!(busy_undo.code(), ApiErrorCode::Invalid);
    assert!(dialog.asked().is_empty());
}

#[cfg(unix)]
#[test]
fn a_run_that_cannot_be_put_back_any_more_when_the_dialog_closes_moves_nothing() {
    let dialog = Dialog::answering(true);
    let state = asking(&dialog);
    let trashed = trash_files(&state, &["a.iso"]);
    let id = recorded(&state, &trashed);

    // The user takes their time: meanwhile the history no longer shows the run, and the run is gone
    // from the undo registry.
    let history = Arc::clone(&state.history);
    dialog.while_open(move || {
        history
            .shown_nothing(HistoryOverviewDto::empty(30, 0, 0, None))
            .cleanups
            .clear()
    });
    let from_history = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();

    let cleanups = Arc::clone(&state.cleanups);
    dialog.while_open(move || {
        cleanups.take_done(7).unwrap();
    });
    let from_undo = tauri::async_runtime::block_on(undo_run(&state, 7)).unwrap_err();

    assert_eq!(from_history.code(), ApiErrorCode::NotFound);
    assert_eq!(from_undo.code(), ApiErrorCode::NotFound);
    assert!(!trashed.home.join("a.iso").exists(), "nothing was put back");
    assert!(!state.history.is_restoring(), "the slot is free");
}

#[cfg(unix)]
#[test]
fn a_run_whose_paths_change_under_the_same_ids_while_the_dialog_is_open_moves_nothing() {
    let dialog = Dialog::answering(true);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("history.sqlite");
    let mut state = asking(&dialog);
    state.history = Arc::new(HistoryService::in_dir(Some(dir.path()), FixedVolumes::root()));
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    let id = recorded(&state, &trashed);

    // The same rows are there when the dialog closes, but they say something else than they did.
    dialog.while_open(move || platter_history::testing::rename_originals(&file, b".elsewhere"));
    let refused = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();

    assert_eq!(refused.code(), ApiErrorCode::Invalid, "{refused:?}");
    assert_eq!(dialog.asked().len(), 1);
    assert!(!trashed.home.join("a.iso").exists(), "nothing was put back");
    assert!(!trashed.home.join("b.iso").exists());
    assert!(!state.history.is_restoring(), "the slot is free");
}

/// A state with its history in a file and a dialog, with a run of two files in the trash that the
/// overview showed.
fn asked_on_file() -> (AppState, Arc<Dialog>, tempfile::TempDir, PathBuf, Trashed, i64) {
    let dialog = Dialog::answering(true);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("history.sqlite");
    let mut state = asking(&dialog);
    state.history = Arc::new(HistoryService::in_dir(Some(dir.path()), FixedVolumes::root()));
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    let id = recorded(&state, &trashed);

    (state, dialog, dir, file, trashed, id)
}

/// A restore whose rows are changed by `tamper` while its dialog is open moves nothing.
fn refused_after_a_change_while_the_dialog_is_open(tamper: fn(&Path)) {
    let (state, dialog, _dir, file, trashed, id) = asked_on_file();

    dialog.while_open(move || tamper(&file));
    let refused = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();

    assert_eq!(refused.code(), ApiErrorCode::Invalid, "{refused:?}");
    assert_eq!(dialog.asked().len(), 1);
    assert!(!trashed.home.join("a.iso").exists(), "nothing was put back");
    assert!(!trashed.home.join("b.iso").exists());
    assert!(!state.history.is_restoring(), "the slot is free");
}

#[cfg(unix)]
#[test]
fn a_restore_whose_trash_paths_change_while_the_dialog_is_open_moves_nothing() {
    refused_after_a_change_while_the_dialog_is_open(|file| {
        platter_history::testing::rename_trashed(file, b".elsewhere");
    });
}

#[cfg(unix)]
#[test]
fn a_restore_whose_identities_change_while_the_dialog_is_open_moves_nothing() {
    refused_after_a_change_while_the_dialog_is_open(platter_history::testing::change_identities);
}

#[cfg(unix)]
#[test]
fn a_restore_with_fewer_entries_when_the_dialog_closes_moves_nothing() {
    refused_after_a_change_while_the_dialog_is_open(platter_history::testing::drop_first_entry);
}

#[cfg(unix)]
#[test]
fn a_restore_with_more_entries_when_the_dialog_closes_moves_nothing() {
    refused_after_a_change_while_the_dialog_is_open(platter_history::testing::duplicate_entries);
}

#[cfg(unix)]
#[test]
fn a_run_that_empties_the_trash_while_a_restore_dialog_is_open_makes_the_restore_refuse() {
    let (state, dialog, _dir, file, trashed, id) = asked_on_file();
    let emptied = TargetResult {
        rule_id: "trash",
        path: "trash".to_owned(),
        exact: trash_folder_of(&trashed),
        identity: (1, 1),
        is_dir: true,
        allocated: 5000,
        outcome: TargetOutcome::Removed(Default::default()),
    };

    // The record of the run that emptied the trash is written while the dialog about the earlier run is
    // open. (The entries stay in the real trash: only what the history says changes.)
    let history = Arc::clone(&state.history);
    let home = trashed.home.clone();
    dialog.while_open(move || {
        history
            .record_run(
                &home,
                &[("trash", CleanupAction::EmptyTrash)],
                &[emptied],
                Some(&report(Some(5000))),
                |_| {},
            )
            .expect("recorded");
    });
    let refused = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();

    assert_eq!(refused.code(), ApiErrorCode::NotFound, "{refused:?}");
    assert_eq!(platter_history::testing::entries_with_paths(&file), 0);
    assert_eq!(dialog.asked().len(), 1);
    assert!(!trashed.home.join("a.iso").exists(), "nothing was put back");
    assert!(!trashed.home.join("b.iso").exists());
    assert!(!state.history.is_restoring(), "the slot is free");
}

// ---- Paths are not kept longer than they are needed ----

/// A state with its history in a file, so a test can look at what the file holds.
fn on_file() -> (AppState, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mut state = approving();
    state.history = Arc::new(HistoryService::in_dir(Some(dir.path()), FixedVolumes::root()));
    let file = dir.path().join("history.sqlite");

    (state, dir, file)
}

#[cfg(unix)]
fn trash_folder_of(trashed: &Trashed) -> Vec<u8> {
    let TargetOutcome::Trashed(to) = &trashed.results[0].outcome else {
        unreachable!()
    };
    let files = Path::new(std::ffi::OsStr::from_bytes(to)).parent().unwrap();

    files.parent().unwrap().as_os_str().as_encoded_bytes().to_vec()
}

#[cfg(unix)]
#[test]
fn an_item_that_was_put_back_keeps_no_path_in_the_file() {
    let (state, _dir, file) = on_file();
    let trashed = trash_files(&state, &["a.iso", "b.iso", "c.iso"]);
    let id = recorded(&state, &trashed);
    assert_eq!(platter_history::testing::entries_with_paths(&file), 3);

    // One by undo (it knows paths), the others by the history (it knows ids).
    let mut one = trashed.results.clone();
    one.truncate(1);
    state.cleanups.take_done(7).unwrap();
    state.cleanups.record(7, Some(id), one, false);
    tauri::async_runtime::block_on(undo_run(&state, 7)).unwrap();
    assert_eq!(platter_history::testing::entries_with_paths(&file), 2);
    tauri::async_runtime::block_on(restore_run(&state, id)).unwrap();

    assert_eq!(platter_history::testing::entries_with_paths(&file), 0);
    assert_eq!(overview_now(&state).cleanups[0].status, Status::Restored);
}

#[cfg(unix)]
#[test]
fn an_item_found_gone_keeps_no_path_and_the_overview_says_the_same_as_before() {
    let (state, _dir, file) = on_file();
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    recorded(&state, &trashed);
    let TargetOutcome::Trashed(to) = &trashed.results[0].outcome else {
        unreachable!()
    };
    // The first one was emptied from the trash by the user's file manager.
    fs::remove_file(Path::new(std::ffi::OsStr::from_bytes(to))).unwrap();
    assert_eq!(platter_history::testing::entries_with_paths(&file), 2);

    let first = overview_now(&state);
    let second = overview_now(&state);

    assert_eq!(first.cleanups[0].status, Status::InTrash);
    assert_eq!(first.cleanups[0].restorable, trashed.results[1].allocated);
    assert_eq!(
        platter_history::testing::entries_with_paths(&file),
        1,
        "only the one in the trash"
    );
    assert_eq!(first, second, "what is shown does not change");
}

#[cfg(unix)]
#[test]
fn an_item_whose_trash_is_out_of_reach_keeps_its_paths_and_one_whose_trash_is_there_loses_them() {
    let (state, _dir, file) = on_file();
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    recorded(&state, &trashed);
    let trash = Path::new(std::ffi::OsStr::from_bytes(&trash_folder_of(&trashed))).to_path_buf();
    let away = trash.with_file_name("trash-unmounted");

    // The disk holding the trash is not mounted: the trash looks as if it was never there.
    fs::rename(&trash, &away).unwrap();
    let unmounted = overview_now(&state);
    assert_eq!(unmounted.cleanups[0].status, Status::NoLongerInTrash);
    assert_eq!(
        platter_history::testing::entries_with_paths(&file),
        2,
        "nothing is forgotten for a trash that cannot be seen"
    );

    // It comes back, and one entry is gone from it.
    fs::rename(&away, &trash).unwrap();
    let TargetOutcome::Trashed(to) = &trashed.results[0].outcome else {
        unreachable!()
    };
    fs::remove_file(Path::new(std::ffi::OsStr::from_bytes(to))).unwrap();
    let back = overview_now(&state);

    assert_eq!(back.cleanups[0].status, Status::InTrash);
    assert_eq!(platter_history::testing::entries_with_paths(&file), 1);
}

#[cfg(unix)]
#[test]
fn a_run_whose_items_are_all_gone_stays_no_longer_in_the_trash_without_any_path() {
    let (state, _dir, file) = on_file();
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    let id = recorded(&state, &trashed);
    for result in &trashed.results {
        let TargetOutcome::Trashed(to) = &result.outcome else {
            unreachable!()
        };
        fs::remove_file(Path::new(std::ffi::OsStr::from_bytes(to))).unwrap();
    }

    let first = overview_now(&state);
    let second = overview_now(&state);

    assert_eq!(platter_history::testing::entries_with_paths(&file), 0);
    assert_eq!(first.cleanups[0].status, Status::NoLongerInTrash);
    assert_eq!(first, second);
    let refused = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();
    assert_eq!(
        refused.code(),
        ApiErrorCode::NotFound,
        "there is nothing to put back"
    );
}

#[cfg(unix)]
#[test]
fn an_overview_does_not_forget_the_paths_while_something_is_being_put_back() {
    let (state, _dir, file) = on_file();
    let trashed = trash_files(&state, &["a.iso"]);
    recorded(&state, &trashed);
    let TargetOutcome::Trashed(to) = &trashed.results[0].outcome else {
        unreachable!()
    };
    fs::remove_file(Path::new(std::ffi::OsStr::from_bytes(to))).unwrap();

    let slot = state.history.begin_restore().unwrap();
    let during = overview_now(&state);
    drop(slot);

    assert_eq!(during.cleanups[0].status, Status::NoLongerInTrash);
    assert_eq!(
        platter_history::testing::entries_with_paths(&file),
        1,
        "left for the next look"
    );
    overview_now(&state);
    assert_eq!(platter_history::testing::entries_with_paths(&file), 0);
}

#[cfg(unix)]
#[test]
fn a_run_that_emptied_the_trash_makes_the_history_forget_the_paths_of_what_was_in_it() {
    let (state, _dir, file) = on_file();
    let trashed = trash_files(&state, &["a.iso", "b.iso"]);
    recorded(&state, &trashed);
    let before = overview_now(&state);
    assert_eq!(platter_history::testing::entries_with_paths(&file), 2);

    let emptied = TargetResult {
        rule_id: "trash",
        path: "trash".to_owned(),
        exact: trash_folder_of(&trashed),
        identity: (1, 1),
        is_dir: true,
        allocated: 5000,
        outcome: TargetOutcome::Removed(Default::default()),
    };
    state
        .history
        .record_run(
            &trashed.home,
            &[("trash", CleanupAction::EmptyTrash)],
            &[emptied],
            Some(&report(Some(5000))),
            |_| {},
        )
        .expect("recorded");

    assert_eq!(platter_history::testing::entries_with_paths(&file), 0);
    let after = overview_now(&state);
    let old = after
        .cleanups
        .iter()
        .find(|row| row.id == before.cleanups[0].id)
        .unwrap();
    assert_eq!(old.status, Status::NoLongerInTrash);
    assert_eq!(
        (old.what.clone(), old.methods.clone(), old.freed),
        (
            before.cleanups[0].what.clone(),
            before.cleanups[0].methods.clone(),
            before.cleanups[0].freed
        )
    );
}

#[cfg(unix)]
#[test]
fn a_run_that_left_something_in_the_trash_does_not_make_the_history_forget_anything() {
    let trashed_run = |outcome| {
        let result = TargetResult {
            rule_id: "trash",
            path: "trash".to_owned(),
            exact: b"/home/u/.local/share/Trash".to_vec(),
            identity: (1, 1),
            is_dir: true,
            allocated: 5000,
            outcome,
        };
        describe_run(&[("trash", CleanupAction::EmptyTrash)], &[result], None, 1).unwrap()
    };

    let complete = trashed_run(TargetOutcome::Removed(Default::default()));
    let mut left = platter_core::Removed::default();
    left.note(b"/home/u/.local/share/Trash/files/x".to_vec(), "busy".to_owned());
    let partial = trashed_run(TargetOutcome::Removed(left));
    let mounted = trashed_run(TargetOutcome::Removed(platter_core::Removed {
        skipped_mounts: 1,
        ..Default::default()
    }));

    assert_eq!(complete.emptied_trashes, [b"/home/u/.local/share/Trash".to_vec()]);
    assert!(partial.emptied_trashes.is_empty());
    assert!(mounted.emptied_trashes.is_empty());
    assert!(partial.emptied_trash, "it is still a run that emptied the trash");
}

// ---- Which runs can be put back ----

/// Records `trashed` as a run that finished `back` seconds ago, on the disk holding `home`.
fn recorded_at(state: &AppState, home: &Path, trashed: &Trashed, back: i64) -> i64 {
    let groups = [("old-downloads", CleanupAction::Trash)];
    let run = describe_run(&groups, &trashed.results, None, unix_now() - back).unwrap();
    let volume = state.history.volume_of(home).unwrap();

    state
        .history
        .record(|store| store.record_cleanup(volume_ref(&volume), &run))
        .expect("recorded")
}

#[cfg(unix)]
#[test]
fn only_a_run_the_latest_overview_showed_can_be_put_back() {
    let state = approving();
    let trashed = trash_files(&state, &["a.iso"]);
    let id = recorded_at(&state, &trashed.home, &trashed, 60);

    let unseen = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();
    overview_now(&state);
    let seen = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap();

    assert_eq!(unseen.code(), ApiErrorCode::NotFound, "no overview has shown it");
    assert_eq!(seen.restored, 1);

    // A run recorded after the overview was read is not on its list.
    let other = trash_files(&state, &["b.iso"]);
    let hidden = recorded_at(&state, &other.home, &other, 60);
    let refused = tauri::async_runtime::block_on(restore_run(&state, hidden)).unwrap_err();
    assert_eq!(refused.code(), ApiErrorCode::NotFound);
    assert!(!other.home.join("b.iso").exists());
}

#[cfg(unix)]
#[test]
fn a_run_older_than_the_window_is_not_shown_and_cannot_be_put_back() {
    let state = AppState::for_test();
    let trashed = trash_files(&state, &["a.iso"]);
    let old = recorded_at(&state, &trashed.home, &trashed, (MAX_DAYS + 1) * 86_400 + 43_200);
    let overview = overview_now(&state);

    let refused = tauri::async_runtime::block_on(restore_run(&state, old)).unwrap_err();

    assert!(overview.cleanups.is_empty());
    assert_eq!(refused.code(), ApiErrorCode::NotFound);
    assert!(!trashed.home.join("a.iso").exists(), "nothing was put back");
}

#[cfg(unix)]
#[test]
fn a_run_on_another_volume_than_the_one_shown_cannot_be_put_back() {
    let mut state = AppState::for_test();
    let history = Arc::new(HistoryService::in_memory(two_disks()));
    state.history = Arc::clone(&history);
    let trashed = trash_files(&state, &["a.iso"]);
    history.record_scan(Path::new("/mnt/data/x"), &scanned_totals());
    let id = recorded_at(&state, Path::new("/mnt/data/x"), &trashed, 60);
    assert_eq!(
        overview_now(&state).cleanups.len(),
        1,
        "shown while Data is the volume"
    );

    // A scan of the other disk moves the history to it before the page looks again.
    history.record_scan(Path::new("/home/u"), &scanned_totals());
    let refused = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();

    assert_eq!(refused.code(), ApiErrorCode::NotFound);
    assert!(!trashed.home.join("a.iso").exists());
}

#[cfg(unix)]
#[test]
fn a_run_with_nothing_left_to_put_back_is_refused() {
    let state = approving();
    let trashed = trash_files(&state, &["a.iso"]);
    let id = recorded(&state, &trashed);
    tauri::async_runtime::block_on(restore_run(&state, id)).unwrap();

    let again = tauri::async_runtime::block_on(restore_run(&state, id)).unwrap_err();

    assert_eq!(again.code(), ApiErrorCode::NotFound);
}

// ---- Recording never gets in the way ----

/// A system that cannot tell its disks: listing them panics.
struct Broken;

impl VolumeProvider for Broken {
    fn list(&self) -> Vec<Volume> {
        panic!("the system would not say");
    }
}

#[cfg(unix)]
#[test]
fn a_history_that_panics_stops_neither_a_scan_nor_a_cleanup() {
    let mut state = AppState::for_test();
    state.history = Arc::new(HistoryService::in_memory(Broken));
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project();

    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();
    let (started, receiver) = start_of(&state, plan.plan_id);
    started.unwrap();
    let events = until_finished(&receiver);

    assert!(matches!(events.last(), Some(CleanupEventDto::Finished { .. })));
    assert!(!root.join("p/target").exists(), "the cleanup ran");
    assert!(!state.cleanups.is_running(), "the slot is free");
    state.history.sample_all();
    state.history.record_scan(&root, &Totals::default());
}

#[cfg(unix)]
#[test]
fn a_run_is_kept_for_undo_and_the_history_before_the_ui_hears_it_ended() {
    let state = AppState::for_test();
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();
    let (started, receiver) = start_of(&state, plan.plan_id);
    started.unwrap();

    until_finished(&receiver);

    assert!(!state.cleanups.is_running(), "the slot is free at Finished");
    assert_eq!(overview_now(&state).cleanup_count, 1, "and the run is on record");
}

// ---- Overviews, one at a time ----

#[cfg(unix)]
#[test]
fn one_overview_reads_and_one_waits_and_a_third_is_refused() {
    let state = AppState::for_test();

    let first = Turn::join(&state.history).expect("a place");
    let second = Turn::join(&state.history).expect("a place to wait");
    let third = Turn::join(&state.history);

    assert!(third.is_none(), "the line is full");
    drop(first);
    assert!(Turn::join(&state.history).is_some(), "a place is free again");
    drop(second);
    assert_eq!(state.history.overviews.load(Ordering::SeqCst), 0);
}

/// Counts how often the trash is looked at.
struct Counting(AtomicUsize);

impl EntryProbe for Counting {
    fn probe(&self, _path: &[u8]) -> std::io::Result<Vec<platter_core::ProbedEntry>> {
        self.0.fetch_add(1, Ordering::SeqCst);

        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    }
}

#[cfg(unix)]
#[test]
fn an_overview_of_a_huge_run_looks_at_a_bounded_number_of_entries_and_says_it_did_not_check() {
    let history = HistoryService::in_memory(FixedVolumes::root());
    let many = CHECK_CAP + 500;
    let items: Vec<NewItem> = (0..many)
        .map(|number| NewItem {
            original: format!("/home/u/{number}").into_bytes(),
            trashed: format!("/t/files/{number}").into_bytes(),
            identity: (1, number as u64),
            is_dir: false,
            allocated: 10,
        })
        .collect();
    let run = NewCleanup {
        at: unix_now(),
        trashed: 10 * many as u64,
        items,
        ..NewCleanup::default()
    };
    history
        .record(|store| store.record_cleanup(volume_ref(&volume("/", "Root", 1000, 400)), &run))
        .unwrap();
    let probe = Counting(AtomicUsize::new(0));

    let overview = history.overview(30, &probe);

    assert_eq!(overview.cleanups[0].status, Status::NotChecked);
    // Each entry that is looked at, and the one trash folder they share.
    assert_eq!(probe.0.load(Ordering::SeqCst), CHECK_CAP + 1);
}

// ---- A file that was locked ----

#[cfg(unix)]
#[test]
fn a_file_another_program_locked_is_used_once_it_is_free_and_a_damaged_one_is_not_tried_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    drop(platter_history::Store::open(&path).unwrap());
    let lock = platter_history::testing::lock_exclusively(&path);

    let history = HistoryService::in_dir(Some(dir.path()), FixedVolumes::root());
    assert!(
        history
            .problem()
            .is_some_and(|problem| problem.contains("in use"))
    );
    history.record_scan(Path::new("/home/u"), &scanned_totals());
    assert!(history.problem().is_some(), "still locked, nothing recorded");

    drop(lock);
    history.record_scan(Path::new("/home/u"), &scanned_totals());

    assert_eq!(history.problem(), None, "the file is open now");
    let overview = history.overview(30, &Counting(AtomicUsize::new(0)));
    assert_eq!(
        overview.volume.map(|volume| volume.label).as_deref(),
        Some("Root")
    );

    let damaged = tempfile::tempdir().unwrap();
    fs::write(
        damaged.path().join("history.sqlite"),
        b"not a database ".repeat(400),
    )
    .unwrap();
    let broken = HistoryService::in_dir(Some(damaged.path()), FixedVolumes::root());
    fs::remove_file(damaged.path().join("history.sqlite")).unwrap();
    broken.record_scan(Path::new("/home/u"), &scanned_totals());
    assert!(
        broken
            .problem()
            .is_some_and(|problem| problem.contains("damaged")),
        "left alone, not opened again"
    );
}

#[cfg(unix)]
#[test]
fn a_history_file_swapped_for_another_between_the_check_and_sqlite_opening_it_is_not_used() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");

    // Another file takes the name after it was checked and before SQLite opens it by name.
    let opened = open_in_with(Some(dir.path()), |name| {
        fs::rename(name, name.with_file_name("moved.sqlite")).unwrap();
        fs::write(name, b"").unwrap();
    });

    let closed = opened.err().expect("the swapped file is not used");
    assert!(closed.problem.contains("replaced"), "{}", closed.problem);
    assert!(closed.reopen.is_none(), "and it is not tried again");

    // Nothing swapped: the same open works.
    fs::remove_file(&path).unwrap();
    assert!(open_in_with(Some(dir.path()), |_| {}).is_ok());
}

#[cfg(unix)]
#[test]
fn a_history_file_that_was_open_to_others_is_closed() {
    let dir = tempfile::tempdir().unwrap();
    let place = dir.path().join("platter");
    drop(HistoryService::in_dir(Some(&place), FixedVolumes::root()));
    let file = place.join("history.sqlite");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o666)).unwrap();
    fs::set_permissions(&place, fs::Permissions::from_mode(0o755)).unwrap();

    let history = HistoryService::in_dir(Some(&place), FixedVolumes::root());

    assert_eq!(history.problem(), None);
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!((mode(&file), mode(&place)), (0o600, 0o700));
}

/// What runs on every system: what is read out of a path or an action, without a disk.
#[test]
fn a_name_is_the_last_part_of_a_path_of_either_syntax() {
    assert_eq!(name_of("/home/u/Projekty/app"), "app");
    assert_eq!(name_of("/home/u/Projekty/app/"), "app");
    assert_eq!(name_of("C:\\Users\\u\\Projekty\\app"), "app");
    assert_eq!(name_of("C:\\Users\\u\\Projekty\\app\\"), "app");
    // A backslash in a Posix name is part of the name.
    assert_eq!(name_of("/home/u/a\\b"), "a\\b");
}

#[test]
fn emptying_a_folder_into_the_bin_is_recorded_as_a_move_to_the_trash() {
    assert_eq!(method_of(CleanupAction::TrashChildren).kind, MethodKind::Trash);
    assert_eq!(method_of(CleanupAction::Trash).kind, MethodKind::Trash);
    assert_eq!(method_of(CleanupAction::EmptyFolder).kind, MethodKind::Deleted);
}
