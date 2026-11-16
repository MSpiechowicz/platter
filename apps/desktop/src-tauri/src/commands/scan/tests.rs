use std::fs;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use platter_core::dto::{ApiErrorCode, ScanEvent, ScanId, ScanOptions, SortKey};
use platter_core::query;
use platter_core::testing::FakeFs;
use platter_core::{NodeId, ScanConfig};

use super::*;

fn options(root: &Path) -> ScanOptions {
    ScanOptions {
        root: root.to_string_lossy().into_owned(),
        one_file_system: false,
    }
}

/// Starts a scan and gathers its events up to and including the terminal one.
fn run_to_the_end(state: &AppState, options: ScanOptions) -> (ScanId, Vec<ScanEvent>) {
    let (sender, receiver) = mpsc::channel();
    let id = tauri::async_runtime::block_on(start(
        state,
        options,
        Arc::new(move |event| {
            let _ = sender.send(event);
        }),
    ))
    .expect("the scan starts");

    let mut events = Vec::new();
    loop {
        let event = receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("a terminal event arrives");
        let terminal = !matches!(event, ScanEvent::Progress { .. });
        events.push(event);

        if terminal {
            return (id, events);
        }
    }
}

fn sample_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("photos/2024")).unwrap();
    fs::write(dir.path().join("photos/2024/a.jpg"), vec![1u8; 20_000]).unwrap();
    fs::write(dir.path().join("photos/b.jpg"), vec![2u8; 10_000]).unwrap();
    fs::write(dir.path().join("notes.txt"), b"hello").unwrap();

    dir
}

#[test]
fn a_scan_reports_progress_then_exactly_one_finished_event() {
    let state = AppState::for_test();
    let dir = sample_dir();

    let (id, events) = run_to_the_end(&state, options(dir.path()));

    let (terminal, progress) = events.split_last().unwrap();
    assert!(
        progress
            .iter()
            .all(|event| matches!(event, ScanEvent::Progress { .. }))
    );
    let ScanEvent::Finished { scan_id, totals, .. } = terminal else {
        panic!("expected finished, got {terminal:?}");
    };
    assert_eq!(*scan_id, id);
    assert_eq!(totals.files, 3);
    assert_eq!(totals.apparent, 20_000 + 10_000 + 5);
}

#[test]
fn queries_work_on_the_scan_as_soon_as_finished_is_seen() {
    let state = AppState::for_test();
    let dir = sample_dir();

    let (id, events) = run_to_the_end(&state, options(dir.path()));

    let Some(ScanEvent::Finished { root_id, .. }) = events.last() else {
        panic!("expected finished");
    };
    let tree = state.scans.tree(id).expect("the finished tree is registered");

    let page = query::children(&tree, *root_id, 0, 10, SortKey::SizeDesc).unwrap();
    let names: Vec<&str> = page.items.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(page.total, 2);
    assert_eq!(names, ["photos", "notes.txt"]);

    let photos = page.items[0].id;
    let path = query::path(&tree, photos).unwrap();
    assert!(path.path.ends_with("photos"));
    assert_eq!(path.segments.len(), 2);
    // `photos` has a single subfolder below the dominance share, so it is the one listed.
    let summary = query::summary(&tree, *root_id).unwrap();
    assert!(!summary.largest_dirs.is_empty());
    assert!(summary.largest_dirs.iter().all(|dir| !dir.unreadable));
    assert!(query::lod(&tree, *root_id, 100, 4).unwrap().children.len() == 2);
    assert_eq!(
        query::node(&tree, NodeId(u32::MAX)).unwrap_err(),
        platter_core::QueryError::NotFound(NodeId(u32::MAX))
    );
}

#[test]
fn a_second_finished_scan_replaces_the_first() {
    let state = AppState::for_test();
    let dir = sample_dir();

    let (first, _) = run_to_the_end(&state, options(dir.path()));
    let (second, _) = run_to_the_end(&state, options(dir.path()));

    assert!(second > first);
    assert_eq!(
        state.scans.tree(first).unwrap_err().code(),
        ApiErrorCode::NotFound
    );
    assert!(state.scans.tree(second).is_ok());
}

#[test]
fn a_cancelled_scan_ends_with_cancelled_and_leaves_no_tree() {
    let registry = ScanRegistry::default();
    let (id, cancel) = registry.begin().unwrap();
    let mut fs = FakeFs::new();
    fs.file("/a/one", 10).file("/b/two", 10).file("/c/three", 10);
    let trigger = cancel.clone();
    fs.on_read_dir(move |_| trigger.cancel());
    let events = Mutex::new(Vec::new());

    run_scan(
        &fs,
        &registry,
        id,
        &cancel,
        Path::new("/"),
        &ScanConfig::default(),
        &|event| {
            events.lock().unwrap().push(event);
        },
    );

    let events = events.into_inner().unwrap();
    assert_eq!(events.last(), Some(&ScanEvent::Cancelled));
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, ScanEvent::Finished { .. }))
    );
    assert_eq!(registry.tree(id).unwrap_err().code(), ApiErrorCode::NotFound);
}

#[test]
fn a_scan_that_cannot_open_its_root_ends_with_failed() {
    let registry = ScanRegistry::default();
    let (id, cancel) = registry.begin().unwrap();
    let fs = FakeFs::new();
    let events = Mutex::new(Vec::new());

    run_scan(
        &fs,
        &registry,
        id,
        &cancel,
        Path::new("/missing"),
        &ScanConfig::default(),
        &|event| {
            events.lock().unwrap().push(event);
        },
    );

    let events = events.into_inner().unwrap();
    assert!(matches!(events.as_slice(), [ScanEvent::Failed { message }] if message.contains("/missing")));
    assert_eq!(registry.tree(id).unwrap_err().code(), ApiErrorCode::NotFound);
}

#[test]
fn the_root_is_validated_before_a_scan_is_registered() {
    let state = AppState::for_test();
    let dir = sample_dir();
    let file = dir.path().join("notes.txt");
    let emit: EmitFn = Arc::new(|_| {});

    for root in [
        "",
        "   ",
        "relative/dir",
        "/definitely/not/here",
        file.to_str().unwrap(),
    ] {
        let rejected = tauri::async_runtime::block_on(start(
            &state,
            ScanOptions {
                root: root.to_owned(),
                one_file_system: false,
            },
            Arc::clone(&emit),
        ))
        .unwrap_err();

        assert_eq!(rejected.code(), ApiErrorCode::Invalid, "root {root:?}");
    }

    let (first_valid, _) = run_to_the_end(&state, options(dir.path()));
    assert_eq!(first_valid, 1, "rejected requests do not consume ids");
}

#[test]
fn the_app_caps_a_scan_at_what_the_free_memory_allows() {
    let config = scan_config(true, Some((1, 2)));

    let budget = config.max_memory_bytes.expect("the app bounds a scan by memory");
    assert!(budget > 0, "this machine has free memory");
    assert_eq!(
        config.max_entries, None,
        "the entry cap is only the hard limit of the ids"
    );
    assert!(config.one_file_system);
    assert_eq!(config.root_identity, Some((1, 2)));
}

#[test]
fn a_scan_over_the_entry_cap_ends_with_failed_and_a_message_in_plain_words() {
    let registry = ScanRegistry::default();
    let (id, cancel) = registry.begin().unwrap();
    let mut fs = FakeFs::new();
    for i in 0..20 {
        fs.file(&format!("/d/f{i}"), 1);
    }
    let events = Mutex::new(Vec::new());
    let config = ScanConfig {
        max_entries: Some(5),
        ..ScanConfig::default()
    };

    run_scan(&fs, &registry, id, &cancel, Path::new("/"), &config, &|event| {
        events.lock().unwrap().push(event);
    });

    let events = events.into_inner().unwrap();
    assert_eq!(
        events.last(),
        Some(&ScanEvent::Failed {
            message: "This folder has more entries than Platter can hold in memory (limit 5 entries). Scan a smaller folder."
                .to_owned()
        })
    );
    let terminal = events
        .iter()
        .filter(|event| !matches!(event, ScanEvent::Progress { .. }))
        .count();
    assert_eq!(terminal, 1, "the failure is reported exactly once");
    assert_eq!(registry.tree(id).unwrap_err().code(), ApiErrorCode::NotFound);
}

#[test]
fn the_finished_event_carries_the_warnings_of_the_scan() {
    let registry = ScanRegistry::default();
    let mut fs = FakeFs::new();
    fs.file("/a", 1)
        .warn(platter_core::ScanWarning::MountTableUnavailable);
    let finished = |fs: &FakeFs| {
        let (id, cancel) = registry.begin().unwrap();
        let events = Mutex::new(Vec::new());
        run_scan(
            fs,
            &registry,
            id,
            &cancel,
            Path::new("/"),
            &ScanConfig::default(),
            &|event| {
                events.lock().unwrap().push(event);
            },
        );
        events.into_inner().unwrap().pop().unwrap()
    };

    let ScanEvent::Finished { warnings, totals, .. } = finished(&fs) else {
        panic!("the scan finishes");
    };
    assert_eq!(warnings, [platter_core::ScanWarning::MountTableUnavailable]);
    assert_eq!(totals.errors, 1, "also counted as an error");

    let ScanEvent::Finished { warnings, .. } = finished(&FakeFs::new()) else {
        panic!("the scan finishes");
    };
    assert!(warnings.is_empty());
}

#[test]
fn a_picked_folder_that_is_no_longer_the_same_folder_is_refused() {
    let state = AppState::for_test();
    let dir = sample_dir();
    let emit: EmitFn = Arc::new(|_| {});

    let swapped = tauri::async_runtime::block_on(start_pinned(
        &state,
        options(dir.path()),
        Some((0, 0)),
        Arc::clone(&emit),
    ))
    .unwrap_err();
    assert_eq!(swapped.code(), ApiErrorCode::Invalid);

    let real = platter_platform::root::resolve_root(dir.path()).unwrap().identity;
    assert!(tauri::async_runtime::block_on(start_pinned(&state, options(dir.path()), real, emit)).is_ok());
}
