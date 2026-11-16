//! How rules combine: one pass, outermost wins, nothing counted twice, and what is never suggested.

mod common;

use common::*;
use platter_core::dto::Verdict::{Keep, Review, Safe};
use platter_core::testing::{FakeFs, NOW};
use platter_core::{NodeId, Os};
use platter_rules::{Environment, Files, MAX_NODES, Matcher, Named, Rule, RuleSet, evaluate};

fn fixed(id: &'static str, verdict: platter_core::dto::Verdict, path: &'static str) -> Rule {
    Rule::new(id, id, verdict, Matcher::Fixed(path), "r")
}

fn home(id: &'static str, verdict: platter_core::dto::Verdict, path: &'static str) -> Rule {
    Rule::new(id, id, verdict, Matcher::Home(path), "r")
}

fn target_rule(id: &'static str, verdict: platter_core::dto::Verdict) -> Rule {
    Rule::new(
        id,
        id,
        verdict,
        Matcher::Named(Named::new(&["target"]).next_to("Cargo.toml")),
        "{n:project}",
    )
}

#[test]
fn the_outermost_match_wins_and_nothing_inside_it_is_matched_again() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/build/Cargo.toml", 5)
        .file("/home/u/.cache/build/target/obj", 1000)
        .file("/home/u/.cache/other", 20);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![target_rule("target", Safe), home("cache", Safe, ".cache")],
    );

    assert_eq!(ids(&found), ["cache"]);
    assert_eq!(get(&found, "cache").unwrap().allocated, 1025);
}

#[test]
fn a_directory_inside_a_match_of_the_same_rule_is_not_matched_again() {
    let mut fs = FakeFs::new();
    fs.file("/a/Cargo.toml", 1)
        .file("/a/target/x", 100)
        .file("/a/target/b/Cargo.toml", 1)
        .file("/a/target/b/target/y", 300);
    let tree = scan_at(&fs, "/", false);

    let found = run(&tree, vec![target_rule("target", Safe)]);

    let suggestion = get(&found, "target").unwrap();
    assert_eq!((suggestion.allocated, suggestion.items), (401, 1));
}

#[test]
fn no_byte_is_counted_twice_and_everything_unmatched_is_accounted_for() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/a", 10)
        .file("/home/u/.cache/proj/Cargo.toml", 1)
        .file("/home/u/.cache/proj/target/t", 20)
        .file("/home/u/Documents/d", 30)
        .file("/home/u/Documents/p/Cargo.toml", 2)
        .file("/home/u/Documents/p/target/t", 40)
        .file("/home/u/code/c/Cargo.toml", 4)
        .file("/home/u/code/c/target/t", 50)
        .file("/home/u/code/c/src/main.rs", 60)
        .file("/var/cache/pacman/pkg/pkg", 70)
        .file("/var/other", 80);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![
            fixed("pacman", Safe, "/var/cache/pacman/pkg"),
            home("cache", Safe, ".cache"),
            home("docs", Keep, "Documents"),
            target_rule("target", Review),
        ],
    );

    let covered: u64 = found.suggestions.iter().map(|s| s.allocated).sum();
    let unmatched = 4 + 60 + 80; // c/Cargo.toml, c/src/main.rs, /var/other
    assert_eq!(covered, (10 + 1 + 20) + (30 + 2 + 40) + 50 + 70);
    assert_eq!(covered + unmatched, tree.allocated(tree.root()));

    let totals = found.totals;
    assert_eq!((totals.safe, totals.review, totals.keep), (101, 50, 72));
    assert_eq!(totals.safe + totals.review + totals.keep, covered);
}

#[test]
fn the_nodes_of_suggestions_never_contain_one_another() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/a/Cargo.toml", 1)
        .file("/home/u/.cache/a/target/x", 5)
        .file("/home/u/p/Cargo.toml", 1)
        .file("/home/u/p/target/x", 5)
        .file("/home/u/p/target/inner/Cargo.toml", 1)
        .file("/home/u/p/target/inner/target/y", 5);
    let tree = scan_at(&fs, "/", false);
    let found = run(
        &tree,
        vec![target_rule("target", Safe), home("cache", Safe, ".cache")],
    );

    let nodes: Vec<NodeId> = found.suggestions.iter().flat_map(|s| s.nodes.clone()).collect();

    for &node in &nodes {
        let ancestors = tree.ancestry(node);
        let inside = nodes.iter().filter(|other| ancestors.contains(other)).count();
        assert_eq!(inside, 1, "only the node itself is on its own path");
    }
    assert_eq!(nodes.len(), 2);
}

#[test]
fn a_keep_match_stops_rules_from_matching_inside_it() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/Documents/p/Cargo.toml", 1)
        .file("/home/u/Documents/p/target/t", 900)
        .file("/home/u/Documents/p/node_modules/x", 5);
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![target_rule("target", Safe), home("docs", Keep, "Documents")],
    );

    assert_eq!(ids(&found), ["docs"]);
    assert_eq!(found.totals.keep, 906);
    assert_eq!(found.totals.safe, 0);
}

#[test]
fn unreadable_mount_and_empty_nodes_are_never_suggested() {
    let mut fs = FakeFs::new();
    fs.unreadable_dir("/var/lib/docker")
        .set_size("/var/lib/docker", 4096, 0);
    fs.mount("/mnt/usb").file("/mnt/usb/big", 5000);
    fs.dir("/home/u/.cache");
    fs.dir("/p/target").file("/p/Cargo.toml", 1);
    let tree = scan_at(&fs, "/", true);
    assert_eq!(tree.kind(find(&tree, "mnt/usb")), platter_core::NodeKind::Mount);
    assert!(tree.is_unreadable(find(&tree, "var/lib/docker")));

    let found = run(
        &tree,
        vec![
            fixed("docker", Review, "/var/lib/docker"),
            fixed("usb", Safe, "/mnt/usb"),
            home("cache", Safe, ".cache"),
            target_rule("target", Safe),
        ],
    );

    assert!(found.suggestions.is_empty(), "{found:?}");
}

#[test]
fn an_unreadable_folder_inside_a_match_does_not_stop_the_match() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/a", 100);
    fs.unreadable_dir("/home/u/.cache/locked");
    let tree = scan_at(&fs, "/", false);

    let found = run(&tree, vec![home("cache", Safe, ".cache")]);

    assert_eq!(get(&found, "cache").unwrap().allocated, 100);
}

#[test]
fn a_folder_that_fails_to_match_is_still_searched() {
    let mut fs = FakeFs::new();
    fs.file("/a/Cargo.toml", 1).file("/a/target/x", 100);
    fs.file("/a/sub/Cargo.toml", 1).file("/a/sub/target/y", 7);
    fs.dir("/home/u/.cache");
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![home("cache", Safe, ".cache"), target_rule("target", Safe)],
    );

    assert_eq!(
        get(&found, "target").unwrap().items,
        2,
        "an empty .cache does not hide the rest"
    );
}

#[test]
fn suggestions_are_ordered_safe_review_keep_and_largest_first_within_each() {
    let mut fs = FakeFs::new();
    for (name, size) in [
        ("safe_small", 10),
        ("safe_big", 100),
        ("review_small", 20),
        ("review_big", 200),
        ("keep_small", 30),
        ("keep_big", 300),
    ] {
        fs.file(&format!("/v/{name}/f"), size);
    }
    let tree = scan_at(&fs, "/", false);

    let found = run(
        &tree,
        vec![
            fixed("keep_small", Keep, "/v/keep_small"),
            fixed("review_small", Review, "/v/review_small"),
            fixed("safe_small", Safe, "/v/safe_small"),
            fixed("keep_big", Keep, "/v/keep_big"),
            fixed("review_big", Review, "/v/review_big"),
            fixed("safe_big", Safe, "/v/safe_big"),
        ],
    );

    assert_eq!(
        ids(&found),
        [
            "safe_big",
            "safe_small",
            "review_big",
            "review_small",
            "keep_big",
            "keep_small"
        ]
    );
}

#[test]
fn the_nodes_are_the_twenty_largest_matches_largest_first() {
    let mut fs = FakeFs::new();
    for project in 0..25u64 {
        fs.file(&format!("/p/{project:02}/Cargo.toml"), 1);
        fs.file(
            &format!("/p/{project:02}/target/x"),
            100 + (project % 5) * 10 + project,
        );
    }
    let tree = scan_at(&fs, "/", false);

    let found = run(&tree, vec![target_rule("target", Safe)]);

    let suggestion = get(&found, "target").unwrap();
    assert_eq!(suggestion.items, 25);
    assert_eq!(suggestion.nodes.len(), MAX_NODES);

    let sizes: Vec<u64> = suggestion.nodes.iter().map(|&n| tree.allocated(n)).collect();
    assert!(sizes.windows(2).all(|pair| pair[0] >= pair[1]), "{sizes:?}");

    let mut all: Vec<u64> = (0..25)
        .map(|p| tree.allocated(find(&tree, &format!("p/{p:02}/target"))))
        .collect();
    all.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(sizes, all[..MAX_NODES], "the 20 largest of all 25");
    assert_eq!(
        suggestion.allocated,
        all.iter().sum::<u64>(),
        "the sum covers all matches, not just listed ones"
    );
}

#[test]
fn equal_sizes_are_listed_by_ascending_node_id() {
    let mut fs = FakeFs::new();
    for project in ["c", "a", "b"] {
        fs.file(&format!("/p/{project}/Cargo.toml"), 1);
        fs.file(&format!("/p/{project}/target/x"), 50);
    }
    let tree = scan_at(&fs, "/", false);

    let found = run(&tree, vec![target_rule("target", Safe)]);

    let nodes = &get(&found, "target").unwrap().nodes;
    assert!(nodes.windows(2).all(|pair| pair[0] < pair[1]), "{nodes:?}");
}

#[test]
fn the_reason_carries_the_real_count_and_age() {
    let mut fs = FakeFs::new();
    for project in 0..14 {
        fs.file(&format!("/p/{project:02}/Cargo.toml"), 1);
        aged_file(&mut fs, &format!("/p/{project:02}/target/x"), 100, 90);
    }
    let tree = scan_at(&fs, "/", false);
    let rule = Rule::new(
        "t",
        "Rust build output",
        Safe,
        Matcher::Named(Named::new(&["target"]).next_to("Cargo.toml").idle(30)),
        "{n:project} not touched for over {days} days.",
    );

    let found = run(&tree, vec![rule]);
    assert_eq!(
        get(&found, "t").unwrap().reason,
        "14 projects not touched for over 30 days."
    );

    let mut one = FakeFs::new();
    one.file("/p/Cargo.toml", 1);
    aged_file(&mut one, "/p/target/x", 100, 90);
    let found = run(&scan_at(&one, "/", false), vec![rule]);
    assert_eq!(
        get(&found, "t").unwrap().reason,
        "1 project not touched for over 30 days."
    );
}

#[test]
fn when_two_rules_match_the_same_folder_the_earlier_one_wins() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/a", 100);
    let tree = scan_at(&fs, "/", false);

    let first = run(
        &tree,
        vec![
            home("first", Keep, ".cache"),
            fixed("second", Safe, "/home/u/.cache"),
        ],
    );
    let second = run(
        &tree,
        vec![
            fixed("second", Safe, "/home/u/.cache"),
            home("first", Keep, ".cache"),
        ],
    );

    assert_eq!(ids(&first), ["first"]);
    assert_eq!(ids(&second), ["second"]);
}

#[test]
fn a_name_rule_and_a_path_rule_on_the_same_folder_are_decided_by_table_order() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/p/Cargo.toml", 1).file("/home/u/p/target/x", 100);
    let tree = scan_at(&fs, "/", false);

    let named_first = run(
        &tree,
        vec![target_rule("named", Review), home("path", Keep, "p/target")],
    );
    let path_first = run(
        &tree,
        vec![home("path", Keep, "p/target"), target_rule("named", Review)],
    );

    assert_eq!(ids(&named_first), ["named"]);
    assert_eq!(ids(&path_first), ["path"]);
}

#[test]
fn a_rule_for_another_operating_system_is_skipped() {
    let mut fs = FakeFs::new();
    fs.file("/var/cache/pacman/pkg/a", 100);
    let tree = scan_at(&fs, "/", false);
    let rules = RuleSet::new(vec![
        fixed("pacman", Safe, "/var/cache/pacman/pkg").only_on(Os::Linux),
    ]);

    let on_linux = evaluate(&tree, &env(), &rules);
    let on_windows = evaluate(
        &tree,
        &Environment {
            os: Os::Windows,
            ..env()
        },
        &rules,
    );

    assert_eq!(ids(&on_linux), ["pacman"]);
    assert!(on_windows.suggestions.is_empty());
}

#[test]
fn the_age_is_measured_from_the_environments_clock() {
    let mut fs = FakeFs::new();
    fs.file("/p/Cargo.toml", 1);
    aged_file(&mut fs, "/p/target/x", 100, 10);
    let tree = scan_at(&fs, "/", false);
    let rules = RuleSet::new(vec![Rule::new(
        "t",
        "T",
        Safe,
        Matcher::Named(Named::new(&["target"]).next_to("Cargo.toml").idle(30)),
        "r",
    )]);

    let now = evaluate(&tree, &env(), &rules);
    let later = evaluate(
        &tree,
        &Environment {
            now: NOW + 25 * DAY,
            ..env()
        },
        &rules,
    );

    assert!(now.suggestions.is_empty());
    assert_eq!(ids(&later), ["t"]);
}

#[test]
fn overlapping_file_rules_give_each_file_to_the_earlier_rule() {
    let mut fs = FakeFs::new();
    aged_file(&mut fs, "/home/u/Downloads/vm.qcow2", 2 << 30, 500);
    aged_file(&mut fs, "/home/u/Downloads/old.zip", 100, 500);
    let tree = scan_at(&fs, "/", false);
    let rules = vec![
        Rule::new(
            "images",
            "Images",
            Review,
            Matcher::Files(
                Files::in_home("")
                    .of(platter_core::Category::DiskImage)
                    .at_least(1 << 30),
            ),
            "r",
        ),
        Rule::new(
            "old",
            "Old",
            Review,
            Matcher::Files(Files::in_home("Downloads").idle(365)),
            "r",
        ),
    ];

    let found = run(&tree, rules);

    assert_eq!(get(&found, "images").unwrap().allocated, 2 << 30);
    assert_eq!(get(&found, "old").unwrap().allocated, 100);
}

mod scan_roots {
    //! Home rules against a scan that starts at different places.

    use super::*;

    fn fs() -> FakeFs {
        let mut fs = FakeFs::new();
        fs.file("/home/u/.cache/a", 100)
            .file("/home/u/Projekty/p/Cargo.toml", 1)
            .file("/home/u/Projekty/p/target/x", 200)
            .file("/srv/q/Cargo.toml", 1)
            .file("/srv/q/target/y", 300);
        fs
    }

    fn rules() -> Vec<Rule> {
        vec![home("cache", Safe, ".cache"), target_rule("target", Safe)]
    }

    #[test]
    fn from_the_filesystem_root() {
        let found = run(&scan_at(&fs(), "/", false), rules());

        assert_eq!(get(&found, "cache").unwrap().path, "~/.cache");
        assert_eq!(get(&found, "target").unwrap().items, 2);
    }

    #[test]
    fn from_the_home_directory_itself() {
        let found = run(&scan_at(&fs(), "/home/u", false), rules());

        assert_eq!(get(&found, "cache").unwrap().allocated, 100);
        assert_eq!(get(&found, "target").unwrap().path, "~/Projekty/p/target");
    }

    #[test]
    fn from_a_folder_inside_home() {
        let found = run(&scan_at(&fs(), "/home/u/Projekty", false), rules());

        assert!(get(&found, "cache").is_none(), "~/.cache lies outside the scan");
        assert_eq!(get(&found, "target").unwrap().path, "~/Projekty/p/target");
    }

    #[test]
    fn from_a_folder_outside_home() {
        let found = run(&scan_at(&fs(), "/srv", false), rules());

        assert!(get(&found, "cache").is_none());
        assert_eq!(get(&found, "target").unwrap().path, "/srv/q/target");
    }

    #[test]
    fn from_the_matched_folder_itself() {
        let found = run(&scan_at(&fs(), "/home/u/.cache", false), rules());

        let cache = get(&found, "cache").expect("the scan root can be the match");
        assert_eq!((cache.allocated, cache.path.as_str()), (100, "~/.cache"));
        assert_eq!(cache.nodes, [NodeId::ROOT]);
    }

    #[test]
    fn without_a_known_home_directory() {
        let found = evaluate(
            &scan_at(&fs(), "/", false),
            &env_with_home(None),
            &RuleSet::new(rules()),
        );

        assert!(get(&found, "cache").is_none());
        assert_eq!(
            get(&found, "target").unwrap().path,
            "/**/target",
            "no home means no tilde"
        );
    }

    #[test]
    fn a_home_that_is_not_in_the_scan_matches_nothing() {
        let found = evaluate(
            &scan_at(&fs(), "/", false),
            &env_with_home(Some("/home/nobody")),
            &RuleSet::new(rules()),
        );

        assert!(get(&found, "cache").is_none());
    }
}
