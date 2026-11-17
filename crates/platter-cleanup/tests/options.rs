//! The options fixed into a plan: how many package versions the pacman cache keeps, and whether tools that
//! need administrator rights are planned at all.

use std::path::{Path, PathBuf};

use platter_cleanup::{
    CleanupAction, CleanupOptions, CleanupPlan, KeepVersions, NEEDS_ADMIN_OFF, action_for, plan_cleanup,
    plan_marked,
};
use platter_core::testing::{FakeFs, NOW};
use platter_core::{CancelToken, Environment, Os, ScanConfig, Tree, scan};
use platter_rules::RuleSet;

fn env() -> Environment {
    Environment {
        home: Some(PathBuf::from("/home/u")),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
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

fn scan_root(fs: &FakeFs) -> Tree {
    let config = ScanConfig {
        now: Some(NOW),
        ..ScanConfig::default()
    };
    scan(fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).unwrap()
}

fn plan_with(options: &CleanupOptions, ids: &[&str]) -> CleanupPlan {
    let fs = system();

    plan_cleanup(&scan_root(&fs), &env(), &RuleSet::builtin(), ids, &fs, options)
}

fn options(keep: KeepVersions, ask_for_admin: bool) -> CleanupOptions {
    CleanupOptions {
        keep_package_versions: keep,
        ask_for_admin,
    }
}

fn tool_args(action: Option<CleanupAction>) -> Vec<&'static str> {
    match action {
        Some(CleanupAction::RunTool(tool)) => tool.args().to_vec(),
        other => panic!("not a tool: {other:?}"),
    }
}

#[test]
fn the_pacman_cache_keeps_the_chosen_number_of_versions() {
    for (keep, flag) in [
        (KeepVersions::One, "-rk1"),
        (KeepVersions::Two, "-rk2"),
        (KeepVersions::Three, "-rk3"),
    ] {
        let action = action_for("pacman-cache", &options(keep, true));

        assert_eq!(tool_args(action), [flag]);
    }
}

#[test]
fn the_options_change_no_other_tool() {
    let one = options(KeepVersions::One, true);
    let three = options(KeepVersions::Three, false);

    for rule in [
        "apt-cache",
        "dnf-cache",
        "journal",
        "docker-data",
        "coredumps",
        "rust-target",
        "trash",
    ] {
        assert_eq!(action_for(rule, &one), action_for(rule, &three), "{rule}");
    }
}

#[test]
fn only_one_two_and_three_are_versions_to_keep() {
    assert_eq!(KeepVersions::from_count(1), Some(KeepVersions::One));
    assert_eq!(KeepVersions::from_count(2), Some(KeepVersions::Two));
    assert_eq!(KeepVersions::from_count(3), Some(KeepVersions::Three));
    assert_eq!(KeepVersions::from_count(0), None);
    assert_eq!(KeepVersions::from_count(4), None);
    assert_eq!(KeepVersions::Three.count(), 3);
}

#[test]
fn the_default_plan_keeps_two_versions_and_asks_for_admin() {
    let plan = plan_with(
        &CleanupOptions::default(),
        &["pacman-cache", "journal", "docker-data"],
    );

    assert_eq!(*plan.options(), options(KeepVersions::Two, true));
    assert!(plan.left_out.is_empty());
    let pacman = plan
        .groups()
        .iter()
        .find(|g| g.rule_id() == "pacman-cache")
        .unwrap();
    assert_eq!(tool_args(Some(pacman.action())), ["-rk2"]);
    assert_eq!(plan.groups().len(), 3);
}

#[test]
fn a_plan_with_admin_off_leaves_out_every_tool_that_needs_it() {
    let plan = plan_with(
        &options(KeepVersions::Two, false),
        &["pacman-cache", "journal", "docker-data", "app-cache"],
    );

    let rules: Vec<_> = plan.groups().iter().map(|group| group.rule_id()).collect();
    assert_eq!(rules, ["app-cache", "docker-data"]);
    assert!(plan.groups().iter().all(|group| match group.action() {
        CleanupAction::RunTool(tool) => !tool.needs_elevation(),
        _ => true,
    }));
    let mut left_out: Vec<_> = plan
        .left_out
        .iter()
        .map(|item| (item.rule_id, item.path.as_str(), item.reason))
        .collect();
    left_out.sort_unstable();
    assert_eq!(
        left_out,
        [
            ("journal", "/var/log/journal", NEEDS_ADMIN_OFF),
            ("pacman-cache", "/var/cache/pacman/pkg", NEEDS_ADMIN_OFF),
        ]
    );
    assert!(plan.refused.is_empty(), "left out is not refused: {plan:#?}");
    assert_eq!(
        NEEDS_ADMIN_OFF,
        "Needs administrator rights, which are turned off in Settings."
    );
}

#[test]
fn a_plan_of_only_elevated_tools_is_empty_when_admin_is_off() {
    let plan = plan_with(&options(KeepVersions::Two, false), &["pacman-cache"]);

    assert!(plan.groups().is_empty());
    assert_eq!(plan.left_out.len(), 1);
}

#[test]
fn a_marked_plan_remembers_the_options_and_is_not_left_out() {
    let fs = system();
    let tree = scan_root(&fs);
    let chosen = options(KeepVersions::Three, false);

    let plan = plan_marked(&tree, &env(), &RuleSet::builtin(), &[], &fs, &chosen);

    assert_eq!(*plan.options(), chosen);
    assert!(plan.left_out.is_empty());
}

#[test]
fn absorbing_a_plan_keeps_what_was_left_out() {
    let mut first = plan_with(&options(KeepVersions::One, false), &["journal"]);
    let second = plan_with(&options(KeepVersions::One, false), &["pacman-cache"]);

    first.absorb(second);

    assert_eq!(first.left_out.len(), 2);
    assert_eq!(*first.options(), options(KeepVersions::One, false));
}
