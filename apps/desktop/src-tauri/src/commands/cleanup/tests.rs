use std::fs::{self, File, FileTimes};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;

use platter_cleanup::plan_labelled;
use platter_core::dto::{ApiErrorCode, ScanEvent, ScanOptions};

use super::*;
use crate::commands::advisor::{AdvisorOptionsDto, UnloadAfterDto};
use crate::commands::scan;
use crate::state::LabelSet;

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
    let state = AppState::for_test();
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
    let state = AppState::for_test();
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
    let state = AppState::for_test();
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
    let state = AppState::for_test();
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
    let state = AppState::for_test();
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
    let state = AppState::for_test();
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
    let state = AppState::for_test();
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
    let state = AppState::for_test();
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
    let make = |ids: &[&str]| platter_cleanup::plan_cleanup(&tree, &env, &RuleSet::builtin(), ids, &fs);

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
    let state = AppState::for_test();
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
    let state = AppState::for_test();
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
    let state = AppState::for_test();
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

// ---- The advisor group ----

/// The project of [`project`] and a folder of the user's own next to it.
fn project_and_stuff() -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = project();
    fs::create_dir_all(root.join("q/stuff")).unwrap();
    fs::create_dir_all(root.join("q/photos")).unwrap();
    fs::write(root.join("q/stuff/data"), vec![1u8; 20_000]).unwrap();
    fs::write(root.join("q/photos/img"), vec![2u8; 20_000]).unwrap();

    (dir, root)
}

fn switch(state: &AppState, enabled: bool) {
    state
        .advisor
        .configure(AdvisorOptionsDto {
            enabled,
            unload_after: UnloadAfterDto::FiveMinutes,
            log_questions: true,
        })
        .unwrap();
}

fn node_in(state: &AppState, scan: ScanId, path: &str) -> NodeId {
    let tree = state.scans.tree(scan).unwrap();
    let mut current = tree.root();

    for part in path.split('/') {
        current = tree
            .children(current)
            .find(|&child| tree.name(child) == part.as_bytes())
            .unwrap_or_else(|| panic!("{path} is in the scan"));
    }

    current
}

/// Stores the labels a run would have made.
fn label(state: &AppState, scan: ScanId, labels: &[(&str, ModelVerdict)]) {
    let labels = labels
        .iter()
        .map(|(path, verdict)| ModelLabel {
            node: node_in(state, scan, path),
            verdict: *verdict,
            reason: "because".to_owned(),
        })
        .collect();

    assert!(state.scans.set_labels(
        scan,
        LabelSet {
            labels,
            model_ms: 1,
            stopped: None,
        }
    ));
}

#[test]
fn the_advisor_group_is_planned_from_the_review_labels_as_review_items_for_the_trash() {
    let state = AppState::for_test();
    switch(&state, true);
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    label(
        &state,
        scan,
        &[
            ("q/stuff", ModelVerdict::Review),
            ("q/photos", ModelVerdict::Keep),
        ],
    );

    let plan = plan_of(&state, scan, &["advisor"]).unwrap();

    assert_eq!(plan.groups.len(), 1);
    let group = &plan.groups[0];
    assert_eq!(group.rule_id, "advisor");
    assert_eq!(group.title, "Labelled by the local model");
    assert_eq!((group.verdict, group.action), (Verdict::Review, ActionDto::Trash));
    assert_eq!(group.items, 1, "the keep label is not planned");
    assert_eq!(group.sample_paths, [root.join("q/stuff").to_string_lossy()]);
    assert!(plan.refused.is_empty() && plan.not_found.is_empty());
    assert!(plan.needs_confirmation, "it is confirmed like every plan");

    let wire = serde_json::to_value(&plan).unwrap();
    assert_eq!(wire["groups"][0]["action"], "trash");
    assert_eq!(wire["groups"][0]["verdict"], "review");
}

#[test]
fn the_advisor_group_is_refused_without_review_labels_and_while_the_advisor_is_off() {
    let state = AppState::for_test();
    switch(&state, true);
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);

    assert_eq!(
        plan_of(&state, scan, &["advisor"]).unwrap_err().code(),
        ApiErrorCode::Invalid,
        "no labels"
    );

    label(&state, scan, &[("q/photos", ModelVerdict::Keep)]);
    assert_eq!(
        plan_of(&state, scan, &["advisor"]).unwrap_err().code(),
        ApiErrorCode::Invalid,
        "only keep labels"
    );

    label(&state, scan, &[("q/stuff", ModelVerdict::Review)]);
    assert!(plan_of(&state, scan, &["advisor"]).is_ok());
    switch(&state, false);
    assert_eq!(
        plan_of(&state, scan, &["advisor"]).unwrap_err().code(),
        ApiErrorCode::Invalid,
        "the suggestion is not listed while the advisor is off"
    );
    assert_eq!(
        plan_of(&state, scan, &["rust-target"]).map(|plan| plan.groups.len()),
        Ok(1),
        "the rules do not need the advisor"
    );
}

#[test]
fn a_label_a_rule_covers_is_left_to_the_rule_and_never_planned_twice() {
    let state = AppState::for_test();
    switch(&state, true);
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    label(
        &state,
        scan,
        &[
            ("p/target", ModelVerdict::Review),
            ("p/target/debug", ModelVerdict::Review),
            ("p", ModelVerdict::Review),
            ("q/stuff", ModelVerdict::Review),
        ],
    );

    let plan = plan_of(&state, scan, &["rust-target", "advisor"]).unwrap();

    let groups: Vec<_> = plan
        .groups
        .iter()
        .map(|group| (group.rule_id.as_str(), group.items, group.sample_paths.clone()))
        .collect();
    assert_eq!(
        groups,
        [
            (
                "rust-target",
                1,
                vec![root.join("p/target").to_string_lossy().into_owned()]
            ),
            (
                "advisor",
                1,
                vec![root.join("q/stuff").to_string_lossy().into_owned()]
            ),
        ],
        "the label on the matched folder, the one inside it and the one holding it are not planned"
    );
    assert!(plan.refused.is_empty(), "{:?}", plan.refused);
}

#[test]
fn when_every_label_is_one_a_rule_speaks_for_there_is_nothing_to_plan() {
    let state = AppState::for_test();
    switch(&state, true);
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    label(
        &state,
        scan,
        &[("p/target", ModelVerdict::Review), ("p", ModelVerdict::Review)],
    );

    assert_eq!(
        plan_of(&state, scan, &["advisor"]).unwrap_err().code(),
        ApiErrorCode::Invalid
    );
}

#[test]
fn the_labels_do_not_outlive_the_disk_they_describe() {
    let state = AppState::for_test();
    switch(&state, true);
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    label(&state, scan, &[("q/stuff", ModelVerdict::Review)]);
    let plan = plan_of(&state, scan, &["advisor"]).unwrap();

    state.scans.disk_changed();

    assert_eq!(state.scans.labels(scan).unwrap(), None);
    assert_eq!(
        plan_of(&state, scan, &["advisor"]).unwrap_err().code(),
        ApiErrorCode::Invalid
    );
    let (started, _) = start_of(&state, plan.plan_id);
    assert_eq!(
        started.unwrap_err().code(),
        ApiErrorCode::Invalid,
        "a plan made before is refused too"
    );
}

#[test]
fn the_advisor_group_is_confirmed_in_words_that_name_the_model_and_the_trash() {
    let state = AppState::for_test();
    switch(&state, true);
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    label(&state, scan, &[("q/stuff", ModelVerdict::Review)]);
    let tree = state.scans.tree(scan).unwrap();
    let env = state.platform.environment_for(tree.root_path());
    let labelled = [ModelLabel {
        node: node_in(&state, scan, "q/stuff"),
        verdict: ModelVerdict::Review,
        reason: "because".to_owned(),
    }];
    let plan = plan_labelled(
        &tree,
        &env,
        &RuleSet::builtin(),
        &labelled,
        &state.platform.entry_probe,
    );

    let asked = question(&plan).expect("a plan with a group is asked about");

    assert!(
        asked
            .text
            .contains("• Move to the trash: Labelled by the local model, 1 items"),
        "{}",
        asked.text
    );
    assert!(asked.text.contains("can be put back"), "{}", asked.text);
}

// ---- The user's rules ----

/// A state whose rules file is in a temp folder (and the advisor switched on).
fn state_with_rules() -> (AppState, tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("platter/rules.json");
    let state = AppState::with_parts(
        crate::commands::advisor::AdvisorService::production(),
        crate::user_rules::UserRulesStore::new(Some(path.clone())),
    );
    switch(&state, true);

    (state, home, path)
}

fn say(state: &AppState, scan: ScanId, path: &str, verdict: crate::commands::rules::UserVerdictDto) {
    let node = node_in(state, scan, path);

    tauri::async_runtime::block_on(crate::commands::rules::add(state, scan, node, verdict))
        .unwrap_or_else(|error| panic!("the rule on {path} is added: {error:?}"));
}

use crate::commands::rules::UserVerdictDto::{Keep as KeepIt, Safe as SafeIt};

#[test]
fn a_user_keep_stops_a_rule_from_planning_the_folder_it_keeps_or_holds() {
    let (state, _home, _) = state_with_rules();
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    assert_eq!(
        plan_of(&state, scan, &["rust-target"]).unwrap().groups.len(),
        1,
        "control"
    );

    say(&state, scan, "p/target", KeepIt);
    let kept = plan_of(&state, scan, &["rust-target"]).unwrap();

    assert!(kept.groups.is_empty(), "{kept:?}");
    assert_eq!(kept.not_found, ["rust-target"]);

    // The same when the kept folder is the outer one.
    let (state, _home, _) = state_with_rules();
    let scan = scanned(&state, &root);
    say(&state, scan, "p", KeepIt);

    assert!(plan_of(&state, scan, &["rust-target"]).unwrap().groups.is_empty());
}

#[test]
fn a_user_keep_inside_a_cleaned_folder_stops_that_folder_being_planned() {
    let (state, _home, _) = state_with_rules();
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    say(&state, scan, "p/target/debug", KeepIt);

    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();

    assert!(
        plan.groups.is_empty(),
        "cleaning `target` would clean the kept folder: {plan:?}"
    );
}

#[test]
fn a_user_keep_refuses_marked_folders_inside_around_and_equal_to_it() {
    let (state, _home, _) = state_with_rules();
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    say(&state, scan, "p/target", KeepIt);
    let marked = ["p/target", "p/target/debug", "p"].map(|path| node_in(&state, scan, path));

    let plan = tauri::async_runtime::block_on(plan_marked_nodes(&state, scan, marked.to_vec())).unwrap();

    assert!(plan.groups.is_empty(), "{plan:?}");
    assert_eq!(plan.refused.len(), 3, "{plan:?}");
    assert!(
        plan.refused
            .iter()
            .all(|refused| refused.reason == "kept on purpose")
    );
}

#[test]
fn a_user_keep_blocks_the_advisor_plan() {
    let (state, _home, _) = state_with_rules();
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    label(&state, scan, &[("q/stuff", ModelVerdict::Review)]);
    assert!(plan_of(&state, scan, &["advisor"]).is_ok(), "control");

    say(&state, scan, "q/stuff", KeepIt);

    assert_eq!(
        plan_of(&state, scan, &["advisor"]).unwrap_err().code(),
        ApiErrorCode::Invalid
    );
}

#[test]
fn a_user_safe_folder_is_planned_for_the_trash_and_a_keep_id_is_never_planned() {
    let (state, _home, _) = state_with_rules();
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    say(&state, scan, "q/stuff", SafeIt);
    say(&state, scan, "q/photos", KeepIt);

    let plan = plan_of(&state, scan, &["user-safe", "user-keep"]).unwrap();

    assert_eq!(plan.groups.len(), 1, "{plan:?}");
    let group = &plan.groups[0];
    assert_eq!(group.rule_id, "user-safe");
    assert_eq!(group.title, "Folders you marked safe");
    assert_eq!((group.verdict, group.action), (Verdict::Safe, ActionDto::Trash));
    assert_eq!(group.sample_paths, [root.join("q/stuff").to_string_lossy()]);
    assert!(plan.needs_confirmation);
    assert!(
        plan.refused
            .iter()
            .any(|refused| refused.rule_id == "user-keep" && refused.reason == "kept on purpose"),
        "{plan:?}"
    );
}

#[test]
fn the_user_rule_ids_are_known_and_match_nothing_while_there_are_no_user_rules() {
    let state = AppState::for_test();
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);

    // The rule set always holds the two user entries, so the ids are known; with no rule they match nothing.
    let plan = plan_of(&state, scan, &["user-safe"]).unwrap();

    assert!(plan.groups.is_empty());
    assert_eq!(plan.not_found, ["user-safe"]);
}

#[test]
fn a_broken_rules_file_refuses_every_plan() {
    let (state, _home, path) = state_with_rules();
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    label(&state, scan, &[("q/stuff", ModelVerdict::Review)]);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"not json").unwrap();
    let node = node_in(&state, scan, "q/stuff");

    for refused in [
        plan_of(&state, scan, &["rust-target"]),
        plan_of(&state, scan, &["advisor"]),
        tauri::async_runtime::block_on(plan_marked_nodes(&state, scan, vec![node])),
    ] {
        let error = refused.unwrap_err();

        assert_eq!(error.code(), ApiErrorCode::Invalid);
        assert!(format!("{error:?}").contains("rules file"), "{error:?}");
    }
    assert_eq!(fs::read(&path).unwrap(), b"not json", "never overwritten");

    tauri::async_runtime::block_on(crate::commands::rules::reset(&state)).unwrap();
    assert!(
        plan_of(&state, scan, &["rust-target"]).is_ok(),
        "planning works again"
    );
}

#[test]
fn changing_a_rule_drops_the_plans_that_were_waiting() {
    let (state, _home, _) = state_with_rules();
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    let first = plan_of(&state, scan, &["rust-target"]).unwrap();
    let second = plan_of(&state, scan, &["rust-target"]).unwrap();

    say(&state, scan, "q/photos", KeepIt);

    for plan in [first, second] {
        let (started, _) = start_of(&state, plan.plan_id);
        assert_eq!(started.unwrap_err().code(), ApiErrorCode::NotFound, "dropped");
    }
    assert!(root.join("p/target/debug/obj").exists(), "nothing was cleaned");

    // Removing and starting over drop them too.
    let again = plan_of(&state, scan, &["rust-target"]).unwrap();
    let rule = tauri::async_runtime::block_on(crate::commands::rules::list(&state))
        .unwrap()
        .rules[0]
        .id;
    tauri::async_runtime::block_on(crate::commands::rules::remove(&state, rule)).unwrap();
    let (started, _) = start_of(&state, again.plan_id);
    assert_eq!(started.unwrap_err().code(), ApiErrorCode::NotFound);
}

#[test]
fn a_plan_taken_before_the_rules_changed_by_hand_is_refused_at_start() {
    let (state, _home, path) = state_with_rules();
    state.set_confirm(Arc::new(Approve));
    let (_dir, root) = project_and_stuff();
    let scan = scanned(&state, &root);
    let plan = plan_of(&state, scan, &["rust-target"]).unwrap();

    // An edit outside the app drops no plan, but the plan was made from other rules.
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            r#"{{"version":1,"next_id":2,"rules":[{{"id":1,"verdict":"keep","folder":"{}"}}]}}"#,
            root.join("p/target").display()
        ),
    )
    .unwrap();
    let (started, _) = start_of(&state, plan.plan_id);

    assert_eq!(started.unwrap_err().code(), ApiErrorCode::Invalid);
    assert!(root.join("p/target/debug/obj").exists(), "nothing was cleaned");
}
