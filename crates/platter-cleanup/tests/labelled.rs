//! Planning what the local model labelled: the same guard as for a rule or a hand-marked folder, always
//! `review` and always the trash. Planning never changes anything.

use std::path::{Path, PathBuf};

use platter_cleanup::{
    ADVISOR, CleanupAction, CleanupPlan, MARKED, Refusal, plan_cleanup, plan_labelled, plan_marked,
};
use platter_core::dto::Verdict;
use platter_core::testing::{FakeFs, NOW};
use platter_core::{CancelToken, Environment, ModelLabel, ModelVerdict, NodeId, Os, ScanConfig, Tree, scan};
use platter_rules::RuleSet;

const DAY: i64 = 86_400;

fn env_with(home: Option<&str>) -> Environment {
    Environment {
        home: home.map(PathBuf::from),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
    }
}

fn env() -> Environment {
    env_with(Some("/home/u"))
}

fn scan_at(fs: &FakeFs, root: &str) -> Tree {
    let config = ScanConfig {
        now: Some(NOW),
        ..ScanConfig::default()
    };
    scan(fs, Path::new(root), &config, &CancelToken::new(), &|_| {}).expect("the fake scans")
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

fn label(tree: &Tree, path: &str, verdict: ModelVerdict) -> ModelLabel {
    ModelLabel {
        node: node(tree, path),
        verdict,
        reason: "the model says so".into(),
    }
}

fn review(tree: &Tree, path: &str) -> ModelLabel {
    label(tree, path, ModelVerdict::Review)
}

fn aged(fs: &mut FakeFs, path: &str, size: u64, days: i64) {
    fs.file(path, size).set_mtime(path, NOW - days * DAY);
}

fn fs() -> FakeFs {
    let mut fs = FakeFs::new();
    aged(&mut fs, "/home/u/stuff/big/a.bin", 1000, 10);
    aged(&mut fs, "/home/u/stuff/big/inner/b.bin", 100, 10);
    aged(&mut fs, "/home/u/stuff/other/c.bin", 400, 10);
    aged(&mut fs, "/home/u/Documents/thesis/draft.odt", 10, 10);
    aged(&mut fs, "/home/u/.ssh/id_ed25519", 1, 10);
    aged(
        &mut fs,
        "/home/u/.local/share/Steam/steamapps/common/G/g.pak",
        10,
        10,
    );
    fs.symlink("/home/u/stuff/link", 1);
    fs.unreadable_dir("/home/u/stuff/locked");
    fs.file("/home/u/code/rs/Cargo.toml", 1);
    aged(&mut fs, "/home/u/code/rs/target/x", 700, 90);
    fs
}

fn paths(plan: &CleanupPlan) -> Vec<String> {
    plan.targets().map(|target| target.display_path()).collect()
}

fn why(plan: &CleanupPlan, path: &str) -> Option<Refusal> {
    plan.refused
        .iter()
        .find(|refused| refused.path == path)
        .map(|refused| refused.reason.clone())
}

#[test]
fn review_labels_are_planned_as_one_review_group_that_goes_to_the_trash() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let labels = [
        review(&tree, "home/u/stuff/big"),
        review(&tree, "home/u/stuff/other"),
        label(&tree, "home/u/code", ModelVerdict::Keep),
    ];

    let plan = plan_labelled(&tree, &env(), &RuleSet::builtin(), &labels, &fs);

    assert_eq!(plan.groups().len(), 1);
    let group = &plan.groups()[0];
    assert_eq!(
        (group.rule_id(), group.verdict(), group.action()),
        (ADVISOR, Verdict::Review, CleanupAction::Trash)
    );
    assert_eq!(ADVISOR, "advisor");
    assert_eq!(paths(&plan), ["/home/u/stuff/big", "/home/u/stuff/other"]);
    assert_eq!(plan.allocated, 1100 + 400);
    assert!(plan.refused.is_empty(), "{plan:#?}");
}

#[test]
fn only_keep_labels_or_none_plan_nothing() {
    let fs = fs();
    let tree = scan_at(&fs, "/");

    let none = plan_labelled(&tree, &env(), &RuleSet::builtin(), &[], &fs);
    let only_keep = plan_labelled(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &[label(&tree, "home/u/stuff/big", ModelVerdict::Keep)],
        &fs,
    );

    for plan in [none, only_keep] {
        assert!(plan.groups().is_empty() && plan.refused.is_empty(), "{plan:#?}");
        assert_eq!(plan.allocated, 0);
    }
}

#[test]
fn a_label_that_is_kept_holds_something_kept_or_lies_inside_it_is_refused() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let labels = [
        review(&tree, "home/u/Documents/thesis"),
        review(&tree, "home/u"),
        review(&tree, "home/u/.local/share/Steam/steamapps/common/G"),
        review(&tree, "home/u/.local/share/Steam"),
        review(&tree, "home/u/stuff/other"),
    ];

    let plan = plan_labelled(&tree, &env(), &RuleSet::builtin(), &labels, &fs);

    assert_eq!(paths(&plan), ["/home/u/stuff/other"]);
    for path in [
        "/home/u/Documents/thesis",
        "/home/u",
        "/home/u/.local/share/Steam/steamapps/common/G",
        "/home/u/.local/share/Steam",
    ] {
        assert_eq!(why(&plan, path), Some(Refusal::Kept), "{path}");
    }
    assert!(plan.refused.iter().all(|refused| refused.rule_id == ADVISOR));
}

#[test]
fn a_scan_inside_a_kept_folder_refuses_every_label() {
    let fs = fs();
    let tree = scan_at(&fs, "/home/u/Documents/thesis");
    let plan = plan_labelled(&tree, &env(), &RuleSet::builtin(), &[], &fs);
    assert!(plan.refused.is_empty());

    let mut fs = fs;
    aged(&mut fs, "/home/u/Documents/thesis/figures/f", 10, 10);
    let tree = scan_at(&fs, "/home/u/Documents/thesis");
    let plan = plan_labelled(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &[review(&tree, "figures")],
        &fs,
    );

    assert!(plan.groups().is_empty());
    assert_eq!(
        plan.refused.iter().map(|r| &r.reason).collect::<Vec<_>>(),
        [&Refusal::Kept]
    );
}

#[test]
fn the_guard_refuses_protected_places_links_unreadable_folders_and_the_scan_root() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let labels = [
        review(&tree, "home/u/.ssh"),
        review(&tree, "home/u/stuff/link"),
        review(&tree, "home/u/stuff/locked"),
        review(&tree, "home/u/stuff"),
        review(&tree, "home/u/stuff/big"),
        ModelLabel {
            node: NodeId(999_999),
            verdict: ModelVerdict::Review,
            reason: "x".into(),
        },
    ];

    let plan = plan_labelled(&tree, &env(), &RuleSet::builtin(), &labels, &fs);

    assert!(matches!(why(&plan, "/home/u/.ssh"), Some(Refusal::Protected(_))));
    assert_eq!(why(&plan, "/home/u/stuff/link"), Some(Refusal::Symlink));
    assert_eq!(why(&plan, "/home/u/stuff/locked"), Some(Refusal::Unreadable));
    assert_eq!(why(&plan, "/home/u/stuff"), Some(Refusal::Unreadable));
    assert_eq!(paths(&plan), ["/home/u/stuff/big"]);
    assert_eq!(plan.refused.len(), 4, "an unknown id is ignored: {plan:#?}");
}

#[test]
fn the_scan_root_is_refused() {
    let fs = fs();
    let tree = scan_at(&fs, "/home/u/stuff/other");
    let root = ModelLabel {
        node: tree.root(),
        verdict: ModelVerdict::Review,
        reason: "x".into(),
    };

    let plan = plan_labelled(&tree, &env(), &RuleSet::builtin(), &[root], &fs);

    assert!(plan.groups().is_empty());
    assert_eq!(why(&plan, "/home/u/stuff/other"), Some(Refusal::ScanRoot));
}

#[test]
fn without_a_home_directory_every_label_is_refused() {
    let fs = fs();
    let tree = scan_at(&fs, "/");

    let plan = plan_labelled(
        &tree,
        &env_with(None),
        &RuleSet::builtin(),
        &[review(&tree, "home/u/stuff/big")],
        &fs,
    );

    assert!(plan.groups().is_empty());
    assert_eq!(why(&plan, "/home/u/stuff/big"), Some(Refusal::HomeUnknown));
}

#[test]
fn a_label_swapped_for_a_link_or_removed_after_the_scan_is_refused() {
    let mut fs = fs();
    let tree = scan_at(&fs, "/");
    let labels = [
        review(&tree, "home/u/stuff/big"),
        review(&tree, "home/u/stuff/other"),
    ];
    fs.swap_for_symlink("/home/u/stuff/big");
    fs.remove("/home/u/stuff/other");

    let plan = plan_labelled(&tree, &env(), &RuleSet::builtin(), &labels, &fs);

    assert!(plan.groups().is_empty());
    assert_eq!(why(&plan, "/home/u/stuff/big"), Some(Refusal::Symlink));
    assert!(matches!(
        why(&plan, "/home/u/stuff/other"),
        Some(Refusal::Changed(what)) if what.contains("gone")
    ));
}

#[test]
fn a_folder_on_the_way_swapped_for_a_link_is_refused() {
    let mut fs = fs();
    let tree = scan_at(&fs, "/");
    let labels = [review(&tree, "home/u/stuff/big")];
    fs.swap_for_symlink("/home/u/stuff");

    let plan = plan_labelled(&tree, &env(), &RuleSet::builtin(), &labels, &fs);

    assert!(plan.groups().is_empty());
    assert!(matches!(
        why(&plan, "/home/u/stuff/big"),
        Some(Refusal::Changed(what)) if what.contains("link")
    ));
}

#[test]
fn a_label_inside_another_label_is_left_to_the_outer_one_and_duplicates_count_once() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let labels = [
        review(&tree, "home/u/stuff/big/inner"),
        review(&tree, "home/u/stuff/big"),
        review(&tree, "home/u/stuff/big"),
    ];

    let plan = plan_labelled(&tree, &env(), &RuleSet::builtin(), &labels, &fs);

    assert_eq!(paths(&plan), ["/home/u/stuff/big"]);
    assert_eq!(why(&plan, "/home/u/stuff/big/inner"), Some(Refusal::Nested));
    assert_eq!(plan.allocated, 1100);
}

mod absorb {
    use super::*;

    #[test]
    fn a_plan_takes_in_the_groups_refusals_and_totals_of_another() {
        let fs = fs();
        let tree = scan_at(&fs, "/");
        let rules = RuleSet::builtin();

        let mut plan = plan_cleanup(&tree, &env(), &rules, &["rust-target", "no-such-rule"], &fs);
        let labelled = plan_labelled(
            &tree,
            &env(),
            &rules,
            &[review(&tree, "home/u/stuff/big"), review(&tree, "home/u/.ssh")],
            &fs,
        );
        plan.absorb(labelled);

        let groups: Vec<_> = plan
            .groups()
            .iter()
            .map(|group| (group.rule_id(), group.allocated()))
            .collect();
        assert_eq!(groups, [("rust-target", 700), (ADVISOR, 1100)]);
        assert_eq!(plan.allocated, 1800);
        assert_eq!(plan.not_found, ["no-such-rule"]);
        assert!(matches!(why(&plan, "/home/u/.ssh"), Some(Refusal::Protected(_))));
        assert_eq!(plan.scan_root(), Path::new("/"));
    }

    #[test]
    fn a_target_inside_another_target_of_the_merged_plan_is_refused_as_nested() {
        let fs = fs();
        let tree = scan_at(&fs, "/");
        let rules = RuleSet::builtin();

        let mut plan = plan_labelled(&tree, &env(), &rules, &[review(&tree, "home/u/stuff/big")], &fs);
        let marked = plan_marked(
            &tree,
            &env(),
            &rules,
            &[
                node(&tree, "home/u/stuff/big/inner"),
                node(&tree, "home/u/stuff/other"),
            ],
            &fs,
        );
        plan.absorb(marked);

        let groups: Vec<_> = plan
            .groups()
            .iter()
            .map(|group| (group.rule_id(), group.allocated()))
            .collect();
        assert_eq!(groups, [(ADVISOR, 1100), (MARKED, 400)]);
        assert_eq!(plan.allocated, 1500);
        assert_eq!(why(&plan, "/home/u/stuff/big/inner"), Some(Refusal::Nested));
    }

    #[test]
    fn the_same_folder_in_both_plans_is_acted_on_once() {
        let fs = fs();
        let tree = scan_at(&fs, "/");
        let rules = RuleSet::builtin();
        let big = node(&tree, "home/u/stuff/big");

        let mut plan = plan_labelled(&tree, &env(), &rules, &[review(&tree, "home/u/stuff/big")], &fs);
        plan.absorb(plan_marked(&tree, &env(), &rules, &[big], &fs));

        assert_eq!(paths(&plan), ["/home/u/stuff/big"]);
        assert_eq!(plan.allocated, 1100);
        assert_eq!(plan.groups().len(), 1);
        assert_eq!(plan.groups()[0].rule_id(), ADVISOR);
        assert_eq!(why(&plan, "/home/u/stuff/big"), Some(Refusal::Nested));
    }

    #[test]
    fn an_empty_plan_takes_over_the_scan_root_of_the_other() {
        let fs = fs();
        let tree = scan_at(&fs, "/home/u");
        let mut plan = CleanupPlan::default();

        plan.absorb(plan_labelled(
            &tree,
            &env(),
            &RuleSet::builtin(),
            &[review(&tree, "stuff/big")],
            &fs,
        ));

        assert_eq!(plan.scan_root(), Path::new("/home/u"));
        assert_eq!(paths(&plan), ["/home/u/stuff/big"]);
    }
}
