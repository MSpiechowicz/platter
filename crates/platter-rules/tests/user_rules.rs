//! The user's own rules: where they stand among the built-in ones (keep of any source wins, then the
//! built-in rules, then the user's `safe`) and that they match exact folders only.

mod common;

use common::*;
use platter_core::dto::{SuggestionSource, Verdict};
use platter_core::testing::FakeFs;
use platter_core::{UserRule, UserVerdict};
use platter_rules::{Explainer, RuleSet, evaluate, matches, scan_is_kept};

fn user(id: u32, verdict: UserVerdict, folder: &str) -> UserRule {
    UserRule {
        id,
        verdict,
        folder: folder.into(),
    }
}

fn with(rules: Vec<UserRule>) -> RuleSet {
    RuleSet::builtin().with_user(rules)
}

fn home_fs() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/app/blob", 300);
    fs.file("/home/u/.cache/keepme/blob", 700);
    fs.file("/home/u/Documents/thesis/draft", 10);
    fs.file("/home/u/stuff/big/blob", 5000);
    fs.file("/home/u/stuff/other/blob", 400);
    fs.file("/home/u/work/proj/Cargo.toml", 1);
    aged_file(&mut fs, "/home/u/work/proj/target/x", 900, 90);
    fs
}

#[test]
fn without_user_rules_the_set_is_the_builtin_one() {
    let tree = scan_at(&home_fs(), "/", false);

    let plain = evaluate(&tree, &env(), &RuleSet::builtin());
    let empty = evaluate(&tree, &env(), &with(Vec::new()));

    assert_eq!(plain, empty);
}

#[test]
fn a_user_keep_is_suggested_as_keep_from_the_user() {
    let tree = scan_at(&home_fs(), "/", false);

    let found = evaluate(
        &tree,
        &env(),
        &with(vec![user(1, UserVerdict::Keep, "/home/u/stuff/big")]),
    );

    let kept = get(&found, "user-keep").unwrap();
    assert_eq!(kept.verdict, Verdict::Keep);
    assert_eq!(kept.source, SuggestionSource::User);
    assert_eq!((kept.allocated, kept.items), (5000, 1));
    assert_eq!(kept.path, "~/stuff/big");
    assert_eq!(found.totals.keep, 5000 + 10);
}

#[test]
fn a_user_keep_beats_a_builtin_safe_rule_at_the_same_node() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![user(1, UserVerdict::Keep, "/home/u/work/proj/target")]);

    let found = evaluate(&tree, &env(), &rules);

    assert!(get(&found, "rust-target").is_none());
    assert_eq!(get(&found, "user-keep").unwrap().allocated, 900);
}

#[test]
fn a_user_keep_beats_a_builtin_rule_that_would_hold_the_folder() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![user(1, UserVerdict::Keep, "/home/u/.cache/keepme")]);

    let found = evaluate(&tree, &env(), &rules);

    // The cache is entered, not claimed: cleaning it would clean the kept folder too.
    assert!(get(&found, "app-cache").is_none());
    assert_eq!(get(&found, "user-keep").unwrap().allocated, 700);
}

#[test]
fn a_user_keep_as_an_outer_folder_is_never_entered() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![user(1, UserVerdict::Keep, "/home/u/work")]);

    let found = evaluate(&tree, &env(), &rules);

    assert!(get(&found, "rust-target").is_none());
    assert_eq!(get(&found, "user-keep").unwrap().allocated, 901);
}

#[test]
fn a_builtin_rule_beats_a_user_safe_at_the_same_node() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![user(1, UserVerdict::Safe, "/home/u/.cache")]);

    let found = evaluate(&tree, &env(), &rules);

    assert_eq!(get(&found, "app-cache").unwrap().verdict, Verdict::Review);
    assert!(get(&found, "user-safe").is_none());
}

#[test]
fn a_builtin_name_rule_beats_a_user_safe_at_the_same_node() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![user(1, UserVerdict::Safe, "/home/u/work/proj/target")]);

    let found = evaluate(&tree, &env(), &rules);

    assert!(get(&found, "rust-target").is_some());
    assert!(get(&found, "user-safe").is_none());
}

#[test]
fn a_user_safe_as_an_outer_folder_claims_what_is_inside() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![user(1, UserVerdict::Safe, "/home/u/work")]);

    let found = evaluate(&tree, &env(), &rules);

    let safe = get(&found, "user-safe").unwrap();
    assert_eq!(safe.verdict, Verdict::Safe);
    assert_eq!(safe.source, SuggestionSource::User);
    assert_eq!(safe.allocated, 901);
    assert!(get(&found, "rust-target").is_none());
}

#[test]
fn a_user_safe_on_a_folder_no_rule_knows_is_suggested() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![
        user(1, UserVerdict::Safe, "/home/u/stuff/big"),
        user(2, UserVerdict::Safe, "/home/u/stuff/other"),
    ]);

    let found = evaluate(&tree, &env(), &rules);

    let safe = get(&found, "user-safe").unwrap();
    assert_eq!((safe.allocated, safe.items), (5400, 2));
    assert_eq!(safe.path, "2 folders you chose");
    assert_eq!(found.totals.safe, 5400 + 900);
}

#[test]
fn a_user_keep_wins_over_a_user_safe_on_the_same_folder() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![
        user(1, UserVerdict::Safe, "/home/u/stuff/big"),
        user(2, UserVerdict::Keep, "/home/u/stuff/big"),
    ]);

    let found = evaluate(&tree, &env(), &rules);

    assert!(get(&found, "user-safe").is_none());
    assert!(get(&found, "user-keep").is_some());
}

#[test]
fn a_user_safe_folder_that_holds_a_kept_one_is_not_claimed() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![
        user(1, UserVerdict::Safe, "/home/u/stuff"),
        user(2, UserVerdict::Keep, "/home/u/stuff/big"),
    ]);

    let found = evaluate(&tree, &env(), &rules);

    assert!(get(&found, "user-safe").is_none());
    assert_eq!(get(&found, "user-keep").unwrap().allocated, 5000);
}

#[test]
fn a_scan_inside_a_user_kept_folder_suggests_nothing() {
    let tree = scan_at(&home_fs(), "/home/u/work", false);
    let rules = with(vec![user(1, UserVerdict::Keep, "/home/u/work")]);
    let inside = with(vec![user(1, UserVerdict::Keep, "/home/u")]);

    // The kept folder itself, scanned as the root, is matched like any other node.
    assert!(!scan_is_kept(&tree, &env(), &rules));
    assert!(scan_is_kept(&tree, &env(), &inside));

    let found = evaluate(&tree, &env(), &inside);
    assert!(found.suggestions.is_empty());
    assert!(matches(&tree, &env(), &inside).is_empty());
    assert!(
        Explainer::new(&tree, &env(), &inside)
            .covering(tree.root())
            .is_none()
    );
}

#[test]
fn user_rules_match_by_exact_folder_never_by_name() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/a/target/x", 10);
    fs.file("/home/u/b/target/y", 20);
    let tree = scan_at(&fs, "/", false);
    let rules = with(vec![user(1, UserVerdict::Keep, "/home/u/a/target")]);

    let found = evaluate(&tree, &env(), &rules);

    assert_eq!(get(&found, "user-keep").unwrap().items, 1);
    assert_eq!(get(&found, "user-keep").unwrap().allocated, 10);
}

#[test]
fn a_rule_for_a_folder_that_is_not_in_the_scan_or_is_a_file_matches_nothing() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![
        user(1, UserVerdict::Keep, "/home/u/missing"),
        user(2, UserVerdict::Safe, "/home/u/work/proj/Cargo.toml"),
    ]);

    let found = evaluate(&tree, &env(), &rules);

    assert!(get(&found, "user-keep").is_none());
    assert!(get(&found, "user-safe").is_none());
}

#[test]
fn matches_and_the_explainer_agree_with_the_suggestions() {
    let tree = scan_at(&home_fs(), "/", false);
    let rules = with(vec![
        user(1, UserVerdict::Keep, "/home/u/work"),
        user(2, UserVerdict::Safe, "/home/u/stuff/big"),
    ]);

    let all = matches(&tree, &env(), &rules);
    let explainer = Explainer::new(&tree, &env(), &rules);

    let keep = all.iter().find(|rule| rule.id == "user-keep").unwrap();
    assert_eq!(keep.nodes, [find(&tree, "home/u/work")]);
    assert!(all.iter().any(|rule| rule.id == "user-safe"));

    let covering = explainer
        .covering(find(&tree, "home/u/work/proj/target/x"))
        .unwrap();
    assert_eq!(
        (covering.id, covering.source),
        ("user-keep", SuggestionSource::User)
    );
    assert_eq!(covering.verdict, Verdict::Keep);

    let builtin = explainer.covering(find(&tree, "home/u/.cache/app")).unwrap();
    assert_eq!(builtin.source, SuggestionSource::Rule);
}

#[test]
fn the_user_rules_are_kept_with_the_set_and_replaced_by_the_next_call() {
    let rules =
        with(vec![user(1, UserVerdict::Keep, "/a")]).with_user(vec![user(2, UserVerdict::Safe, "/b")]);

    assert_eq!(rules.user().len(), 1);
    assert_eq!(rules.user()[0].id, 2);
    assert_eq!(
        rules.rules().iter().filter(|rule| rule.id == "user-keep").count(),
        1
    );
    assert_eq!(rules.rules().first().unwrap().id, "user-keep");
    assert_eq!(rules.rules().last().unwrap().id, "user-safe");
}

// A folder can be kept and have no bytes of its own: a directory has no blocks on btrfs, XFS or tmpfs, and
// it holds only empty files and links, or hard links whose bytes the scan charged to another folder (a
// `cp -al` or rsnapshot backup).

fn empty_keep_fs() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/proj/data/blob", 5000);
    fs.file("/home/u/proj/keepme/.gitkeep", 0);
    fs.symlink("/home/u/proj/keepme/link", 0);
    fs
}

fn hard_linked_backup_fs() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/backups/2024/photos.tar", 900_000);
    fs.hardlink(
        "/home/u/backups/2024/photos.tar",
        "/home/u/backups/2025/photos.tar",
    );
    fs.file("/home/u/backups/other/blob", 70);
    fs
}

fn label(tree: &platter_core::Tree, path: &str) -> platter_core::ModelLabel {
    platter_core::ModelLabel {
        node: find(tree, path),
        verdict: platter_core::ModelVerdict::Review,
        reason: String::new(),
    }
}

#[test]
fn a_kept_folder_with_no_bytes_is_still_a_match_and_explained_as_the_users() {
    let cases = [
        (
            empty_keep_fs(),
            "/home/u/proj/keepme",
            "home/u/proj/keepme",
            "home/u/proj",
        ),
        (
            hard_linked_backup_fs(),
            "/home/u/backups/2025",
            "home/u/backups/2025",
            "home/u/backups",
        ),
    ];

    for (fs, folder, at, parent) in cases {
        let tree = scan_at(&fs, "/", false);
        let rules = with(vec![user(1, UserVerdict::Keep, folder)]);
        assert_eq!(tree.allocated(find(&tree, at)), 0, "{folder}: control");

        let found = matches(&tree, &env(), &rules);
        let explainer = Explainer::new(&tree, &env(), &rules);

        let kept = found.iter().find(|rule| rule.id == "user-keep").expect(folder);
        assert_eq!(kept.nodes, [find(&tree, at)], "{folder}");
        assert!(found.iter().all(|rule| rule.id != "user-safe"), "{folder}");
        let covering = explainer.covering(find(&tree, at)).expect(folder);
        assert_eq!(
            (covering.id, covering.verdict, covering.source),
            ("user-keep", Verdict::Keep, SuggestionSource::User),
            "{folder}"
        );
        assert!(
            explainer.touches(find(&tree, parent)),
            "{folder}: the parent holds a kept folder"
        );

        // It has nothing to show, so no suggestion lists it.
        let shown = evaluate(&tree, &env(), &rules);
        assert!(get(&shown, "user-keep").is_none(), "{folder}: {shown:?}");
    }
}

#[test]
fn a_model_label_on_the_parent_of_a_kept_folder_with_no_bytes_is_not_listed() {
    let cases = [
        (empty_keep_fs(), "/home/u/proj/keepme", "home/u/proj"),
        (hard_linked_backup_fs(), "/home/u/backups/2025", "home/u/backups"),
    ];

    for (fs, folder, parent) in cases {
        let tree = scan_at(&fs, "/", false);
        let rules = with(vec![user(1, UserVerdict::Keep, folder)]);

        let found = platter_rules::evaluate_with(&tree, &env(), &rules, &[label(&tree, parent)]);

        assert!(
            get(&found, platter_rules::ADVISOR_ID).is_none(),
            "{folder}: {found:?}"
        );
    }
}

#[test]
fn a_builtin_keep_with_no_bytes_is_still_kept_and_is_the_rules() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/Documents/empty.txt", 0);
    fs.file("/home/u/stuff/blob", 40);
    let tree = scan_at(&fs, "/", false);
    let rules = RuleSet::builtin();

    let explainer = Explainer::new(&tree, &env(), &rules);

    let covering = explainer.covering(find(&tree, "home/u/Documents")).expect("kept");
    assert_eq!((covering.id, covering.verdict), ("documents", Verdict::Keep));
    assert_eq!(covering.source, SuggestionSource::Rule);
    assert!(
        matches(&tree, &env(), &rules)
            .iter()
            .any(|rule| rule.id == "documents")
    );
}

#[test]
fn a_kept_folder_that_is_a_skipped_mount_is_a_match_with_nothing_to_show() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/proj/data/blob", 5000);
    fs.mount("/home/u/proj/nas");
    fs.file("/home/u/proj/nas/photo", 100);
    let tree = scan_at(&fs, "/home/u", true);
    let nas = find(&tree, "proj/nas");
    let proj = find(&tree, "proj");
    assert_eq!(tree.kind(nas), platter_core::NodeKind::Mount, "control");
    let rules = with(vec![
        user(1, UserVerdict::Keep, "/home/u/proj/nas"),
        user(2, UserVerdict::Safe, "/home/u/proj"),
    ]);

    let found = matches(&tree, &env(), &rules);
    let explainer = Explainer::new(&tree, &env(), &rules);
    let shown = evaluate(&tree, &env(), &rules);

    let keep = found.iter().find(|rule| rule.id == "user-keep").expect("matched");
    assert_eq!(keep.nodes, [nas]);
    assert!(explainer.holds(proj));
    let covering = explainer.covering(nas).expect("covered");
    assert_eq!(
        (covering.id, covering.source),
        ("user-keep", SuggestionSource::User)
    );
    // The folder around it is entered, not claimed by the user's `safe`; a mount has no bytes to list.
    assert!(found.iter().all(|rule| rule.id != "user-safe"), "{found:?}");
    assert!(get(&shown, "user-keep").is_none());
    assert_eq!(shown.totals.keep, 0);
}
