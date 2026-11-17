//! A folder the user left out of the scan is a `mount` node with the reason `excluded`, so everything that
//! refuses a mount refuses it: rules do not match it, no plan targets it and the model is not asked about it.

use std::path::{Path, PathBuf};

use platter_cleanup::{
    CandidateLimits, CleanupOptions, Refusal, label_candidates, plan_cleanup, plan_marked,
};
use platter_core::testing::{FakeFs, NOW};
use platter_core::{CancelToken, Environment, NodeId, NodeKind, Os, ScanConfig, SkipReason, Tree, scan};
use platter_rules::{RuleSet, evaluate, matches};

const DAY: i64 = 86_400;

fn env() -> Environment {
    Environment {
        home: Some(PathBuf::from("/home/u")),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
    }
}

fn scan_excluding(fs: &FakeFs, excluded: &[&str]) -> Tree {
    let config = ScanConfig {
        now: Some(NOW),
        excluded: excluded.iter().map(|path| fs.identity(path)).collect(),
        ..ScanConfig::default()
    };

    scan(fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).expect("the fake scans")
}

fn aged(fs: &mut FakeFs, path: &str, size: u64, days: i64) {
    fs.file(path, size).set_mtime(path, NOW - days * DAY);
}

fn node(tree: &Tree, path: &str) -> NodeId {
    let mut current = tree.root();

    for part in path.split('/').filter(|part| !part.is_empty()) {
        current = tree
            .children(current)
            .find(|&child| tree.name(child) == part.as_bytes())
            .expect("the path exists");
    }
    current
}

/// An idle Rust project and an idle npm project, a folder of old files and a big unknown folder.
fn laptop() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/code/rs/Cargo.toml", 1);
    aged(&mut fs, "/home/u/code/rs/target/debug/x", 1000, 90);
    aged(&mut fs, "/home/u/code/js/package.json", 1, 90);
    aged(&mut fs, "/home/u/code/js/package-lock.json", 1, 90);
    aged(&mut fs, "/home/u/code/js/node_modules/dep/i.js", 500, 90);
    aged(&mut fs, "/home/u/old/stuff/a.bin", 700, 10);
    fs.file("/home/u/big/blob/data", 900);
    fs
}

fn selected() -> [&'static str; 2] {
    ["rust-target", "node-modules"]
}

fn planned_paths(tree: &Tree, fs: &FakeFs) -> Vec<String> {
    let plan = plan_cleanup(
        tree,
        &env(),
        &RuleSet::builtin(),
        &selected(),
        fs,
        &CleanupOptions::default(),
    );

    plan.targets().map(|target| target.display_path()).collect()
}

#[test]
fn control_without_an_exclusion_the_rules_plan_both_folders() {
    let fs = laptop();
    let tree = scan_excluding(&fs, &[]);

    let mut paths = planned_paths(&tree, &fs);
    paths.sort();

    assert_eq!(paths, ["/home/u/code/js/node_modules", "/home/u/code/rs/target"]);
}

#[test]
fn a_folder_matching_a_rule_by_name_is_not_matched_once_it_is_excluded() {
    let fs = laptop();
    let tree = scan_excluding(&fs, &["/home/u/code/rs/target", "/home/u/code/js/node_modules"]);

    assert_eq!(
        tree.skip_reason(node(&tree, "home/u/code/rs/target")),
        Some(SkipReason::Excluded)
    );
    assert_eq!(
        tree.kind(node(&tree, "home/u/code/js/node_modules")),
        NodeKind::Mount
    );

    let found: Vec<_> = matches(&tree, &env(), &RuleSet::builtin())
        .into_iter()
        .map(|rule| rule.id)
        .collect();
    assert!(
        !found
            .iter()
            .any(|id| id.starts_with("rust-target") || id.starts_with("node-modules")),
        "{found:?}"
    );
    assert!(planned_paths(&tree, &fs).is_empty());
}

#[test]
fn an_excluded_folder_is_not_suggested_and_the_others_still_are() {
    let fs = laptop();
    let control = scan_excluding(&fs, &[]);
    let tree = scan_excluding(&fs, &["/home/u/code/rs/target"]);

    let ids = |tree: &Tree| -> Vec<String> {
        evaluate(tree, &env(), &RuleSet::builtin())
            .suggestions
            .into_iter()
            .map(|suggestion| suggestion.id)
            .collect()
    };

    assert!(ids(&control).iter().any(|id| id.starts_with("rust-target")));
    assert!(!ids(&tree).iter().any(|id| id.starts_with("rust-target")));
    assert!(ids(&tree).iter().any(|id| id.starts_with("node-modules")));
}

#[test]
fn a_target_that_is_the_excluded_node_is_refused_by_the_guard() {
    let fs = laptop();
    let tree = scan_excluding(&fs, &["/home/u/old/stuff"]);
    let stuff = node(&tree, "home/u/old/stuff");

    let plan = plan_marked(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &[stuff],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert_eq!(plan.refused.len(), 1);
    assert_eq!(plan.refused[0].reason, Refusal::Mount);
}

#[test]
fn the_model_is_not_asked_about_an_excluded_folder() {
    let fs = laptop();
    let limits = CandidateLimits {
        min_bytes: 100,
        max: 50,
    };
    let candidates = |tree: &Tree| -> Vec<String> {
        label_candidates(tree, &env(), &RuleSet::builtin(), limits)
            .into_iter()
            .map(|node| String::from_utf8_lossy(&tree.path_bytes(node)).into_owned())
            .collect()
    };

    let control = scan_excluding(&fs, &[]);
    let tree = scan_excluding(&fs, &["/home/u/big"]);

    assert!(
        candidates(&control)
            .iter()
            .any(|path| path.starts_with("/home/u/big"))
    );
    assert!(
        !candidates(&tree)
            .iter()
            .any(|path| path.starts_with("/home/u/big"))
    );
}

#[test]
fn a_target_holding_an_excluded_folder_is_refused_and_a_sibling_target_is_not() {
    let mut fs = laptop();
    aged(&mut fs, "/home/u/code/rs/target/release/y", 400, 90);
    let tree = scan_excluding(&fs, &["/home/u/code/rs/target/debug"]);

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &selected(),
        &fs,
        &CleanupOptions::default(),
    );

    let planned: Vec<String> = plan.targets().map(|target| target.display_path()).collect();
    assert_eq!(planned, ["/home/u/code/js/node_modules"]);
    assert_eq!(plan.refused.len(), 1);
    assert_eq!(plan.refused[0].path, "/home/u/code/rs/target");
    assert_eq!(plan.refused[0].reason, Refusal::Excluded);
}

#[test]
fn a_marked_folder_holding_an_excluded_one_is_refused() {
    let mut fs = laptop();
    aged(&mut fs, "/home/u/old/other/b.bin", 50, 10);
    let tree = scan_excluding(&fs, &["/home/u/old/stuff"]);

    let plan = plan_marked(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &[node(&tree, "home/u/old")],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert_eq!(plan.refused[0].reason, Refusal::Excluded);
}
