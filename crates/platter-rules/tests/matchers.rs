//! Each matcher on its own, through the public API and a fake filesystem.

mod common;

use common::*;
use platter_core::dto::Verdict::{Review, Safe};
use platter_core::testing::{FakeFs, NOW};
use platter_core::{Category, UserDir};
use platter_rules::{Age, Files, Matcher, Named, Rule};

fn named_rule(id: &'static str, named: Named) -> Rule {
    Rule::new(id, "Named", Safe, Matcher::Named(named), "{n:folder}")
}

#[test]
fn a_fixed_rule_matches_the_directory_at_that_absolute_path() {
    let mut fs = FakeFs::new();
    fs.file("/var/cache/pacman/pkg/a.zst", 300)
        .file("/var/cache/pacman/pkg/b.zst", 200)
        .file("/var/cache/other/c", 50);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![Rule::new(
            "pacman",
            "Pacman",
            Safe,
            Matcher::Fixed("/var/cache/pacman/pkg"),
            "r",
        )],
    );

    let suggestion = get(&found, "pacman").expect("matched");
    assert_eq!(suggestion.path, "/var/cache/pacman/pkg");
    assert_eq!((suggestion.allocated, suggestion.items), (500, 1));
    assert_eq!(suggestion.nodes, [find(&tree, "var/cache/pacman/pkg")]);
}

#[test]
fn a_rule_whose_prerequisite_is_absent_produces_nothing() {
    let mut fs = FakeFs::new();
    fs.file("/var/cache/other/c", 50);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![
            Rule::new("a", "A", Safe, Matcher::Fixed("/var/cache/pacman/pkg"), "r"),
            Rule::new("b", "B", Safe, Matcher::Home(".cache"), "r"),
        ],
    );

    assert!(found.suggestions.is_empty());
}

#[test]
fn a_fixed_or_home_rule_only_matches_a_directory() {
    let mut fs = FakeFs::new();
    fs.file("/var/cache/pacman/pkg", 300)
        .symlink("/home/u/.cache", 20)
        .file("/home/u/.npm", 100);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![
            Rule::new("file", "F", Safe, Matcher::Fixed("/var/cache/pacman/pkg"), "r"),
            Rule::new("link", "L", Safe, Matcher::Home(".cache"), "r"),
            Rule::new("npm", "N", Safe, Matcher::Home(".npm"), "r"),
        ],
    );

    assert!(found.suggestions.is_empty());
}

#[test]
fn a_home_rule_is_shown_with_a_tilde_and_never_with_the_real_home() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.local/share/Trash/files/x", 700);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![Rule::new(
            "trash",
            "Trash",
            Safe,
            Matcher::Home(".local/share/Trash"),
            "Deleted files in {n:place}.",
        )],
    );

    let suggestion = get(&found, "trash").expect("matched");
    assert_eq!(suggestion.path, "~/.local/share/Trash");
    assert_eq!(suggestion.allocated, 700);
    assert!(!format!("{found:?}").contains(HOME), "{found:?}");
}

#[test]
fn a_name_rule_needs_its_marker_file_next_to_the_directory() {
    let mut fs = FakeFs::new();
    fs.file("/code/with/Cargo.toml", 1)
        .file("/code/with/target/a", 100);
    fs.file("/code/without/target/a", 200);
    fs.dir("/code/marker_is_a_folder/Cargo.toml")
        .file("/code/marker_is_a_folder/target/a", 400);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![named_rule("t", Named::new(&["target"]).next_to("Cargo.toml"))],
    );

    let suggestion = get(&found, "t").expect("matched");
    assert_eq!((suggestion.allocated, suggestion.items), (100, 1));
    assert_eq!(suggestion.nodes, [find(&tree, "code/with/target")]);
}

#[test]
fn a_name_rule_without_a_marker_matches_anywhere_but_only_directories() {
    let mut fs = FakeFs::new();
    fs.file("/a/__pycache__/x.pyc", 10)
        .file("/b/c/__pycache__/y.pyc", 20)
        .file("/d/__pycache__", 40);
    let tree = scan_at(&fs, "/", false);

    let found = run(&tree, vec![named_rule("py", Named::new(&["__pycache__"]))]);

    let suggestion = get(&found, "py").expect("matched");
    assert_eq!((suggestion.allocated, suggestion.items), (30, 2));
}

#[test]
fn the_age_condition_has_a_sharp_boundary_and_the_two_ages_are_opposites() {
    let mut fs = FakeFs::new();
    for project in ["exactly", "just_under", "future"] {
        fs.file(&format!("/p/{project}/Cargo.toml"), 1);
    }
    aged_file(&mut fs, "/p/exactly/target/f", 100, 30);
    fs.file("/p/just_under/target/f", 200)
        .set_mtime("/p/just_under/target/f", NOW - 30 * DAY + 1);
    fs.file("/p/future/target/f", 400)
        .set_mtime("/p/future/target/f", NOW + 10 * DAY);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![
            named_rule("idle", Named::new(&["target"]).next_to("Cargo.toml").idle(30)),
            named_rule("recent", Named::new(&["target"]).next_to("Cargo.toml").recent(30)),
        ],
    );

    let idle = get(&found, "idle").expect("idle");
    let recent = get(&found, "recent").expect("recent");
    assert_eq!((idle.allocated, idle.items), (100, 1), "30 days exactly is idle");
    assert_eq!(
        (recent.allocated, recent.items),
        (600, 2),
        "a second younger, or in the future, is recent"
    );
    assert!(Age::Idle(30).holds(NOW - 30 * DAY, NOW));
    assert!(!Age::Idle(30).holds(NOW - 30 * DAY + 1, NOW));
    assert!(Age::Recent(30).holds(NOW - 30 * DAY + 1, NOW));
}

#[test]
fn a_directory_is_as_young_as_the_newest_file_anywhere_inside() {
    let mut fs = FakeFs::new();
    fs.file("/p/Cargo.toml", 1);
    aged_file(&mut fs, "/p/target/old", 100, 400);
    aged_file(&mut fs, "/p/target/debug/deps/fresh", 100, 1);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![named_rule(
            "idle",
            Named::new(&["target"]).next_to("Cargo.toml").idle(30),
        )],
    );

    assert!(found.suggestions.is_empty());
}

#[test]
fn a_name_rule_with_several_names_shows_the_names_that_matched() {
    let mut fs = FakeFs::new();
    fs.file("/a/__pycache__/x", 10)
        .file("/a/.mypy_cache/y", 10)
        .file("/b/__pycache__/z", 10);
    let tree = scan_at(&fs, "/", false);
    let all = Named::new(&["__pycache__", ".pytest_cache", ".mypy_cache"]);

    let found = run(&tree, vec![named_rule("py", all)]);

    assert_eq!(get(&found, "py").unwrap().path, "/**/{__pycache__,.mypy_cache}");
}

#[test]
fn the_display_pattern_sits_below_the_deepest_folder_holding_every_match() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/Projekty/a/Cargo.toml", 1)
        .file("/home/u/Projekty/a/target/x", 10)
        .file("/home/u/Projekty/deep/b/Cargo.toml", 1)
        .file("/home/u/Projekty/deep/b/target/x", 10);
    fs.file("/srv/c/Cargo.toml", 1).file("/srv/c/target/x", 10);
    let tree = scan_at(&fs, "/", false);
    let rule = named_rule("t", Named::new(&["target"]).next_to("Cargo.toml"));

    let under_home = run(&scan_at(&fs, "/home/u", false), vec![rule]);
    let whole_disk = run(&tree, vec![rule]);

    assert_eq!(get(&under_home, "t").unwrap().path, "~/Projekty/**/target");
    assert_eq!(
        get(&whole_disk, "t").unwrap().path,
        "/**/target",
        "matches in two trees share only /"
    );

    let mut one = FakeFs::new();
    one.file("/home/u/Projekty/a/Cargo.toml", 1)
        .file("/home/u/Projekty/a/target/x", 10);
    let one = run(&scan_at(&one, "/", false), vec![rule]);
    assert_eq!(
        get(&one, "t").unwrap().path,
        "~/Projekty/a/target",
        "one match shows its own path"
    );
}

#[test]
fn a_file_rule_covers_only_the_files_that_qualify() {
    let mut fs = FakeFs::new();
    aged_file(&mut fs, "/home/u/Downloads/old.iso", 1000, 400);
    aged_file(&mut fs, "/home/u/Downloads/nested/deeper/old.zip", 500, 800);
    aged_file(&mut fs, "/home/u/Downloads/new.zip", 7000, 10);
    aged_file(&mut fs, "/home/u/Documents/old.pdf", 9000, 900);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![Rule::new(
            "dl",
            "Old downloads",
            Review,
            Matcher::Files(Files::in_home("Downloads").idle(365)),
            "{n:file} older than {days} days.",
        )],
    );

    let suggestion = get(&found, "dl").expect("matched");
    assert_eq!((suggestion.allocated, suggestion.items), (1500, 2));
    assert_eq!(suggestion.path, "~/Downloads/**");
    assert_eq!(suggestion.reason, "2 files older than 365 days.");
    assert_eq!(suggestion.nodes.len(), 2);
}

#[test]
fn a_file_rule_with_a_single_match_shows_that_file() {
    let mut fs = FakeFs::new();
    aged_file(&mut fs, "/home/u/Downloads/old.iso", 1000, 400);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![Rule::new(
            "dl",
            "Old downloads",
            Review,
            Matcher::Files(Files::in_home("Downloads").idle(365)),
            "r",
        )],
    );

    assert_eq!(get(&found, "dl").unwrap().path, "~/Downloads/old.iso");
}

#[test]
fn a_file_rule_can_select_by_category_and_size() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/VMs/big.qcow2", 3 << 30)
        .file("/home/u/VMs/small.qcow2", 100 << 20)
        .file("/home/u/VMs/notes.txt", 5 << 30)
        .dir("/home/u/VMs/folder.qcow2");
    let tree = scan_at(&fs, "/", false);
    assert_eq!(
        tree.category(find(&tree, "home/u/VMs/big.qcow2")),
        Category::DiskImage
    );

    let found = run(
        &tree,
        vec![Rule::new(
            "img",
            "Images",
            Review,
            Matcher::Files(Files::in_home("").of(Category::DiskImage).at_least(1 << 30)),
            "{n:image} of {size} or more.",
        )],
    );

    let suggestion = get(&found, "img").expect("matched");
    assert_eq!((suggestion.allocated, suggestion.items), (3 << 30, 1));
    assert_eq!(suggestion.reason, "1 image of 1.0 GiB or more.");
}

#[test]
fn files_inside_a_folder_another_rule_matched_are_left_to_that_rule() {
    let mut fs = FakeFs::new();
    aged_file(&mut fs, "/home/u/Downloads/old.zip", 100, 500);
    fs.file("/home/u/Downloads/proj/package.json", 1);
    aged_file(
        &mut fs,
        "/home/u/Downloads/proj/node_modules/dep/index.js",
        4000,
        500,
    );
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![
            Rule::new(
                "dl",
                "Old downloads",
                Review,
                Matcher::Files(Files::in_home("Downloads").idle(365)),
                "r",
            ),
            named_rule("nm", Named::new(&["node_modules"]).next_to("package.json")),
        ],
    );

    assert_eq!(get(&found, "nm").unwrap().allocated, 4000);
    let downloads = get(&found, "dl").unwrap();
    assert_eq!(
        (downloads.allocated, downloads.items),
        (101, 2),
        "the old zip and package.json; not node_modules files"
    );
}

#[test]
fn a_file_rule_in_a_user_folder_looks_where_the_desktop_put_it() {
    let mut fs = FakeFs::new();
    aged_file(&mut fs, "/home/u/Pobrane/old.iso", 1000, 400);
    aged_file(&mut fs, "/home/u/Pobrane/new.zip", 7000, 10);
    aged_file(&mut fs, "/home/u/Downloads/elsewhere.iso", 500, 400);
    let tree = scan_at(&fs, "/", false);
    let env = platter_rules::Environment {
        user_dirs: vec![(UserDir::Downloads, std::path::PathBuf::from("/home/u/Pobrane"))],
        ..env()
    };
    let rules = platter_rules::RuleSet::new(vec![Rule::new(
        "dl",
        "Old downloads",
        Review,
        Matcher::Files(Files::in_user_dir(UserDir::Downloads).idle(365)),
        "r",
    )]);

    let found = platter_rules::evaluate(&tree, &env, &rules);

    let suggestion = get(&found, "dl").expect("matched");
    assert_eq!((suggestion.allocated, suggestion.items), (1000, 1));
    assert_eq!(suggestion.path, "~/Pobrane/old.iso");
}

#[test]
fn a_file_rule_in_a_user_folder_uses_the_default_name_when_none_is_configured() {
    let mut fs = FakeFs::new();
    aged_file(&mut fs, "/home/u/Downloads/old.iso", 1000, 400);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![Rule::new(
            "dl",
            "Old downloads",
            Review,
            Matcher::Files(Files::in_user_dir(UserDir::Downloads).idle(365)),
            "r",
        )],
    );

    assert_eq!(get(&found, "dl").unwrap().path, "~/Downloads/old.iso");
}

#[test]
fn the_old_downloads_rule_matches_nothing_when_the_environment_has_no_downloads_folder() {
    let mut fs = FakeFs::new();
    aged_file(&mut fs, "/home/u/Downloads/old.iso", 1000, 400);
    let tree = scan_at(&fs, "/", false);
    let mut env = env();
    env.set_downloads_missing();

    let found = platter_rules::evaluate(&tree, &env, &platter_rules::RuleSet::builtin());

    assert!(get(&found, "old-downloads").is_none());
}

#[test]
fn the_old_downloads_rule_follows_the_configured_downloads_folder_and_leaves_a_namesake_alone() {
    let mut fs = FakeFs::new();
    aged_file(&mut fs, "/home/u/Pobrane/old.iso", 1000, 400);
    aged_file(&mut fs, "/home/u/Downloads/namesake.iso", 500, 400);
    let tree = scan_at(&fs, "/", false);
    let env = platter_rules::Environment {
        user_dirs: vec![(UserDir::Downloads, std::path::PathBuf::from("/home/u/Pobrane"))],
        ..env()
    };

    let found = platter_rules::evaluate(&tree, &env, &platter_rules::RuleSet::builtin());

    let old = get(&found, "old-downloads").expect("matched");
    assert_eq!((old.allocated, old.items), (1000, 1));
    assert_eq!(old.path, "~/Pobrane/old.iso");
}
