//! Where a folder stands among the matches: covered by one, holding some, or beside others.

mod common;

use common::*;
use platter_core::dto::Verdict;
use platter_core::testing::FakeFs;
use platter_rules::{Explainer, RuleSet};

/// A cache, kept documents, an idle Rust project, an old download and two folders no rule knows.
fn home_with_everything() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/app/blob", 300);
    fs.file("/home/u/Documents/thesis/draft", 10);
    fs.file("/home/u/Documents/thesis/figures/f1", 10);
    fs.file("/home/u/code/rs/Cargo.toml", 1);
    aged_file(&mut fs, "/home/u/code/rs/target/x", 1000, 90);
    aged_file(&mut fs, "/home/u/Downloads/old.iso", 200, 800);
    fs.file("/home/u/stuff/big/blob", 5000);
    fs.file("/home/u/stuff/other/blob", 400);
    fs
}

#[test]
fn a_folder_is_covered_by_the_match_at_it_or_above_it() {
    let tree = scan_at(&home_with_everything(), "/", false);
    let explainer = Explainer::new(&tree, &env(), &RuleSet::builtin());

    let inside = explainer.covering(find(&tree, "home/u/.cache/app")).unwrap();
    assert_eq!((inside.id, inside.verdict), ("app-cache", Verdict::Review));
    assert_eq!(inside.title, "Application caches");
    assert_eq!(inside.at, find(&tree, "home/u/.cache"));
    assert!(!inside.reason.is_empty());

    let itself = explainer.covering(find(&tree, "home/u/Documents")).unwrap();
    assert_eq!((itself.id, itself.verdict), ("documents", Verdict::Keep));
    assert_eq!(itself.at, find(&tree, "home/u/Documents"));

    let project = explainer
        .covering(find(&tree, "home/u/code/rs/target/x"))
        .unwrap();
    assert_eq!(project.id, "rust-target");
    assert!(project.reason.contains("1 project"), "{}", project.reason);
}

#[test]
fn a_folder_no_rule_reaches_has_no_covering_rule() {
    let tree = scan_at(&home_with_everything(), "/", false);
    let explainer = Explainer::new(&tree, &env(), &RuleSet::builtin());

    for path in [
        "home/u/stuff",
        "home/u/stuff/big",
        "home/u/code",
        "home/u",
        "home",
    ] {
        assert_eq!(explainer.covering(find(&tree, path)), None, "{path}");
    }
}

#[test]
fn inside_lists_the_rules_matched_below_with_their_bytes_largest_first() {
    let tree = scan_at(&home_with_everything(), "/", false);
    let explainer = Explainer::new(&tree, &env(), &RuleSet::builtin());

    assert_eq!(
        explainer.inside(find(&tree, "home/u")),
        [
            ("rust-target", 1000),
            ("app-cache", 300),
            ("old-downloads", 200),
            ("documents", 20)
        ]
    );
    assert_eq!(
        explainer.inside(find(&tree, "home/u/code")),
        [("rust-target", 1000)]
    );
    assert_eq!(
        explainer.inside(find(&tree, "home/u/Downloads")),
        [("old-downloads", 200)],
        "a file rule matches the files, so the folder holds them"
    );
}

#[test]
fn inside_is_empty_for_a_match_itself_and_for_what_holds_none() {
    let tree = scan_at(&home_with_everything(), "/", false);
    let explainer = Explainer::new(&tree, &env(), &RuleSet::builtin());

    for path in [
        "home/u/.cache",
        "home/u/.cache/app",
        "home/u/Documents",
        "home/u/stuff",
        "home/u/stuff/big",
    ] {
        assert!(explainer.inside(find(&tree, path)).is_empty(), "{path}");
    }
}

#[test]
fn beside_lists_the_rules_matched_on_the_siblings_only() {
    let tree = scan_at(&home_with_everything(), "/", false);
    let explainer = Explainer::new(&tree, &env(), &RuleSet::builtin());

    assert_eq!(
        explainer.beside(find(&tree, "home/u/stuff")),
        ["app-cache", "documents"]
    );
    assert_eq!(
        explainer.beside(find(&tree, "home/u/.cache")),
        ["documents"],
        "not itself"
    );
    assert!(
        explainer.beside(find(&tree, "home/u/stuff/big")).is_empty(),
        "stuff/other is not matched"
    );
    assert!(
        explainer.beside(tree.root()).is_empty(),
        "the root has no siblings"
    );
}

#[test]
fn a_scan_inside_a_kept_folder_has_no_matches() {
    let tree = scan_at(&home_with_everything(), "/home/u/Documents/thesis", false);
    let explainer = Explainer::new(&tree, &env(), &RuleSet::builtin());

    assert_eq!(explainer.covering(find(&tree, "figures")), None);
    assert!(explainer.inside(tree.root()).is_empty());
    assert!(!explainer.touches(find(&tree, "figures")));
}

#[test]
fn touches_is_true_for_covered_folders_and_folders_holding_a_match() {
    let tree = scan_at(&home_with_everything(), "/", false);
    let explainer = Explainer::new(&tree, &env(), &RuleSet::builtin());

    for path in ["home/u/.cache/app", "home/u/code", "home/u"] {
        assert!(explainer.touches(find(&tree, path)), "{path}");
    }
    for path in ["home/u/stuff", "home/u/stuff/big"] {
        assert!(!explainer.touches(find(&tree, path)), "{path}");
    }
}
