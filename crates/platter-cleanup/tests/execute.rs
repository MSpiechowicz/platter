//! Running a plan against a recording remover: what is acted on, in what order, and what is left alone.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use platter_cleanup::{
    CleanupEvent, CleanupOptions, Disk, KeepVersions, TargetOutcome, execute, plan_cleanup,
};
use platter_core::testing::{FakeFs, NOW};
use platter_core::{
    CancelToken, EnteredMount, EntryRemover, Environment, ExpectedEntry, Os, RemoveMode, Removed, ScanConfig,
    ToolCommand, ToolFailure, ToolRunner, Tree, scan,
};
use platter_rules::RuleSet;

const DAY: i64 = 86_400;

fn env() -> Environment {
    Environment {
        home: Some(PathBuf::from("/home/u")),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
        system_root: None,
    }
}

fn scan_root(fs: &FakeFs) -> Tree {
    let config = ScanConfig {
        now: Some(NOW),
        ..ScanConfig::default()
    };
    scan(fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).unwrap()
}

fn aged(fs: &mut FakeFs, path: &str, size: u64, days: i64) {
    fs.file(path, size).set_mtime(path, NOW - days * DAY);
}

fn projects(count: usize) -> FakeFs {
    let mut fs = FakeFs::new();
    for i in 0..count {
        fs.file(&format!("/home/u/p{i}/Cargo.toml"), 1);
        aged(&mut fs, &format!("/home/u/p{i}/target/x"), 10, 90);
    }
    fs.file("/home/u/.cache/a", 5);
    aged(&mut fs, "/home/u/Downloads/old.iso", 7, 800);
    fs.file("/var/cache/pacman/pkg/a", 3);
    fs
}

/// Records every call; fails the paths it is told to.
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<(String, &'static str)>>,
    fail: Vec<String>,
    /// The mounts the plan allowed, as the last call to `remove` was told.
    entered: Mutex<Vec<EnteredMount>>,
}

impl Recorder {
    fn calls(&self) -> Vec<(String, &'static str)> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, path: &[u8], what: &'static str) -> io::Result<()> {
        let path = String::from_utf8_lossy(path).into_owned();
        self.calls.lock().unwrap().push((path.clone(), what));
        if self.fail.contains(&path) {
            return Err(io::Error::other("permission denied"));
        }
        Ok(())
    }
}

impl EntryRemover for Recorder {
    fn remove(&self, path: &[u8], expected: &ExpectedEntry, mode: RemoveMode) -> io::Result<Removed> {
        assert_eq!(expected.root_depth, 0, "the plan's scan root depth is passed on");
        self.entered.lock().unwrap().clone_from(&expected.entered_mounts);
        self.record(
            path,
            if mode == RemoveMode::Whole {
                "delete"
            } else {
                "empty"
            },
        )?;
        Ok(Removed {
            entries: 1,
            ..Removed::default()
        })
    }

    fn trash(&self, path: &[u8], _expected: &ExpectedEntry, trash_dir: &Path) -> io::Result<Vec<u8>> {
        self.record(path, "trash")?;
        Ok(trash_dir.join("files/x").into_os_string().into_encoded_bytes())
    }

    fn free_bytes(&self, _path: &Path) -> io::Result<u64> {
        let done = self.calls.lock().unwrap().len() as u64;
        Ok(1000 + done * 10)
    }
}

const ALL: [&str; 4] = ["rust-target", "app-cache", "old-downloads", "pacman-cache"];

#[test]
fn every_target_gets_its_action_and_tools_are_not_run_yet() {
    let fs = projects(2);
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &ALL,
        &fs,
        &CleanupOptions::default(),
    );
    let recorder = Recorder::default();
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: Some(Path::new("/home/u/.local/share/Trash")),
        tools: None,
    };
    let mut events = Vec::new();

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |event| events.push(event));

    let mut calls = recorder.calls();
    calls.sort();
    assert_eq!(
        calls,
        [
            ("/home/u/.cache".to_owned(), "empty"),
            ("/home/u/Downloads/old.iso".to_owned(), "trash"),
            ("/home/u/p0/target".to_owned(), "delete"),
            ("/home/u/p1/target".to_owned(), "delete"),
        ]
    );
    assert_eq!(report.tools_not_run, ["pacman-cache"]);
    assert!(!report.cancelled);
    assert_eq!(
        report.freed,
        Some(40),
        "measured from free space, not from the plan"
    );
    assert_eq!(report.results.len(), 4);
    assert!(matches!(&events[0], CleanupEvent::GroupStarted { .. }));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, CleanupEvent::TargetDone(_)))
            .count(),
        4
    );
}

#[test]
fn a_target_changed_after_the_plan_is_left_alone_and_the_rest_goes_on() {
    let mut fs = projects(3);
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );
    fs.swap_for_symlink("/home/u/p1/target");
    let recorder = Recorder::default();
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: None,
        tools: None,
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    let touched: Vec<String> = recorder.calls().into_iter().map(|(path, _)| path).collect();
    assert!(!touched.contains(&"/home/u/p1/target".to_owned()));
    assert_eq!(touched.len(), 2);
    let skipped = report
        .results
        .iter()
        .find(|r| r.path == "/home/u/p1/target")
        .unwrap();
    assert!(
        matches!(&skipped.outcome, TargetOutcome::Skipped(why) if why.contains("changed since the plan"))
    );
}

/// A home folder on its own mount of the same disk, scanned from `/`.
fn home_on_its_own_mount() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.same_filesystem_mount("/home");
    fs.file("/home/u/p0/Cargo.toml", 1);
    aged(&mut fs, "/home/u/p0/target/x", 10, 90);
    fs
}

/// Executes a plan of the `rust-target` rule made on `before` against what the disk is now (`fs`).
fn run_rust_target(before: &FakeFs, fs: &FakeFs) -> (Recorder, platter_cleanup::CleanupReport) {
    let plan = plan_cleanup(
        &scan_root(before),
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        before,
        &CleanupOptions::default(),
    );
    assert_eq!(plan.targets().count(), 1, "{:?}", plan.refused);

    let recorder = Recorder::default();
    let disk = Disk {
        probe: fs,
        remover: &recorder,
        trash_dir: None,
        tools: None,
    };
    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    (recorder, report)
}

fn skipped_for_a_mount(report: &platter_cleanup::CleanupReport) -> bool {
    matches!(
        &report.results[0].outcome,
        TargetOutcome::Skipped(why) if why.contains("a mount lies on the way")
    )
}

#[test]
fn a_target_below_a_mount_the_scan_went_into_is_removed() {
    let fs = home_on_its_own_mount();

    let (recorder, report) = run_rust_target(&fs, &fs);

    assert_eq!(recorder.calls(), [("/home/u/p0/target".to_owned(), "delete")]);
    assert!(matches!(&report.results[0].outcome, TargetOutcome::Removed(_)));
    let (dev, ino) = fs.identity("/home");
    let entered = recorder.entered.lock().unwrap().clone();
    assert_eq!(
        entered,
        [EnteredMount {
            depth: 0,
            dev,
            ino,
            mount_id: None
        }],
        "the remover is told which mount may be crossed"
    );
}

#[test]
fn a_different_mount_at_the_same_place_is_refused() {
    let before = home_on_its_own_mount();
    let mut now = home_on_its_own_mount();
    now.set_identity("/home", 77, 5, 1);

    let (recorder, report) = run_rust_target(&before, &now);

    assert!(recorder.calls().is_empty());
    assert!(skipped_for_a_mount(&report), "{report:?}");
}

#[test]
fn a_mount_that_appeared_after_the_plan_is_refused() {
    let before = home_on_its_own_mount();
    let mut now = home_on_its_own_mount();
    now.mount("/home/u/p0");

    let (recorder, report) = run_rust_target(&before, &now);

    assert!(recorder.calls().is_empty());
    assert!(skipped_for_a_mount(&report), "{report:?}");
}

#[test]
fn a_mount_on_the_way_that_the_plan_never_saw_is_refused() {
    let mut before = FakeFs::new();
    before.file("/home/u/p0/Cargo.toml", 1);
    aged(&mut before, "/home/u/p0/target/x", 10, 90);
    let mut now = FakeFs::new();
    now.same_filesystem_mount("/home");
    now.file("/home/u/p0/Cargo.toml", 1);
    aged(&mut now, "/home/u/p0/target/x", 10, 90);

    let (recorder, report) = run_rust_target(&before, &now);

    assert!(recorder.calls().is_empty());
    assert!(skipped_for_a_mount(&report), "{report:?}");
}

#[test]
fn nothing_is_acted_on_when_the_scanned_folder_was_replaced_after_the_plan() {
    let mut fs = projects(2);
    let config = ScanConfig {
        now: Some(NOW),
        ..ScanConfig::default()
    };
    let tree = scan(&fs, Path::new("/home/u"), &config, &CancelToken::new(), &|_| {}).unwrap();
    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );
    assert_eq!(plan.targets().count(), 2);

    // Another folder (same names) is now at the scan root's place.
    let (dev, ino) = fs.identity("/home/u");
    fs.set_identity("/home/u", dev, ino + 500, 1);
    let recorder = Recorder::default();
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: None,
        tools: None,
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    assert!(recorder.calls().is_empty());
    assert_eq!(report.results.len(), 2);
    assert!(report.results.iter().all(|result| matches!(
        &result.outcome,
        TargetOutcome::Skipped(why) if why.contains("not the one that was scanned. Scan again.")
    )));
}

#[test]
fn a_failure_on_one_target_does_not_stop_the_others() {
    let fs = projects(3);
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );
    let recorder = Recorder {
        fail: vec!["/home/u/p0/target".to_owned()],
        ..Recorder::default()
    };
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: None,
        tools: None,
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    assert_eq!(recorder.calls().len(), 3);
    let failed: Vec<_> = report
        .results
        .iter()
        .filter(|r| matches!(r.outcome, TargetOutcome::Failed(_)))
        .map(|r| r.path.as_str())
        .collect();
    assert_eq!(failed, ["/home/u/p0/target"]);
}

#[test]
fn cancelling_stops_between_targets() {
    let fs = projects(5);
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );
    let recorder = Recorder::default();
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: None,
        tools: None,
    };
    let cancel = CancelToken::new();

    let mut finished = 0;
    let report = execute(&plan, &disk, &cancel, &mut |event| match event {
        CleanupEvent::TargetDone(_) => cancel.cancel(),
        CleanupEvent::GroupFinished { .. } => finished += 1,
        CleanupEvent::GroupStarted { .. } => {}
    });

    assert_eq!(finished, 1, "a cancelled group is still closed");
    assert!(report.cancelled);
    assert_eq!(
        recorder.calls().len(),
        1,
        "the target in progress finished, no other began"
    );
    assert_eq!(report.results.len(), 1);
}

#[test]
fn without_a_trash_folder_nothing_is_trashed() {
    let fs = projects(0);
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["old-downloads"],
        &fs,
        &CleanupOptions::default(),
    );
    let recorder = Recorder::default();
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: None,
        tools: None,
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    assert!(recorder.calls().is_empty());
    assert!(matches!(&report.results[0].outcome, TargetOutcome::Skipped(why) if why.contains("trash")));
}

/// Answers each program with a fixed result and records what ran.
struct Tools {
    answers: Vec<(&'static str, Result<(), ToolFailure>)>,
    ran: Mutex<Vec<String>>,
}

impl ToolRunner for Tools {
    fn run(&self, command: &ToolCommand<'_>) -> Result<(), ToolFailure> {
        self.ran.lock().unwrap().push(format!(
            "{}{}",
            if command.elevated { "sudo " } else { "" },
            command.program
        ));
        self.answers
            .iter()
            .find(|(program, _)| *program == command.program)
            .map_or(Ok(()), |(_, answer)| answer.clone())
    }
}

fn system() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/var/cache/pacman/pkg/a", 3)
        .file("/var/log/journal/x", 4)
        .file("/var/lib/docker/y", 5)
        .file("/home/u/.cache/a", 5);
    fs
}

#[test]
fn owning_tools_run_once_per_rule_with_their_fixed_arguments() {
    let fs = system();
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["pacman-cache", "docker-data"],
        &fs,
        &CleanupOptions::default(),
    );
    let recorder = Recorder::default();
    let tools = Tools {
        answers: Vec::new(),
        ran: Mutex::new(Vec::new()),
    };
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: None,
        tools: Some(&tools),
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    let mut ran = tools.ran.lock().unwrap().clone();
    ran.sort();
    assert_eq!(ran, ["docker", "sudo paccache"]);
    assert!(
        recorder.calls().is_empty(),
        "a tool's folder is never deleted by Platter itself"
    );
    assert!(report.tools_not_run.is_empty());
    assert!(
        report
            .results
            .iter()
            .all(|r| matches!(&r.outcome, TargetOutcome::ByTool(_)))
    );
}

/// Records the arguments of every tool that ran.
struct ArgTools {
    ran: Mutex<Vec<(String, Vec<String>)>>,
}

impl ToolRunner for ArgTools {
    fn run(&self, command: &ToolCommand<'_>) -> Result<(), ToolFailure> {
        self.ran.lock().unwrap().push((
            command.program.to_owned(),
            command.args.iter().map(|arg| (*arg).to_owned()).collect(),
        ));

        Ok(())
    }
}

#[test]
fn a_plan_made_with_one_version_runs_the_pacman_cache_tool_with_one_version() {
    let fs = system();
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["pacman-cache"],
        &fs,
        &CleanupOptions {
            keep_package_versions: KeepVersions::One,
            ask_for_admin: true,
        },
    );
    let recorder = Recorder::default();
    let tools = ArgTools {
        ran: Mutex::new(Vec::new()),
    };
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: None,
        tools: Some(&tools),
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    assert_eq!(
        *tools.ran.lock().unwrap(),
        [("paccache".to_owned(), vec!["-rk1".to_owned()])]
    );
    assert!(recorder.calls().is_empty());
    assert!(matches!(&report.results[0].outcome, TargetOutcome::ByTool(_)));
}

#[test]
fn a_refused_password_is_asked_for_only_once_per_run() {
    let fs = system();
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["pacman-cache", "journal", "docker-data"],
        &fs,
        &CleanupOptions::default(),
    );
    let recorder = Recorder::default();
    let tools = Tools {
        answers: vec![
            ("paccache", Err(ToolFailure::NotAuthorized)),
            ("journalctl", Err(ToolFailure::NotAuthorized)),
        ],
        ran: Mutex::new(Vec::new()),
    };
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: None,
        tools: Some(&tools),
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    let ran = tools.ran.lock().unwrap().clone();
    assert_eq!(ran.iter().filter(|r| r.starts_with("sudo")).count(), 1, "{ran:?}");
    assert!(
        ran.contains(&"docker".to_owned()),
        "a tool that needs no password still runs"
    );
    let refused = report
        .results
        .iter()
        .filter(|r| matches!(&r.outcome, TargetOutcome::Skipped(why) if why.contains("password")))
        .count();
    assert_eq!(refused, 2);
}

/// Answers elevated commands only as one batch, and records every batch and every single run.
struct BatchTools {
    batch_answers: Vec<Result<(), ToolFailure>>,
    batches: Mutex<Vec<Vec<String>>>,
    singles: Mutex<Vec<String>>,
}

impl BatchTools {
    fn new(batch_answers: Vec<Result<(), ToolFailure>>) -> Self {
        Self {
            batch_answers,
            batches: Mutex::new(Vec::new()),
            singles: Mutex::new(Vec::new()),
        }
    }
}

impl ToolRunner for BatchTools {
    fn run(&self, command: &ToolCommand<'_>) -> Result<(), ToolFailure> {
        assert!(!command.elevated, "an elevated command goes through the batch");
        self.singles.lock().unwrap().push(command.program.to_owned());

        Ok(())
    }

    fn run_batch(&self, commands: &[ToolCommand<'_>]) -> Vec<Result<(), ToolFailure>> {
        assert!(commands.iter().all(|command| command.elevated));
        self.batches.lock().unwrap().push(
            commands
                .iter()
                .map(|command| command.program.to_owned())
                .collect(),
        );

        self.batch_answers.clone()
    }
}

fn run_with_batch(
    fs: &FakeFs,
    selected: &[&str],
    options: &CleanupOptions,
    tools: &BatchTools,
    cancel: &CancelToken,
) -> (platter_cleanup::CleanupReport, Vec<String>) {
    let plan = plan_cleanup(&scan_root(fs), &env(), &RuleSet::builtin(), selected, fs, options);
    let recorder = Recorder::default();
    let disk = Disk {
        probe: fs,
        remover: &recorder,
        trash_dir: None,
        tools: Some(tools),
    };

    let mut events = Vec::new();
    let report = execute(&plan, &disk, cancel, &mut |event| {
        events.push(match event {
            CleanupEvent::GroupStarted { rule_id, .. } => format!("start {rule_id}"),
            CleanupEvent::TargetDone(result) => format!("done {}", result.rule_id),
            CleanupEvent::GroupFinished { rule_id } => format!("finish {rule_id}"),
        });
    });

    (report, events)
}

fn outcome_of(report: &platter_cleanup::CleanupReport, id: &str) -> TargetOutcome {
    report
        .results
        .iter()
        .find(|r| r.rule_id == id)
        .unwrap()
        .outcome
        .clone()
}

#[test]
fn elevated_groups_go_through_one_batch_and_each_reports_its_own_answer_in_place() {
    let fs = system();
    let tools = BatchTools::new(vec![
        Ok(()),
        Err(ToolFailure::Failed {
            code: Some(2),
            message: "no details".to_owned(),
        }),
    ]);

    let (report, events) = run_with_batch(
        &fs,
        &["pacman-cache", "docker-data", "journal"],
        &CleanupOptions::default(),
        &tools,
        &CancelToken::new(),
    );

    assert_eq!(*tools.batches.lock().unwrap(), [["paccache", "journalctl"]]);
    assert_eq!(*tools.singles.lock().unwrap(), ["docker"]);
    assert!(matches!(
        outcome_of(&report, "pacman-cache"),
        TargetOutcome::ByTool(_)
    ));
    assert!(matches!(
        outcome_of(&report, "docker-data"),
        TargetOutcome::ByTool(_)
    ));
    assert!(
        matches!(outcome_of(&report, "journal"), TargetOutcome::Failed(why) if why.contains("code 2: no details"))
    );
    assert_eq!(
        events,
        [
            "start pacman-cache",
            "done pacman-cache",
            "finish pacman-cache",
            "start journal",
            "done journal",
            "finish journal",
            "start docker-data",
            "done docker-data",
            "finish docker-data",
        ]
    );
}

#[test]
fn a_refusal_of_the_batch_stops_every_elevated_group_after_one_prompt() {
    let fs = system();
    let tools = BatchTools::new(vec![
        Err(ToolFailure::NotAuthorized),
        Err(ToolFailure::NotAuthorized),
    ]);

    let (report, _) = run_with_batch(
        &fs,
        &["pacman-cache", "docker-data", "journal"],
        &CleanupOptions::default(),
        &tools,
        &CancelToken::new(),
    );

    assert_eq!(tools.batches.lock().unwrap().len(), 1);
    for id in ["pacman-cache", "journal"] {
        assert!(
            matches!(outcome_of(&report, id), TargetOutcome::Skipped(why) if why.contains("password")),
            "{id}"
        );
    }
    assert!(
        matches!(outcome_of(&report, "docker-data"), TargetOutcome::ByTool(_)),
        "a tool that needs no password still runs"
    );
}

#[test]
fn no_batch_is_asked_for_when_no_group_needs_the_password_or_the_run_is_cancelled_first() {
    let fs = system();

    let tools = BatchTools::new(Vec::new());
    run_with_batch(
        &fs,
        &["docker-data"],
        &CleanupOptions::default(),
        &tools,
        &CancelToken::new(),
    );
    assert!(tools.batches.lock().unwrap().is_empty());

    let tools = BatchTools::new(Vec::new());
    run_with_batch(
        &fs,
        &["pacman-cache", "journal"],
        &CleanupOptions {
            ask_for_admin: false,
            ..CleanupOptions::default()
        },
        &tools,
        &CancelToken::new(),
    );
    assert!(
        tools.batches.lock().unwrap().is_empty(),
        "rules left out by the options are not asked for"
    );

    let cancel = CancelToken::new();
    cancel.cancel();
    let tools = BatchTools::new(Vec::new());
    let (report, _) = run_with_batch(
        &fs,
        &["pacman-cache", "journal"],
        &CleanupOptions::default(),
        &tools,
        &cancel,
    );
    assert!(report.cancelled);
    assert!(tools.batches.lock().unwrap().is_empty());
}

#[test]
fn a_missing_or_failing_tool_is_reported() {
    let fs = system();
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["pacman-cache", "docker-data"],
        &fs,
        &CleanupOptions::default(),
    );
    let recorder = Recorder::default();
    let tools = Tools {
        answers: vec![
            ("paccache", Err(ToolFailure::NotInstalled)),
            (
                "docker",
                Err(ToolFailure::Failed {
                    code: Some(1),
                    message: "daemon not running".to_owned(),
                }),
            ),
        ],
        ran: Mutex::new(Vec::new()),
    };
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: None,
        tools: Some(&tools),
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    let outcome = |id: &str| {
        report
            .results
            .iter()
            .find(|r| r.rule_id == id)
            .unwrap()
            .outcome
            .clone()
    };
    assert_eq!(
        outcome("pacman-cache"),
        TargetOutcome::Skipped("paccache is not installed".to_owned())
    );
    assert!(
        matches!(outcome("docker-data"), TargetOutcome::Failed(why) if why.contains("code 1: daemon not running"))
    );
}

#[test]
fn a_file_modified_or_a_marker_removed_after_the_plan_is_left_alone() {
    let mut fs = projects(2);
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["rust-target", "old-downloads"],
        &fs,
        &CleanupOptions::default(),
    );
    fs.set_mtime("/home/u/Downloads/old.iso", NOW);
    fs.remove("/home/u/p0/Cargo.toml");
    let recorder = Recorder::default();
    let disk = Disk {
        probe: &fs,
        remover: &recorder,
        trash_dir: Some(Path::new("/t")),
        tools: None,
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    let touched: Vec<String> = recorder.calls().into_iter().map(|(path, _)| path).collect();
    assert_eq!(touched, ["/home/u/p1/target"]);
    let skipped: Vec<&str> = report
        .results
        .iter()
        .filter(
            |r| matches!(&r.outcome, TargetOutcome::Skipped(why) if why.contains("changed since the plan")),
        )
        .map(|r| r.path.as_str())
        .collect();
    assert_eq!(skipped.len(), 2, "{report:#?}");
}

#[test]
fn a_workspace_file_removed_after_the_plan_leaves_the_member_alone() {
    let mut fs = FakeFs::new();
    aged(&mut fs, "/home/u/code/ws/package.json", 1, 90);
    aged(&mut fs, "/home/u/code/ws/pnpm-workspace.yaml", 1, 90);
    aged(&mut fs, "/home/u/code/ws/pnpm-lock.yaml", 1, 90);
    aged(&mut fs, "/home/u/code/ws/packages/a/package.json", 1, 90);
    aged(
        &mut fs,
        "/home/u/code/ws/packages/a/node_modules/dep/i.js",
        500,
        90,
    );
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["node-modules"],
        &fs,
        &CleanupOptions::default(),
    );
    assert_eq!(plan.targets().count(), 1, "{:?}", plan.refused);

    let run = |fs: &FakeFs| {
        let recorder = Recorder::default();
        let disk = Disk {
            probe: fs,
            remover: &recorder,
            trash_dir: Some(Path::new("/t")),
            tools: None,
        };
        let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

        (recorder.calls(), report)
    };

    let (calls, _) = run(&fs);
    assert_eq!(calls.len(), 1, "the unchanged workspace is acted on");

    fs.remove("/home/u/code/ws/pnpm-lock.yaml");
    let (calls, report) = run(&fs);
    assert!(calls.is_empty(), "{calls:?}");
    assert!(
        report.results.iter().any(|r| matches!(
            &r.outcome,
            TargetOutcome::Skipped(why) if why.contains("changed since the plan") && why.contains("pnpm-lock.yaml")
        )),
        "{report:#?}"
    );
}
