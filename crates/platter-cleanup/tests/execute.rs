//! Running a plan against a recording remover: what is acted on, in what order, and what is left alone.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use platter_cleanup::{CleanupEvent, Disk, TargetOutcome, execute, plan_cleanup};
use platter_core::testing::{FakeFs, NOW};
use platter_core::{
    CancelToken, EntryRemover, Environment, ExpectedEntry, Os, RemoveMode, Removed, ScanConfig, ToolCommand,
    ToolFailure, ToolRunner, Tree, scan,
};
use platter_rules::RuleSet;

const DAY: i64 = 86_400;

fn env() -> Environment {
    Environment {
        home: Some(PathBuf::from("/home/u")),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
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
    let plan = plan_cleanup(&scan_root(&fs), &env(), &RuleSet::builtin(), &ALL, &fs);
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

#[test]
fn a_failure_on_one_target_does_not_stop_the_others() {
    let fs = projects(3);
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
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

#[test]
fn a_refused_password_is_asked_for_only_once_per_run() {
    let fs = system();
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["pacman-cache", "journal", "docker-data"],
        &fs,
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

#[test]
fn a_missing_or_failing_tool_is_reported() {
    let fs = system();
    let plan = plan_cleanup(
        &scan_root(&fs),
        &env(),
        &RuleSet::builtin(),
        &["pacman-cache", "docker-data"],
        &fs,
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
