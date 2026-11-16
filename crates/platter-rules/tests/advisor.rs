//! The model's labels as one more suggestion: only `review`, never where a rule already has an opinion.

mod common;

use common::*;
use platter_core::dto::{SuggestionSource, Verdict};
use platter_core::testing::FakeFs;
use platter_core::{ModelLabel, ModelVerdict, NodeId, Tree};
use platter_rules::{RuleSet, Suggestions, evaluate, evaluate_with};

fn fs() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/app/blob", 300);
    fs.file("/home/u/Documents/thesis/draft", 10);
    fs.file("/home/u/Documents/thesis/figures/f1", 10);
    fs.file("/home/u/stuff/big/blob", 5000);
    fs.file("/home/u/stuff/big/inner/blob", 100);
    fs.file("/home/u/stuff/other/blob", 400);
    fs.file("/home/u/stuff/loose", 7);
    fs.unreadable_dir("/home/u/stuff/locked");
    fs
}

fn label(tree: &Tree, path: &str, verdict: ModelVerdict) -> ModelLabel {
    ModelLabel {
        node: find(tree, path),
        verdict,
        reason: format!("because of {path}"),
    }
}

fn review(tree: &Tree, path: &str) -> ModelLabel {
    label(tree, path, ModelVerdict::Review)
}

fn with(tree: &Tree, labels: &[ModelLabel]) -> Suggestions {
    evaluate_with(tree, &env(), &RuleSet::builtin(), labels)
}

#[test]
fn without_labels_the_result_is_exactly_evaluate() {
    let tree = scan_at(&fs(), "/", false);

    let plain = evaluate(&tree, &env(), &RuleSet::builtin());

    assert_eq!(with(&tree, &[]), plain);
    assert!(
        plain
            .suggestions
            .iter()
            .all(|s| s.source == SuggestionSource::Rule)
    );
}

#[test]
fn review_labels_become_one_model_suggestion_after_the_rules_of_the_same_verdict() {
    let tree = scan_at(&fs(), "/", false);
    let labels = [
        review(&tree, "home/u/stuff/big"),
        review(&tree, "home/u/stuff/other"),
    ];

    let found = with(&tree, &labels);

    let advisor = get(&found, "advisor").unwrap();
    assert_eq!(advisor.title, "Labelled by the local model");
    assert_eq!(advisor.source, SuggestionSource::Model);
    assert_eq!(advisor.verdict, Verdict::Review);
    assert!(!advisor.reason.is_empty());
    assert_eq!((advisor.allocated, advisor.items), (5500, 2));
    assert_eq!(
        advisor.nodes,
        [find(&tree, "home/u/stuff/big"), find(&tree, "home/u/stuff/other")]
    );
    assert_eq!(advisor.path, "~/stuff/**");
    assert_eq!(found.totals.review, 300 + 5500, "the app cache is a review rule");
    assert_eq!(
        ids(&found),
        ["advisor", "app-cache", "documents"],
        "largest review first"
    );
    assert_eq!(get(&found, "app-cache").unwrap().source, SuggestionSource::Rule);
}

#[test]
fn a_single_label_shows_its_own_path() {
    let tree = scan_at(&fs(), "/", false);

    let found = with(&tree, &[review(&tree, "home/u/stuff/other")]);

    assert_eq!(get(&found, "advisor").unwrap().path, "~/stuff/other");
}

#[test]
fn keep_labels_are_never_listed() {
    let tree = scan_at(&fs(), "/", false);
    let labels = [
        label(&tree, "home/u/stuff/big", ModelVerdict::Keep),
        review(&tree, "home/u/stuff/other"),
    ];

    let found = with(&tree, &labels);

    let advisor = get(&found, "advisor").unwrap();
    assert_eq!((advisor.allocated, advisor.items), (400, 1));

    let only_keep = with(&tree, &labels[..1]);
    assert!(get(&only_keep, "advisor").is_none());
    assert_eq!(only_keep, evaluate(&tree, &env(), &RuleSet::builtin()));
}

#[test]
fn a_label_a_rule_already_covers_holds_or_sits_in_is_left_out() {
    let tree = scan_at(&fs(), "/", false);
    let labels = [
        review(&tree, "home/u/.cache"),
        review(&tree, "home/u/.cache/app"),
        review(&tree, "home/u/Documents"),
        review(&tree, "home/u/Documents/thesis"),
        review(&tree, "home/u"),
        review(&tree, "home"),
    ];

    let found = with(&tree, &labels);

    assert!(get(&found, "advisor").is_none(), "{found:#?}");
    assert_eq!(found, evaluate(&tree, &env(), &RuleSet::builtin()));
}

#[test]
fn a_label_inside_another_listed_label_is_left_to_the_outer_one() {
    let tree = scan_at(&fs(), "/", false);
    let labels = [
        review(&tree, "home/u/stuff/big/inner"),
        review(&tree, "home/u/stuff/big"),
        review(&tree, "home/u/stuff/big"),
    ];

    let found = with(&tree, &labels);

    let advisor = get(&found, "advisor").unwrap();
    assert_eq!((advisor.allocated, advisor.items), (5100, 1));
}

#[test]
fn the_scan_root_files_unreadable_and_unknown_nodes_are_not_listed() {
    let tree = scan_at(&fs(), "/home/u/stuff", false);
    let labels = [
        ModelLabel {
            node: tree.root(),
            verdict: ModelVerdict::Review,
            reason: "x".into(),
        },
        review(&tree, "loose"),
        review(&tree, "locked"),
        ModelLabel {
            node: NodeId(999_999),
            verdict: ModelVerdict::Review,
            reason: "x".into(),
        },
    ];

    let found = evaluate_with(&tree, &env(), &RuleSet::builtin(), &labels);

    assert!(found.suggestions.is_empty(), "{found:#?}");
}

#[test]
fn a_scan_inside_a_kept_folder_gets_no_suggestions_at_all() {
    let tree = scan_at(&fs(), "/home/u/Documents/thesis", false);
    let labels = [review(&tree, "figures")];

    let found = evaluate_with(&tree, &env(), &RuleSet::builtin(), &labels);

    assert!(found.suggestions.is_empty());
    assert_eq!(found.totals.review, 0);
}

#[test]
fn a_model_label_is_never_safe_and_totals_count_it_as_review() {
    let tree = scan_at(&fs(), "/", false);

    let found = with(&tree, &[review(&tree, "home/u/stuff/big")]);

    assert_eq!((found.totals.safe, found.totals.keep), (0, 20));
    assert!(
        found
            .suggestions
            .iter()
            .filter(|s| s.source == SuggestionSource::Model)
            .all(|s| s.verdict == Verdict::Review)
    );
}
