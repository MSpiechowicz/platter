//! What the built-in rules must never call `safe`: personal folders under any name, a scan that starts
//! inside one, tools' own packages, and folders whose contents were only partly read. Each case comes
//! from the review before cleanup was built (docs/plan/NEXT.md, R1).

mod common;

use std::path::PathBuf;

use common::*;
use platter_core::UserDir;
use platter_core::dto::Verdict;
use platter_core::testing::FakeFs;
use platter_rules::{Environment, RuleSet, Suggestions, evaluate};

fn builtin(tree: &platter_core::Tree, env: &Environment) -> Suggestions {
    evaluate(tree, env, &RuleSet::builtin())
}

/// Every suggestion labelled `safe`, by id.
fn safe_ids(found: &Suggestions) -> Vec<&str> {
    found
        .suggestions
        .iter()
        .filter(|s| s.verdict == Verdict::Safe)
        .map(|s| s.id.as_str())
        .collect()
}

/// A Rust project and an npm project with lockfile, both idle for a year, below `dir`.
fn old_projects(fs: &mut FakeFs, dir: &str) {
    fs.file(&format!("{dir}/rs/Cargo.toml"), 1);
    aged_file(fs, &format!("{dir}/rs/target/x"), 100, 400);
    aged_file(fs, &format!("{dir}/js/package.json"), 1, 400);
    aged_file(fs, &format!("{dir}/js/package-lock.json"), 1, 400);
    aged_file(fs, &format!("{dir}/js/node_modules/dep/i.js"), 100, 400);
}

#[test]
fn projects_in_a_localized_documents_folder_are_kept() {
    let mut fs = FakeFs::new();
    old_projects(&mut fs, "/home/u/Dokumenty");
    let tree = scan_at(&fs, "/", false);
    let env = Environment {
        user_dirs: vec![(UserDir::Documents, PathBuf::from("/home/u/Dokumenty"))],
        ..env()
    };

    let found = builtin(&tree, &env);

    assert_eq!(ids(&found), ["documents"]);
    assert_eq!(get(&found, "documents").unwrap().path, "~/Dokumenty");
}

#[test]
fn every_personal_folder_is_kept_under_its_default_name() {
    for dir in ["Desktop", "Documents", "Music", "Pictures", "Videos"] {
        let mut fs = FakeFs::new();
        old_projects(&mut fs, &format!("/home/u/{dir}"));
        let tree = scan_at(&fs, "/", false);

        let found = builtin(&tree, &env());

        assert!(safe_ids(&found).is_empty(), "{dir}: {found:?}");
    }
}

#[test]
fn a_scan_that_starts_inside_a_kept_folder_suggests_nothing() {
    let mut fs = FakeFs::new();
    old_projects(&mut fs, "/home/u/Documents/work");
    let tree = scan_at(&fs, "/home/u/Documents/work", false);

    let found = builtin(&tree, &env());

    assert!(found.suggestions.is_empty(), "{found:?}");
}

#[test]
fn a_scan_of_a_kept_folder_itself_shows_it_as_kept() {
    let mut fs = FakeFs::new();
    old_projects(&mut fs, "/home/u/Documents");
    let tree = scan_at(&fs, "/home/u/Documents", false);

    let found = builtin(&tree, &env());

    assert_eq!(ids(&found), ["documents"]);
}

#[test]
fn without_a_known_home_build_folders_are_at_most_review() {
    let mut fs = FakeFs::new();
    old_projects(&mut fs, "/home/u/Documents");
    let tree = scan_at(&fs, "/", false);

    let found = builtin(&tree, &env_with_home(None));

    assert!(safe_ids(&found).is_empty(), "{found:?}");
    assert_eq!(get(&found, "rust-target").unwrap().verdict, Verdict::Review);
    assert_eq!(found.totals.safe, 0);
}

#[test]
fn node_modules_of_tools_are_not_suggested() {
    let mut fs = FakeFs::new();
    for package in [
        "/home/u/.vscode/extensions/pub.ext-1.0.0",
        "/home/u/.nvm/versions/node/v20/lib/node_modules/npm",
        "/usr/lib/node_modules/typescript",
        "/opt/app/resources",
    ] {
        aged_file(&mut fs, &format!("{package}/package.json"), 1, 400);
        aged_file(&mut fs, &format!("{package}/package-lock.json"), 1, 400);
        aged_file(&mut fs, &format!("{package}/node_modules/dep/i.js"), 100, 400);
    }
    let tree = scan_at(&fs, "/", false);

    let found = builtin(&tree, &env());

    assert!(get(&found, "node-modules").is_none(), "{found:?}");
    assert!(get(&found, "node-modules-recent").is_none(), "{found:?}");
}

#[test]
fn node_modules_without_a_lockfile_is_not_a_project() {
    let mut fs = FakeFs::new();
    aged_file(&mut fs, "/home/u/thing/package.json", 1, 400);
    aged_file(&mut fs, "/home/u/thing/node_modules/dep/i.js", 100, 400);
    let tree = scan_at(&fs, "/", false);

    let found = builtin(&tree, &env());

    assert!(found.suggestions.is_empty(), "{found:?}");
}

#[test]
fn a_project_with_a_lockfile_is_still_found() {
    for lockfile in ["package-lock.json", "pnpm-lock.yaml", "yarn.lock", "bun.lock"] {
        let mut fs = FakeFs::new();
        aged_file(&mut fs, "/home/u/code/web/package.json", 1, 400);
        aged_file(&mut fs, &format!("/home/u/code/web/{lockfile}"), 1, 400);
        aged_file(&mut fs, "/home/u/code/web/node_modules/dep/i.js", 100, 400);
        let tree = scan_at(&fs, "/", false);

        let found = builtin(&tree, &env());

        assert_eq!(safe_ids(&found), ["node-modules"], "{lockfile}");
    }
}

#[test]
fn a_marker_that_is_a_link_does_not_make_a_project() {
    let mut fs = FakeFs::new();
    fs.symlink("/home/u/p/Cargo.toml", 1);
    aged_file(&mut fs, "/home/u/p/target/x", 100, 400);
    let tree = scan_at(&fs, "/", false);

    let found = builtin(&tree, &env());

    assert!(found.suggestions.is_empty(), "{found:?}");
}

#[test]
fn a_build_folder_with_an_unreadable_part_is_not_safe() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/p/Cargo.toml", 1);
    aged_file(&mut fs, "/home/u/p/target/x", 100, 400);
    fs.unreadable_dir("/home/u/p/target/debug/build/locked");
    let tree = scan_at(&fs, "/", false);

    let found = builtin(&tree, &env());

    assert!(safe_ids(&found).is_empty(), "{found:?}");
}

#[test]
fn caches_that_can_hold_what_cannot_be_rebuilt_are_review() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/a", 10)
        .file("/home/u/.m2/repository/a", 10)
        .file("/var/log/journal/a", 10)
        .file("/var/cache/pacman/pkg/a", 10);
    let tree = scan_at(&fs, "/", false);

    let found = builtin(&tree, &env());

    for id in ["app-cache", "maven-repository", "journal", "pacman-cache"] {
        assert_eq!(get(&found, id).unwrap().verdict, Verdict::Review, "{id}");
    }
}
