//! Every refusal of the guard, on the in-memory filesystem. Planning never changes anything.

use std::path::{Path, PathBuf};

use platter_cleanup::{CleanupAction, CleanupPlan, Refusal, plan_cleanup};
use platter_core::dto::Verdict;
use platter_core::testing::{FakeFs, NOW};
use platter_core::{CancelToken, Environment, Os, ScanConfig, Tree, UserDir, scan};
use platter_rules::{Matcher, Rule, RuleSet};

const DAY: i64 = 86_400;

fn env() -> Environment {
    Environment {
        home: Some(PathBuf::from("/home/u")),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
    }
}

fn scan_at(fs: &FakeFs, root: &str) -> Tree {
    let config = ScanConfig {
        now: Some(NOW),
        ..ScanConfig::default()
    };
    scan(fs, Path::new(root), &config, &CancelToken::new(), &|_| {}).expect("the fake scans")
}

fn aged(fs: &mut FakeFs, path: &str, size: u64, days: i64) {
    fs.file(path, size).set_mtime(path, NOW - days * DAY);
}

/// An idle Rust project, an idle npm project, a cache, an old download and a pacman cache.
fn laptop() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/code/rs/Cargo.toml", 1);
    aged(&mut fs, "/home/u/code/rs/target/debug/x", 1000, 90);
    aged(&mut fs, "/home/u/code/js/package.json", 1, 90);
    aged(&mut fs, "/home/u/code/js/package-lock.json", 1, 90);
    aged(&mut fs, "/home/u/code/js/node_modules/dep/i.js", 500, 90);
    fs.file("/home/u/.cache/app/blob", 300);
    aged(&mut fs, "/home/u/Downloads/old.iso", 200, 800);
    fs.file("/var/cache/pacman/pkg/a.pkg", 100);
    fs
}

fn reasons(plan: &CleanupPlan) -> Vec<(&str, &str, &Refusal)> {
    plan.refused
        .iter()
        .map(|r| (r.rule_id, r.path.as_str(), &r.reason))
        .collect()
}

fn paths(plan: &CleanupPlan) -> Vec<String> {
    plan.targets().map(|t| t.display_path()).collect()
}

fn one_rule(id: &'static str, matcher: Matcher) -> RuleSet {
    RuleSet::new(vec![Rule::new(id, id, Verdict::Safe, matcher, "r")])
}

#[test]
fn a_dry_run_plans_every_selected_match_with_its_action() {
    let fs = laptop();
    let tree = scan_at(&fs, "/");
    let selected = [
        "rust-target",
        "node-modules",
        "app-cache",
        "old-downloads",
        "pacman-cache",
    ];

    let plan = plan_cleanup(&tree, &env(), &RuleSet::builtin(), &selected, &fs);

    let groups: Vec<_> = plan
        .groups()
        .iter()
        .map(|g| (g.rule_id(), g.action(), g.allocated()))
        .collect();
    assert_eq!(groups.len(), 5, "{plan:#?}");
    assert!(groups.contains(&("rust-target", CleanupAction::Delete, 1000)));
    assert!(groups.contains(&("node-modules", CleanupAction::Delete, 500)));
    assert!(groups.contains(&("app-cache", CleanupAction::EmptyFolder, 300)));
    assert!(groups.contains(&("old-downloads", CleanupAction::Trash, 200)));
    assert!(matches!(
        groups.iter().find(|g| g.0 == "pacman-cache").unwrap().1,
        CleanupAction::RunTool(tool) if tool.program() == "paccache" && tool.needs_elevation()
    ));
    assert_eq!(plan.allocated, 1000 + 500 + 300 + 200 + 100);
    assert!(plan.refused.is_empty(), "{plan:#?}");

    let target = plan
        .targets()
        .find(|t| t.display_path() == "/home/u/code/rs/target")
        .unwrap();
    assert!(target.is_dir());
    assert_ne!(
        target.identity(),
        (0, 0),
        "identity comes from the disk at plan time"
    );
}

#[test]
fn only_selected_rules_are_planned_and_unknown_ids_are_reported() {
    let fs = laptop();
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target", "no-such-rule", "apt-cache"],
        &fs,
    );

    assert_eq!(paths(&plan), ["/home/u/code/rs/target"]);
    assert_eq!(plan.not_found, ["no-such-rule", "apt-cache"]);
}

#[test]
fn keep_and_unsupported_rules_are_refused() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/Documents/a", 10).file("/.snapshots/1/f", 10);
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["documents", "btrfs-snapshots"],
        &fs,
    );

    assert!(plan.groups().is_empty());
    assert_eq!(
        reasons(&plan),
        [
            ("btrfs-snapshots", "/.snapshots", &Refusal::NotCleaned),
            ("documents", "/home/u/Documents", &Refusal::Kept),
        ]
    );
}

#[test]
fn without_a_home_directory_nothing_is_planned() {
    let fs = laptop();
    let tree = scan_at(&fs, "/");
    let no_home = Environment { home: None, ..env() };

    let plan = plan_cleanup(
        &tree,
        &no_home,
        &RuleSet::builtin(),
        &["rust-target", "pacman-cache"],
        &fs,
    );

    assert!(plan.groups().is_empty());
    assert!(!plan.refused.is_empty());
    assert!(plan.refused.iter().all(|r| r.reason == Refusal::HomeUnknown));
}

#[test]
fn the_scan_root_itself_is_never_a_target() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/a/b", 10);
    let tree = scan_at(&fs, "/home/u/.cache");

    let plan = plan_cleanup(&tree, &env(), &RuleSet::builtin(), &["app-cache"], &fs);

    assert_eq!(
        reasons(&plan),
        [("app-cache", "/home/u/.cache", &Refusal::ScanRoot)]
    );
}

#[test]
fn protected_places_are_refused_whatever_a_rule_says() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.ssh/id_ed25519", 1)
        .file("/home/u/.config/app/settings", 1)
        .file("/etc/x/conf", 1)
        .file("/home/u/Dokumenty/thesis.odt", 1)
        .file("/home/u/notes.txt", 1)
        .file("/var/lib/x", 1);
    let tree = scan_at(&fs, "/");
    let env = Environment {
        user_dirs: vec![(UserDir::Documents, PathBuf::from("/home/u/Dokumenty"))],
        ..env()
    };

    let cases = [
        ("app-cache", Matcher::Home(".ssh"), "/home/u/.ssh"),
        ("app-cache", Matcher::Home(".config/app"), "/home/u/.config"),
        ("coredumps", Matcher::Fixed("/etc/x"), "/etc"),
        ("trash", Matcher::Fixed("/home/u"), "/home/u"),
        ("trash", Matcher::Fixed("/home"), "/home"),
        ("trash", Matcher::Fixed("/var/lib"), "/var/lib"),
        ("npm-cache", Matcher::Home("Dokumenty"), "/home/u/Dokumenty"),
    ];
    for (id, matcher, place) in cases {
        let plan = plan_cleanup(&tree, &env, &one_rule(id, matcher), &[id], &fs);

        assert!(plan.groups().is_empty(), "{matcher:?}: {plan:#?}");
        assert_eq!(
            plan.refused[0].reason,
            Refusal::Protected(place.to_owned()),
            "{matcher:?}"
        );
    }
}

#[test]
fn a_target_swapped_for_a_link_after_the_scan_is_refused() {
    let mut fs = laptop();
    let tree = scan_at(&fs, "/");
    fs.swap_for_symlink("/home/u/code/rs/target");

    let plan = plan_cleanup(&tree, &env(), &RuleSet::builtin(), &["rust-target"], &fs);

    assert_eq!(
        reasons(&plan),
        [("rust-target", "/home/u/code/rs/target", &Refusal::Symlink)]
    );
}

#[test]
fn a_folder_on_the_way_swapped_for_a_link_is_refused() {
    let mut fs = laptop();
    let tree = scan_at(&fs, "/");
    fs.swap_for_symlink("/home/u/code");

    let plan = plan_cleanup(&tree, &env(), &RuleSet::builtin(), &["rust-target"], &fs);

    assert!(plan.groups().is_empty());
    assert!(matches!(&plan.refused[0].reason, Refusal::Changed(what) if what.contains("link")));
}

#[test]
fn a_target_that_is_gone_or_changed_kind_is_refused() {
    let mut fs = laptop();
    let tree = scan_at(&fs, "/");
    fs.remove("/home/u/code/rs/target");
    fs.remove("/home/u/code/js/node_modules")
        .file("/home/u/code/js/node_modules", 5);

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target", "node-modules"],
        &fs,
    );

    assert!(plan.groups().is_empty());
    assert_eq!(
        reasons(&plan),
        [
            (
                "rust-target",
                "/home/u/code/rs/target",
                &Refusal::Changed("it is gone".to_owned())
            ),
            (
                "node-modules",
                "/home/u/code/js/node_modules",
                &Refusal::Changed("it is now a file".to_owned())
            ),
        ]
    );
}

#[test]
fn a_mount_as_target_or_on_the_way_is_refused() {
    let mut fs = FakeFs::new();
    fs.same_filesystem_mount("/home/u/a/target");
    fs.file("/home/u/a/Cargo.toml", 1);
    aged(&mut fs, "/home/u/a/target/x", 10, 90);
    fs.same_filesystem_mount("/home/u/vol");
    fs.file("/home/u/vol/b/Cargo.toml", 1);
    aged(&mut fs, "/home/u/vol/b/target/x", 10, 90);
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(&tree, &env(), &RuleSet::builtin(), &["rust-target"], &fs);

    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert_eq!(plan.refused.len(), 2);
    assert!(plan.refused.iter().all(|r| r.reason == Refusal::Mount));
}

#[test]
fn a_mount_above_the_scan_root_is_fine() {
    let mut fs = FakeFs::new();
    fs.same_filesystem_mount("/home");
    fs.file("/home/u/a/Cargo.toml", 1);
    aged(&mut fs, "/home/u/a/target/x", 10, 90);
    let tree = scan_at(&fs, "/home/u");

    let plan = plan_cleanup(&tree, &env(), &RuleSet::builtin(), &["rust-target"], &fs);

    assert_eq!(paths(&plan), ["/home/u/a/target"]);
}

#[test]
fn a_review_target_with_an_unreadable_part_is_refused() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/a", 10);
    fs.unreadable_dir("/home/u/.cache/locked");
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(&tree, &env(), &RuleSet::builtin(), &["app-cache"], &fs);

    assert_eq!(
        reasons(&plan),
        [("app-cache", "/home/u/.cache", &Refusal::Unreadable)]
    );
}

#[test]
fn file_rules_plan_single_files() {
    let mut fs = FakeFs::new();
    aged(&mut fs, "/home/u/Downloads/a.iso", 10, 800);
    aged(&mut fs, "/home/u/Downloads/sub/b.zip", 20, 800);
    aged(&mut fs, "/home/u/Downloads/new.zip", 5, 3);
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(&tree, &env(), &RuleSet::builtin(), &["old-downloads"], &fs);

    let mut found = paths(&plan);
    found.sort();
    assert_eq!(found, ["/home/u/Downloads/a.iso", "/home/u/Downloads/sub/b.zip"]);
    assert!(plan.targets().all(|t| !t.is_dir()));
}

#[test]
fn a_part_platter_cannot_read_does_not_stop_a_tool_that_cleans_it() {
    let mut fs = FakeFs::new();
    fs.file("/var/cache/pacman/pkg/a.pkg", 100);
    fs.unreadable_dir("/var/cache/pacman/pkg/download-x");
    fs.file("/home/u/.cache/a", 10);
    fs.unreadable_dir("/home/u/.cache/locked");
    let tree = scan_at(&fs, "/var/cache");
    let home_tree = scan_at(&fs, "/");

    let tool = plan_cleanup(&tree, &env(), &RuleSet::builtin(), &["pacman-cache"], &fs);
    let own = plan_cleanup(&home_tree, &env(), &RuleSet::builtin(), &["app-cache"], &fs);

    assert_eq!(paths(&tool), ["/var/cache/pacman/pkg"], "{tool:#?}");
    assert_eq!(
        own.refused[0].reason,
        Refusal::Unreadable,
        "Platter's own deletes still refuse it"
    );
}

#[test]
fn a_file_modified_after_the_scan_or_a_project_whose_marker_is_gone_is_refused() {
    let mut fs = laptop();
    let tree = scan_at(&fs, "/");
    fs.set_mtime("/home/u/Downloads/old.iso", NOW);
    fs.remove("/home/u/code/rs/Cargo.toml");
    fs.remove("/home/u/code/js/package-lock.json");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["old-downloads", "rust-target", "node-modules"],
        &fs,
    );

    assert!(plan.groups().is_empty(), "{plan:#?}");
    let why: Vec<String> = plan
        .refused
        .iter()
        .map(|r| format!("{} {}", r.rule_id, r.reason))
        .collect();
    assert!(
        why.iter()
            .any(|w| w.starts_with("old-downloads") && w.contains("modified after the scan")),
        "{why:?}"
    );
    assert!(
        why.iter()
            .any(|w| w.starts_with("rust-target") && w.contains("(Cargo.toml)")),
        "{why:?}"
    );
    assert!(
        why.iter()
            .any(|w| w.starts_with("node-modules") && w.contains("package-lock.json or")),
        "{why:?}"
    );
}
