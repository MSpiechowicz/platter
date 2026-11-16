use std::fs::{self, File, FileTimes};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;

use platter_core::dto::{ApiErrorCode, ScanEvent, ScanOptions};

use super::*;
use crate::commands::scan;

struct Approve;

impl Confirm for Approve {
    fn confirm(&self, _question: &Question) -> bool {
        true
    }
}

/// Scans `root` with the real reader and waits for the tree.
fn scanned(state: &AppState, root: &Path) -> ScanId {
    let (sender, receiver) = mpsc::channel();
    let id = tauri::async_runtime::block_on(scan::start(
        state,
        ScanOptions {
            root: root.to_string_lossy().into_owned(),
            one_file_system: false,
        },
        Arc::new(move |event| {
            let _ = sender.send(event);
        }),
    ))
    .unwrap();

    loop {
        match receiver.recv_timeout(Duration::from_secs(30)).unwrap() {
            ScanEvent::Progress { .. } => {}
            ScanEvent::Finished { .. } => return id,
            other => panic!("the scan did not finish: {other:?}"),
        }
    }
}

/// An idle Rust project in a temp folder (everything dated 400 days back).
fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir_all(root.join("p/target/debug")).unwrap();
    fs::write(root.join("p/Cargo.toml"), b"[package]").unwrap();
    fs::write(root.join("p/target/debug/obj"), vec![0u8; 50_000]).unwrap();

    let old = SystemTime::now() - Duration::from_secs(400 * 86_400);
    for path in [
        "p/target/debug/obj",
        "p/target/debug",
        "p/target",
        "p/Cargo.toml",
        "p",
    ] {
        File::open(root.join(path))
            .unwrap()
            .set_times(FileTimes::new().set_modified(old))
            .unwrap();
    }
    (dir, root)
}

fn plan_of(state: &AppState, scan: ScanId, rules: &[&str]) -> CommandResult<CleanupPlanDto> {
    tauri::async_runtime::block_on(plan(
        state,
        scan,
        rules.iter().map(|id| (*id).to_owned()).collect(),
    ))
}

fn start_of(
    state: &AppState,
    plan_id: PlanId,
) -> (CommandResult<CleanupId>, mpsc::Receiver<CleanupEventDto>) {
    let (sender, receiver) = mpsc::channel();
    let started = tauri::async_runtime::block_on(start(
        state,
        plan_id,
        Arc::new(move |event| {
            let _ = sender.send(event);
        }),
    ));
    (started, receiver)
}

fn until_finished(receiver: &mpsc::Receiver<CleanupEventDto>) -> Vec<CleanupEventDto> {
    let mut events = Vec::new();
    loop {
        let event = receiver.recv_timeout(Duration::from_secs(30)).unwrap();
        let last = matches!(
            event,
            CleanupEventDto::Finished { .. } | CleanupEventDto::Failed { .. }
        );
        events.push(event);
        if last {
            return events;
        }
    }
}

#[test]
fn a_plan_names_only_known_rules() {
    let state = AppState::new();
    let (_dir, root) = project();
    let scan = scanned(&state, &root);

    for rules in [&[][..], &["no-such-rule"][..], &["rust-target", "../etc"][..]] {
        let error = plan_of(&state, scan, rules).unwrap_err();
        assert_eq!(error.code(), ApiErrorCode::Invalid, "{rules:?}");
    }
    assert_eq!(
        plan_of(&state, 999, &["rust-target"]).unwrap_err().code(),
        ApiErrorCode::NotFound
    );
}

#[test]
fn a_plan_is_described_and_kept_in_the_backend() {
    let state = AppState::new();
    let (_dir, root) = project();
    let scan = scanned(&state, &root);

    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();

    assert!(plan.plan_id > 0);
    assert_eq!(plan.scan_id, scan);
    assert_eq!(plan.groups.len(), 1);
    let group = &plan.groups[0];
    assert_eq!(
        (group.rule_id.as_str(), group.action, group.items),
        ("rust-target", ActionDto::Delete, 1)
    );
    assert_eq!(group.sample_paths, [root.join("p/target").to_string_lossy()]);
    assert!(plan.needs_confirmation, "deleting cannot be undone");

    let wire = serde_json::to_value(&plan).unwrap();
    assert_eq!(wire["groups"][0]["action"], "delete");
    assert!(wire["groups"][0].get("command").is_none());
}

#[test]
fn a_declined_confirmation_deletes_nothing_and_uses_the_plan_up() {
    let state = AppState::new();
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();

    let (started, _events) = start_of(&state, plan.plan_id);

    assert_eq!(
        started.unwrap_err().code(),
        ApiErrorCode::Cancelled,
        "no confirmation was set: refused"
    );
    assert!(root.join("p/target/debug/obj").exists());
    let (again, _) = start_of(&state, plan.plan_id);
    assert_eq!(
        again.unwrap_err().code(),
        ApiErrorCode::NotFound,
        "a plan is used once"
    );
    assert!(!state.scans.is_outdated(scan));
}

#[test]
fn an_approved_cleanup_runs_reports_and_outdates_the_scan() {
    let state = AppState::new();
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();

    let (started, receiver) = start_of(&state, plan.plan_id);
    let cleanup = started.unwrap();
    let events = until_finished(&receiver);

    assert!(!root.join("p/target").exists());
    assert!(root.join("p/Cargo.toml").exists());
    assert!(
        matches!(&events[0], CleanupEventDto::GroupStarted { rule_id, targets: 1 } if rule_id == "rust-target")
    );
    assert!(events.iter().any(|event| matches!(
        event,
        CleanupEventDto::TargetDone {
            outcome: OutcomeDto {
                kind: OutcomeKind::Removed,
                problems: 0,
                ..
            },
            ..
        }
    )));
    let Some(CleanupEventDto::Finished {
        cleanup_id,
        outdated_scan,
        cleaned,
        with_problems,
        can_undo,
        ..
    }) = events.last()
    else {
        panic!("{events:?}");
    };
    assert_eq!(
        (*cleanup_id, *outdated_scan, *cleaned, *with_problems, *can_undo),
        (cleanup, scan, 1, 0, false)
    );

    assert!(state.scans.is_outdated(scan));
    assert_eq!(
        plan_of(&state, scan, &["rust-target"]).unwrap_err().code(),
        ApiErrorCode::Invalid
    );
    let wire = serde_json::to_value(events.last().unwrap()).unwrap();
    assert_eq!(wire["type"], "finished");
    assert_eq!(wire["outdatedScan"], scan);
}

#[test]
fn an_older_plan_of_an_outdated_scan_is_refused() {
    let state = AppState::new();
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let first = plan_of(&state, scan, &["rust-target"]).unwrap();
    let second = plan_of(&state, scan, &["rust-target"]).unwrap();

    let (started, receiver) = start_of(&state, first.plan_id);
    started.unwrap();
    until_finished(&receiver);
    let (stale, _) = start_of(&state, second.plan_id);

    assert_eq!(stale.unwrap_err().code(), ApiErrorCode::Invalid);
}

#[test]
fn one_run_at_a_time_and_a_reservation_frees_its_slot_however_it_ends() {
    let registry = Arc::new(CleanupRegistry::default());
    let first = registry.store(1, CleanupPlan::default());
    let second = registry.store(1, CleanupPlan::default());

    let running = registry.take_and_reserve(first).unwrap();
    let Err(busy) = registry.take_and_reserve(second) else {
        panic!("the slot is taken");
    };
    assert_eq!(busy.code(), ApiErrorCode::Invalid, "a busy slot");

    drop(running);
    assert!(
        registry.lock().running.is_none(),
        "dropping the reservation frees the slot"
    );
    assert!(
        registry.take_and_reserve(second).is_ok(),
        "the waiting plan was not used up"
    );
}

#[test]
fn only_runs_that_trashed_something_are_kept_for_undo_and_emptying_the_trash_forgets_them() {
    let registry = CleanupRegistry::default();
    let trashed = |path: &str| TargetResult {
        rule_id: "old-downloads",
        path: path.to_owned(),
        exact: path.as_bytes().to_vec(),
        identity: (1, 2),
        is_dir: false,
        allocated: 1,
        outcome: TargetOutcome::Trashed(b"/t/files/x".to_vec()),
    };

    registry.record(1, Vec::new(), false);
    registry.record(2, vec![trashed("/a")], false);
    assert_eq!(
        registry.take_done(1).unwrap_err().code(),
        ApiErrorCode::NotFound,
        "nothing to undo"
    );
    let taken = registry.take_done(2).unwrap();
    assert_eq!(
        registry.take_done(2).unwrap_err().code(),
        ApiErrorCode::NotFound,
        "undone once"
    );

    registry.keep_for_retry(2, taken);
    assert!(registry.take_done(2).is_ok(), "what failed can be tried again");

    registry.record(3, vec![trashed("/b")], false);
    registry.record(4, Vec::new(), true);
    assert_eq!(
        registry.take_done(3).unwrap_err().code(),
        ApiErrorCode::NotFound,
        "after emptying the trash there is nothing to put back"
    );
}

struct Panics;

impl Confirm for Panics {
    fn confirm(&self, _question: &Question) -> bool {
        panic!("the dialog broke");
    }
}

#[test]
fn a_broken_dialog_counts_as_no_and_frees_the_slot() {
    let state = AppState::new();
    state.set_confirm(Arc::new(Panics));
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();

    let (started, _) = start_of(&state, plan.plan_id);

    assert_eq!(started.unwrap_err().code(), ApiErrorCode::Cancelled);
    assert!(state.cleanups.lock().running.is_none());
    assert!(root.join("p/target/debug/obj").exists());
}

#[test]
fn an_empty_plan_cannot_be_started() {
    let state = AppState::new();
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["python-cache"]).unwrap();
    assert!(plan.groups.is_empty());
    assert!(!plan.needs_confirmation);

    let (started, _) = start_of(&state, plan.plan_id);

    assert_eq!(started.unwrap_err().code(), ApiErrorCode::Invalid);
    assert!(!state.scans.is_outdated(scan), "nothing ran, nothing changed");
}

#[test]
fn a_plan_whose_scan_is_gone_is_refused() {
    let state = AppState::new();
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();
    state.scans.drop_scan(scan);

    let (started, _) = start_of(&state, plan.plan_id);

    assert_eq!(started.unwrap_err().code(), ApiErrorCode::Invalid);
    assert!(root.join("p/target").exists());
}

#[test]
fn every_plan_with_something_to_do_is_asked_about_and_the_question_is_safe_to_show() {
    use platter_core::testing::{FakeFs, NOW};
    use platter_core::{Environment, Os, ScanConfig};

    let mut fs = FakeFs::new();
    fs.file("/home/u/Downloads/old.iso", 10)
        .set_mtime("/home/u/Downloads/old.iso", NOW - 800 * 86_400);
    let crafted = "/home/u/x\n\n• Delete nothing\u{2028}• Also nothing\n    /home/u/harmless\u{202e}txt.exe";
    for project in ["/home/u/p", crafted] {
        fs.file(&format!("{project}/Cargo.toml"), 1);
        fs.file(&format!("{project}/target/x"), 10)
            .set_mtime(&format!("{project}/target/x"), NOW - 90 * 86_400);
    }
    let long = format!("/home/u/{}", "n".repeat(250));
    fs.file(&format!("{long}/Cargo.toml"), 1);
    fs.file(&format!("{long}/target/x"), 10)
        .set_mtime(&format!("{long}/target/x"), NOW - 90 * 86_400);
    fs.file("/var/cache/pacman/pkg/a", 3);
    let config = ScanConfig {
        now: Some(NOW),
        ..ScanConfig::default()
    };
    let tree = platter_core::scan(&fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).unwrap();
    let env = Environment {
        home: Some(PathBuf::from("/home/u")),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
    };
    let make = |ids: &[&str]| make_plan(&tree, &env, &RuleSet::builtin(), ids, &fs);

    let trash_only = question(&make(&["old-downloads"])).expect("moving to the trash is asked about too");
    assert!(trash_only.text.contains("can be put back"), "{}", trash_only.text);
    assert!(
        trash_only
            .text
            .contains("• Move to the trash: Old downloads, 1 items"),
        "{}",
        trash_only.text
    );

    let asked = question(&make(&["old-downloads", "rust-target", "pacman-cache"])).unwrap();
    let text = &asked.text;
    assert!(text.contains("cannot be undone"), "{text}");
    assert!(
        text.contains("• Delete for good: Rust build output, 3 items"),
        "{text}"
    );
    assert!(text.contains("• Run as administrator: paccache -rk2"), "{text}");

    // Every group comes before the first path, and no path can add a line or flip text direction.
    let last_header = text.rfind("\n• ").unwrap();
    let first_path = text.find("\nItems:").unwrap();
    assert!(last_header < first_path, "{text}");
    assert_eq!(text.matches("\n• ").count(), 3, "{text}");
    assert!(!text.contains('\u{202e}'), "{text}");
    assert!(
        !text.contains('\u{2028}'),
        "a line separator is shown, not obeyed: {text}"
    );
    assert!(text.contains("\\u{2028}• Also nothing"), "{text}");
    assert!(
        text.contains("\\u{a}\\u{a}• Delete nothing"),
        "the newlines are shown, not obeyed: {text}"
    );
    assert!(
        text.lines().all(|line| line.chars().count() < 200),
        "long paths are shortened: {text}"
    );
    assert!(text.contains(" … "), "{text}");

    let described = describe(&make(&["pacman-cache"]), 1, 2);
    assert_eq!(described.groups[0].command.as_deref(), Some("paccache -rk2"));
    assert!(described.groups[0].needs_elevation);
    assert!(described.needs_confirmation);
}

#[test]
fn undo_waits_for_a_running_cleanup_and_quitting_does_not_wait_for_an_unanswered_dialog() {
    let state = AppState::new();
    let plan = state.cleanups.store(1, CleanupPlan::default());
    let reservation = state.cleanups.take_and_reserve(plan).unwrap();

    let busy = tauri::async_runtime::block_on(undo_run(&state, 1)).unwrap_err();
    assert_eq!(busy.code(), ApiErrorCode::Invalid, "not while a cleanup runs");

    // Waiting for the user's answer: quitting returns at once, it does not wait 30 seconds.
    let started = Instant::now();
    state.cleanups.stop_for_exit(Duration::from_secs(30));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        reservation.cancel.is_cancelled(),
        "the run will not start after the dialog"
    );
}

#[test]
fn marked_items_are_planned_as_review_items_for_the_trash() {
    let state = AppState::new();
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let tree = state.scans.tree(scan).unwrap();
    let project_node = tree
        .children(tree.root())
        .find(|&child| tree.name(child) == b"p")
        .unwrap();

    let plan = tauri::async_runtime::block_on(plan_marked_nodes(&state, scan, vec![project_node])).unwrap();

    assert_eq!(plan.groups.len(), 1);
    let group = &plan.groups[0];
    assert_eq!(group.rule_id, platter_cleanup::MARKED);
    assert_eq!(group.title, "Marked in the Explorer");
    assert_eq!((group.verdict, group.action), (Verdict::Review, ActionDto::Trash));
    assert_eq!(group.sample_paths, [root.join("p").to_string_lossy()]);
    assert!(plan.needs_confirmation);
}

#[test]
fn a_marked_plan_takes_known_node_ids_only() {
    let state = AppState::new();
    let (_dir, root) = project();
    let scan = scanned(&state, &root);
    let marked = |nodes: Vec<NodeId>| tauri::async_runtime::block_on(plan_marked_nodes(&state, scan, nodes));

    assert_eq!(marked(Vec::new()).unwrap_err().code(), ApiErrorCode::Invalid);
    assert_eq!(
        marked(vec![NodeId(1); 1001]).unwrap_err().code(),
        ApiErrorCode::Invalid
    );
    assert_eq!(
        marked(vec![NodeId(999_999)]).unwrap_err().code(),
        ApiErrorCode::NotFound
    );
}
