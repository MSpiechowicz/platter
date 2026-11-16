//! The built-in rule table, on a fake machine that looks like a developer's laptop.

mod common;

use std::collections::HashSet;

use common::*;
use platter_core::dto::Verdict;
use platter_core::testing::FakeFs;
use platter_rules::RuleSet;

#[test]
fn rule_ids_and_titles_are_unique_and_reasons_leave_no_placeholder_behind() {
    let rules = RuleSet::builtin();
    let mut ids = HashSet::new();
    let mut titles = HashSet::new();

    for rule in rules.rules() {
        assert!(ids.insert(rule.id), "duplicate id {}", rule.id);
        assert!(titles.insert(rule.title), "duplicate title {}", rule.title);
        assert!(!rule.reason.is_empty() && !rule.title.is_empty());
    }
    assert!(rules.rules().len() >= 30);
}

fn laptop() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/var/cache/pacman/pkg/a.pkg.tar.zst", 5000)
        .file("/var/log/journal/abc/system.journal", 300)
        .file("/.snapshots/1/snapshot/f", 9000)
        .file("/home/u/.cache/mozilla/f", 4000)
        .file("/home/u/.local/share/Trash/files/x", 600)
        .file("/home/u/.local/share/Steam/steamapps/common/Game/g.pak", 20_000)
        .file("/home/u/.local/share/Steam/steamapps/shadercache/1/c", 700)
        .file("/home/u/.npm/_cacache/x", 800)
        .file("/home/u/Documents/thesis.odt", 100)
        .file("/home/u/VMs/win11.qcow2", 3 << 30);
    for project in ["a", "b"] {
        fs.file(&format!("/home/u/Projekty/{project}/Cargo.toml"), 1);
        aged_file(
            &mut fs,
            &format!("/home/u/Projekty/{project}/target/debug/x"),
            1000,
            90,
        );
    }
    fs.file("/home/u/Projekty/c/Cargo.toml", 1);
    aged_file(&mut fs, "/home/u/Projekty/c/target/debug/x", 2000, 1);
    aged_file(&mut fs, "/home/u/Projekty/web/package.json", 1, 100);
    aged_file(&mut fs, "/home/u/Projekty/web/node_modules/dep/i.js", 1500, 100);
    aged_file(&mut fs, "/home/u/Downloads/iso/old.iso", 2500, 800);
    aged_file(&mut fs, "/home/u/Downloads/new.zip", 99, 3);
    fs
}

#[test]
fn a_developers_laptop_gets_the_expected_suggestions() {
    let tree = scan_at(&laptop(), "/", false);

    let found = platter_rules::evaluate(&tree, &env(), &RuleSet::builtin());

    let summary: Vec<_> = found
        .suggestions
        .iter()
        .map(|s| (s.id.as_str(), s.verdict, s.allocated, s.items))
        .collect();
    assert_eq!(
        summary,
        [
            ("pacman-cache", Verdict::Safe, 5000, 1),
            ("app-cache", Verdict::Safe, 4000, 1),
            ("rust-target", Verdict::Safe, 2000, 2),
            ("node-modules", Verdict::Safe, 1500, 1),
            ("npm-cache", Verdict::Safe, 800, 1),
            ("steam-shader-cache", Verdict::Safe, 700, 1),
            ("trash", Verdict::Safe, 600, 1),
            ("journal", Verdict::Safe, 300, 1),
            ("disk-images", Verdict::Review, 3 << 30, 1),
            ("btrfs-snapshots", Verdict::Review, 9000, 1),
            ("old-downloads", Verdict::Review, 2500, 1),
            ("rust-target-recent", Verdict::Review, 2000, 1),
            ("steam-library", Verdict::Keep, 20_000, 1),
            ("documents", Verdict::Keep, 100, 1),
        ]
    );
    assert_eq!(
        found.totals.safe,
        5000 + 4000 + 2000 + 1500 + 800 + 700 + 600 + 300
    );
    assert_eq!(found.totals.keep, 20_100);
}

#[test]
fn the_texts_name_what_was_found_and_never_the_real_home() {
    let tree = scan_at(&laptop(), "/", false);

    let found = platter_rules::evaluate(&tree, &env(), &RuleSet::builtin());

    let target = get(&found, "rust-target").unwrap();
    assert_eq!(target.path, "~/Projekty/**/target");
    assert_eq!(
        target.reason,
        "Build output that cargo creates again on the next build. 2 projects with nothing modified in over 30 days."
    );

    let node_modules = get(&found, "node-modules").unwrap();
    assert_eq!(node_modules.path, "~/Projekty/web/node_modules");
    assert_eq!(
        node_modules.reason,
        "Dependencies that a package manager installs again from package.json. 1 project with nothing modified in over 60 days."
    );

    let downloads = get(&found, "old-downloads").unwrap();
    assert!(
        downloads
            .reason
            .starts_with("1 file in Downloads not modified for 365 days or more.")
    );

    let recent = get(&found, "rust-target-recent").unwrap();
    assert!(
        recent.reason.starts_with("1 project touched in the last 30 days"),
        "{}",
        recent.reason
    );

    let images = get(&found, "disk-images").unwrap();
    assert!(
        images.reason.starts_with("1 image of 1 GiB or more"),
        "{}",
        images.reason
    );
    assert_eq!(images.path, "~/VMs/win11.qcow2");

    for suggestion in &found.suggestions {
        assert!(!suggestion.path.contains(HOME), "{suggestion:?}");
        assert!(!suggestion.reason.contains(['{', '}']), "{suggestion:?}");
    }
}

#[test]
fn a_machine_with_none_of_it_gets_no_suggestions() {
    let mut fs = FakeFs::new();
    fs.file("/etc/hostname", 10).file("/home/u/notes.txt", 20);
    let tree = scan_at(&fs, "/", false);

    let found = platter_rules::evaluate(&tree, &env(), &RuleSet::builtin());

    assert!(found.suggestions.is_empty());
    assert_eq!(
        (found.totals.safe, found.totals.review, found.totals.keep),
        (0, 0, 0)
    );
}

#[test]
fn the_wire_form_is_what_the_contract_says() {
    let mut fs = FakeFs::new();
    fs.file("/var/cache/pacman/pkg/a", 5000);
    let tree = scan_at(&fs, "/", false);

    let found = platter_rules::evaluate(&tree, &env(), &RuleSet::builtin());

    let wire = serde_json::to_value(&found).unwrap();
    assert_eq!(
        wire["totals"],
        serde_json::json!({ "safe": 5000, "review": 0, "keep": 0 })
    );
    let first = &wire["suggestions"][0];
    assert_eq!(first["id"], "pacman-cache");
    assert_eq!(first["verdict"], "safe");
    assert_eq!(
        first["nodes"],
        serde_json::json!([find(&tree, "var/cache/pacman/pkg").0])
    );
}

#[test]
fn the_app_cache_reason_does_not_promise_that_everything_in_it_is_rebuilt() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/huggingface/model.bin", 9000);
    let tree = scan_at(&fs, "/", false);

    let found = platter_rules::evaluate(&tree, &env(), &RuleSet::builtin());

    let cache = get(&found, "app-cache").unwrap();
    assert_eq!(cache.verdict, Verdict::Safe);
    assert_eq!(
        cache.reason,
        "Rebuilt by the apps that use it. Large downloads kept here, such as models, would have to be downloaded again."
    );
}

#[test]
fn hardlinked_files_inside_a_suggestion_are_counted_whichever_link_owns_the_bytes() {
    let mut fs = FakeFs::new();
    // One link of each pair is charged the bytes, the one the scan meets first; the Trash holds the
    // other one or not, and the count must not depend on which.
    fs.file("/home/u/.local/share/Trash/files/shared-a", 4000)
        .hardlink(
            "/home/u/.local/share/Trash/files/shared-a",
            "/home/u/Downloads/copy-of-a",
        )
        .file("/home/u/.local/share/Trash/files/shared-b", 3000)
        .hardlink(
            "/home/u/.local/share/Trash/files/shared-b",
            "/home/u/Downloads/copy-of-b",
        )
        .file("/home/u/.local/share/Trash/files/plain", 500)
        .file("/home/u/.local/share/Trash/files/sub/deep-shared", 700)
        .hardlink(
            "/home/u/.local/share/Trash/files/sub/deep-shared",
            "/var/lib/other/deep-link",
        );
    let tree = scan_at(&fs, "/", false);

    let found = platter_rules::evaluate(&tree, &env(), &RuleSet::builtin());

    let trash = get(&found, "trash").unwrap();
    assert_eq!(trash.shared_files, 3, "two at the top and one in a sub folder");
    assert_eq!(trash.items, 1);

    let wire = serde_json::to_value(trash).unwrap();
    assert_eq!(wire["sharedFiles"], 3);
}

#[test]
fn a_suggestion_without_hardlinks_says_zero_and_a_scan_without_any_costs_nothing() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.local/share/Trash/files/x", 600)
        .file("/home/u/Documents/a", 100);
    let tree = scan_at(&fs, "/", false);

    let found = platter_rules::evaluate(&tree, &env(), &RuleSet::builtin());

    assert!(found.suggestions.iter().all(|s| s.shared_files == 0));
    let wire = serde_json::to_value(get(&found, "trash").unwrap()).unwrap();
    assert_eq!(wire["sharedFiles"], 0);
}
