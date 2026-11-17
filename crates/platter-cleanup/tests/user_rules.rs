//! Planning with the user's own rules: a `keep` blocks every kind of plan, a `safe` goes to the trash
//! and never past the guard. Planning never changes anything.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use platter_cleanup::{
    CandidateLimits, CleanupAction, CleanupOptions, CleanupPlan, KEPT_LEARNING_BUDGET, KeptFolders,
    Protected, Refusal, action_for, label_candidates, plan_cleanup, plan_cleanup_with, plan_labelled,
    plan_labelled_with, plan_marked, with_real_keeps,
};
use platter_core::dto::SuggestionSource;
use platter_core::dto::Verdict;
use platter_core::testing::{FakeFs, NOW};
use platter_core::{
    CancelToken, EntryProbe, Environment, ModelLabel, ModelVerdict, NodeId, NodeKind, Os, ProbedEntry,
    ScanConfig, Tree, UserRule, UserVerdict, scan,
};
use platter_rules::{Explainer, RuleSet, evaluate};

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

fn rules(user: &[(UserVerdict, &str)]) -> RuleSet {
    let user = user
        .iter()
        .zip(1u32..)
        .map(|((verdict, folder), id)| UserRule {
            id,
            verdict: *verdict,
            folder: folder.into(),
        })
        .collect();

    RuleSet::builtin().with_user(user)
}

fn fs() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/stuff/big/a.bin", 1000);
    fs.file("/home/u/stuff/big/inner/b.bin", 100);
    fs.file("/home/u/stuff/other/c.bin", 400);
    fs.file("/home/u/.cache/app/blob", 300);
    fs.file("/home/u/.cache/precious/blob", 700);
    fs.file("/home/u/.ssh/keys/id", 5);
    fs.file("/home/u/Documents/thesis/draft", 10);
    fs
}

fn refused_for<'a>(plan: &'a CleanupPlan, path: &str) -> Option<&'a Refusal> {
    plan.refused
        .iter()
        .find(|refused| refused.path == path)
        .map(|refused| &refused.reason)
}

#[test]
fn a_user_safe_folder_is_planned_for_the_trash_and_never_deleted() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let rules = rules(&[(UserVerdict::Safe, "/home/u/stuff/big")]);

    let plan = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["user-safe"],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(plan.groups().len(), 1);
    let group = &plan.groups()[0];
    assert_eq!(
        (group.rule_id(), group.verdict(), group.action()),
        ("user-safe", Verdict::Safe, CleanupAction::Trash)
    );
    assert_eq!(group.targets()[0].display_path(), "/home/u/stuff/big");
    assert_eq!(plan.allocated, 1100);
    assert!(matches!(
        action_for("user-safe", &CleanupOptions::default()),
        Some(CleanupAction::Trash)
    ));
}

#[test]
fn user_keep_is_never_planned() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let rules = rules(&[(UserVerdict::Keep, "/home/u/stuff/big")]);

    let plan = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["user-keep"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty());
    assert_eq!(refused_for(&plan, "/home/u/stuff/big"), Some(&Refusal::Kept));
}

#[test]
fn a_user_safe_cannot_get_past_the_protected_places() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.ssh/keys/id", 5);
    fs.file("/home/u/stuff/big/a.bin", 1000);
    let tree = scan_at(&fs, "/");
    // What the add command refuses could still be written into the file by hand. The outermost wins, so
    // each is tried alone.
    for folder in ["/home/u", "/home/u/.ssh/keys"] {
        let rules = rules(&[(UserVerdict::Safe, folder)]);

        let plan = plan_cleanup(
            &tree,
            &env(),
            &rules,
            &["user-safe"],
            &fs,
            &CleanupOptions::default(),
        );

        assert!(plan.groups().is_empty(), "{folder}: {plan:#?}");
        assert!(
            matches!(refused_for(&plan, folder), Some(Refusal::Protected(_))),
            "{folder}: {plan:#?}"
        );
    }
}

#[test]
fn a_user_safe_on_a_personal_folder_loses_to_the_builtin_keep() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let rules = rules(&[(UserVerdict::Safe, "/home/u/Documents")]);

    let plan = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["user-safe", "documents"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert_eq!(refused_for(&plan, "/home/u/Documents"), Some(&Refusal::Kept));
}

#[test]
fn a_user_safe_on_the_scan_root_is_refused() {
    let fs = fs();
    let tree = scan_at(&fs, "/home/u/stuff");
    let rules = rules(&[(UserVerdict::Safe, "/home/u/stuff")]);

    let plan = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["user-safe"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty());
    assert_eq!(refused_for(&plan, "/home/u/stuff"), Some(&Refusal::ScanRoot));
}

#[test]
fn a_user_keep_inside_a_builtin_match_stops_that_match_being_planned() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let rules = rules(&[(UserVerdict::Keep, "/home/u/.cache/precious")]);

    let plan = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["app-cache"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert_eq!(plan.not_found, ["app-cache"]);
}

#[test]
fn a_user_keep_blocks_marked_folders_inside_around_and_equal() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let rules = rules(&[(UserVerdict::Keep, "/home/u/stuff/big")]);
    let marked = [
        node(&tree, "home/u/stuff/big"),
        node(&tree, "home/u/stuff/big/inner"),
        node(&tree, "home/u/stuff"),
    ];

    let plan = plan_marked(&tree, &env(), &rules, &marked, &fs, &CleanupOptions::default());

    assert!(plan.groups().is_empty(), "{plan:#?}");
    for path in ["/home/u/stuff/big", "/home/u/stuff/big/inner", "/home/u/stuff"] {
        assert_eq!(refused_for(&plan, path), Some(&Refusal::Kept), "{path}");
    }
}

#[test]
fn a_user_keep_leaves_other_marked_folders_alone() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let rules = rules(&[(UserVerdict::Keep, "/home/u/stuff/big")]);

    let plan = plan_marked(
        &tree,
        &env(),
        &rules,
        &[node(&tree, "home/u/stuff/other")],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(plan.groups().len(), 1);
}

#[test]
fn a_user_keep_blocks_labelled_folders() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let rules = rules(&[(UserVerdict::Keep, "/home/u/stuff/big")]);
    let labels: Vec<ModelLabel> = ["home/u/stuff/big", "home/u/stuff/big/inner", "home/u/stuff/other"]
        .iter()
        .map(|path| ModelLabel {
            node: node(&tree, path),
            verdict: ModelVerdict::Review,
            reason: String::new(),
        })
        .collect();

    let plan = plan_labelled(&tree, &env(), &rules, &labels, &fs, &CleanupOptions::default());

    let planned: Vec<String> = plan.targets().map(|target| target.display_path()).collect();
    assert_eq!(planned, ["/home/u/stuff/other"]);
    assert_eq!(refused_for(&plan, "/home/u/stuff/big"), Some(&Refusal::Kept));
    assert_eq!(
        refused_for(&plan, "/home/u/stuff/big/inner"),
        Some(&Refusal::Kept)
    );
}

#[test]
fn a_scan_inside_a_user_kept_folder_plans_nothing() {
    let fs = fs();
    let tree = scan_at(&fs, "/home/u/stuff/big");
    let rules = rules(&[(UserVerdict::Keep, "/home/u/stuff")]);

    let marked = plan_marked(
        &tree,
        &env(),
        &rules,
        &[node(&tree, "inner")],
        &fs,
        &CleanupOptions::default(),
    );
    let by_rule = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["user-safe", "app-cache"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(marked.groups().is_empty());
    assert_eq!(
        refused_for(&marked, "/home/u/stuff/big/inner"),
        Some(&Refusal::Kept)
    );
    assert!(by_rule.groups().is_empty());
}

#[test]
fn a_user_kept_folder_is_never_a_label_candidate_and_a_scan_inside_one_has_none() {
    let fs = fs();
    let tree = scan_at(&fs, "/");
    let limits = platter_cleanup::CandidateLimits {
        min_bytes: 1,
        max: 50,
    };
    let names = |rules: &RuleSet, tree: &Tree| -> Vec<String> {
        platter_cleanup::label_candidates(tree, &env(), rules, limits)
            .into_iter()
            .map(|node| tree.path_of(node).to_string_lossy().into_owned())
            .collect()
    };

    let free = names(&rules(&[]), &tree);
    let kept = names(&rules(&[(UserVerdict::Keep, "/home/u/stuff/big")]), &tree);

    assert!(free.contains(&"/home/u/stuff".to_owned()), "control: {free:?}");
    // The folder that holds the kept one is entered, not chosen; what lies beside the kept folder is.
    assert!(!kept.contains(&"/home/u/stuff".to_owned()), "{kept:?}");
    assert!(
        !kept.iter().any(|path| path.starts_with("/home/u/stuff/big")),
        "{kept:?}"
    );
    assert!(kept.contains(&"/home/u/stuff/other".to_owned()), "{kept:?}");

    let inside = scan_at(&fs, "/home/u/stuff/big");
    assert!(names(&rules(&[(UserVerdict::Keep, "/home/u/stuff")]), &inside).is_empty());
}

// A folder can be kept and have no bytes of its own: a directory has no blocks on btrfs, XFS or tmpfs, and
// it holds only empty files and links, or hard links whose bytes the scan charged to another folder (a
// `cp -al` or rsnapshot backup). It is kept all the same, by every kind of plan.

struct Bytesless {
    fs: FakeFs,
    kept: &'static str,
    inside: &'static str,
    parent: &'static str,
}

fn bytesless_cases() -> Vec<Bytesless> {
    let mut empty = FakeFs::new();
    empty.file("/home/u/proj/data/blob", 5000);
    empty.file("/home/u/proj/keepme/.gitkeep", 0);
    empty.symlink("/home/u/proj/keepme/link", 0);
    empty.file("/home/u/proj/keepme/inner/.gitkeep", 0);

    let mut backups = FakeFs::new();
    backups.file("/home/u/backups/2024/photos.tar", 900_000);
    backups.hardlink(
        "/home/u/backups/2024/photos.tar",
        "/home/u/backups/2025/photos.tar",
    );
    backups.file("/home/u/backups/2025/inner/other.tar", 0);
    backups.file("/home/u/backups/other/blob", 70);

    vec![
        Bytesless {
            fs: empty,
            kept: "/home/u/proj/keepme",
            inside: "/home/u/proj/keepme/inner",
            parent: "/home/u/proj",
        },
        Bytesless {
            fs: backups,
            kept: "/home/u/backups/2025",
            inside: "/home/u/backups/2025/inner",
            parent: "/home/u/backups",
        },
    ]
}

#[test]
fn a_kept_folder_with_no_bytes_blocks_marked_folders_inside_around_and_equal() {
    for case in bytesless_cases() {
        let tree = scan_at(&case.fs, "/");
        let relative = |path: &str| path.trim_start_matches('/').to_owned();
        assert_eq!(
            tree.allocated(node(&tree, &relative(case.kept))),
            0,
            "{}: control",
            case.kept
        );
        let rules = rules(&[(UserVerdict::Keep, case.kept)]);
        let marked: Vec<NodeId> = [case.kept, case.inside, case.parent]
            .iter()
            .map(|path| node(&tree, &relative(path)))
            .collect();

        let plan = plan_marked(
            &tree,
            &env(),
            &rules,
            &marked,
            &case.fs,
            &CleanupOptions::default(),
        );

        assert!(plan.groups().is_empty(), "{}: {plan:#?}", case.kept);
        for path in [case.kept, case.inside, case.parent] {
            assert_eq!(refused_for(&plan, path), Some(&Refusal::Kept), "{path}");
        }
    }
}

#[test]
fn a_kept_folder_with_no_bytes_blocks_a_label_on_its_parent_and_a_user_safe_there() {
    for case in bytesless_cases() {
        let tree = scan_at(&case.fs, "/");
        let relative = |path: &str| path.trim_start_matches('/').to_owned();
        let user = [(UserVerdict::Keep, case.kept), (UserVerdict::Safe, case.parent)];
        let rules = rules(&user);
        let labels = [ModelLabel {
            node: node(&tree, &relative(case.parent)),
            verdict: ModelVerdict::Review,
            reason: String::new(),
        }];

        let labelled = plan_labelled(
            &tree,
            &env(),
            &rules,
            &labels,
            &case.fs,
            &CleanupOptions::default(),
        );
        let by_rule = plan_cleanup(
            &tree,
            &env(),
            &rules,
            &["user-safe"],
            &case.fs,
            &CleanupOptions::default(),
        );

        assert!(labelled.groups().is_empty(), "{}: {labelled:#?}", case.kept);
        assert_eq!(
            refused_for(&labelled, case.parent),
            Some(&Refusal::Kept),
            "{}",
            case.kept
        );
        assert!(by_rule.groups().is_empty(), "{}: {by_rule:#?}", case.kept);
    }
}

#[test]
fn the_parent_of_a_kept_folder_with_no_bytes_is_not_a_label_candidate() {
    let limits = platter_cleanup::CandidateLimits {
        min_bytes: 1,
        max: 50,
    };

    for case in bytesless_cases() {
        let tree = scan_at(&case.fs, "/");
        let free = platter_cleanup::label_candidates(&tree, &env(), &rules(&[]), limits);
        let kept = platter_cleanup::label_candidates(
            &tree,
            &env(),
            &rules(&[(UserVerdict::Keep, case.kept)]),
            limits,
        );
        let parent = node(&tree, case.parent.trim_start_matches('/'));

        assert!(free.contains(&parent), "{}: control", case.kept);
        assert!(!kept.contains(&parent), "{}", case.kept);
    }
}

// Another path to a kept folder (a bind mount, a btrfs top level mounted elsewhere) is the same folder: the
// rules match by path, the guard also compares identities.

fn aliased_fs() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/work/proj/Cargo.toml", 1);
    fs.file("/home/u/work/proj/target/x", 900);
    fs.file("/home/u/work/proj/keep/a.bin", 50);
    fs.set_mtime("/home/u/work/proj/target/x", NOW - 90 * 86_400);
    fs.set_mtime("/home/u/work/proj/target", NOW - 90 * 86_400);

    // The same folder under another root, and an unrelated one that only looks alike.
    fs.file("/data/alias/Cargo.toml", 1);
    fs.file("/data/alias/target/x", 900);
    fs.set_mtime("/data/alias/target/x", NOW - 90 * 86_400);
    fs.set_mtime("/data/alias/target", NOW - 90 * 86_400);
    fs.file("/data/alias/keep/a.bin", 50);
    fs.file("/data/other/Cargo.toml", 1);
    fs.file("/data/other/target/x", 900);
    fs.set_mtime("/data/other/target/x", NOW - 90 * 86_400);
    fs.set_mtime("/data/other/target", NOW - 90 * 86_400);
    fs.file("/data/other/keep/a.bin", 50);

    let (dev, ino) = fs.identity("/home/u/work/proj");
    fs.set_identity("/data/alias", dev, ino, 2);
    fs
}

#[test]
fn another_path_to_a_user_kept_folder_is_kept_in_every_kind_of_plan() {
    let fs = aliased_fs();
    let tree = scan_at(&fs, "/");
    let rules = rules(&[(UserVerdict::Keep, "/home/u/work/proj")]);

    let by_rule = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );
    let marked = plan_marked(
        &tree,
        &env(),
        &rules,
        &[
            node(&tree, "data/alias"),
            node(&tree, "data/alias/keep"),
            node(&tree, "data/other/keep"),
        ],
        &fs,
        &CleanupOptions::default(),
    );
    let labels: Vec<ModelLabel> = ["data/alias/keep", "data/other/keep"]
        .iter()
        .map(|path| ModelLabel {
            node: node(&tree, path),
            verdict: ModelVerdict::Review,
            reason: String::new(),
        })
        .collect();
    let labelled = plan_labelled(&tree, &env(), &rules, &labels, &fs, &CleanupOptions::default());

    let planned =
        |plan: &CleanupPlan| -> Vec<String> { plan.targets().map(|target| target.display_path()).collect() };
    // The folder at the other path is planned as the control; the alias is refused as kept.
    assert_eq!(planned(&by_rule), ["/data/other/target"], "{by_rule:#?}");
    assert_eq!(
        refused_for(&by_rule, "/data/alias/target"),
        Some(&Refusal::Kept),
        "{by_rule:#?}"
    );
    assert_eq!(planned(&marked), ["/data/other/keep"], "{marked:#?}");
    assert_eq!(refused_for(&marked, "/data/alias"), Some(&Refusal::Kept));
    assert_eq!(refused_for(&marked, "/data/alias/keep"), Some(&Refusal::Kept));
    assert_eq!(planned(&labelled), ["/data/other/keep"], "{labelled:#?}");
    assert_eq!(refused_for(&labelled, "/data/alias/keep"), Some(&Refusal::Kept));
}

#[test]
fn a_second_path_to_the_home_folder_or_a_place_in_it_is_protected_on_the_disk() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/stuff/a", 10);
    fs.file("/home/u/.ssh/keys/id", 5);
    fs.file("/data/home/stuff/a", 10);
    fs.file("/data/home/.ssh/keys/id", 5);
    fs.file("/data/else/stuff/a", 10);
    let (dev, ino) = fs.identity("/home/u");
    fs.set_identity("/data/home", dev, ino, 2);
    let (dev, ino) = fs.identity("/home/u/.ssh");
    fs.set_identity("/data/home/.ssh", dev, ino, 2);

    let protected = platter_cleanup::Protected::on_disk(&env(), &fs).expect("a home is known");
    let refuses = |path: &str| protected.refuses_on_disk(&fs, path.as_bytes()).is_some();

    assert!(refuses("/home/u"), "the home folder by its own path");
    assert!(refuses("/data/home"), "the home folder by another path");
    assert!(
        refuses("/data/home/.ssh/keys"),
        "inside a protected place by another path"
    );
    assert!(
        !refuses("/data/home/stuff"),
        "an ordinary folder of the home is not protected"
    );
    assert!(!refuses("/data/else"), "control: another folder");
}

// A kept folder that is a mount the scan did not enter (`one_file_system`) is a mount node, not a folder
// node, and is kept all the same: by every kind of plan, and in every question of "is this kept?".

fn scan_on_one_filesystem(fs: &FakeFs, root: &str) -> Tree {
    let config = ScanConfig {
        now: Some(NOW),
        one_file_system: true,
        ..ScanConfig::default()
    };
    scan(fs, Path::new(root), &config, &CancelToken::new(), &|_| {}).expect("the fake scans")
}

fn label(tree: &Tree, path: &str) -> ModelLabel {
    ModelLabel {
        node: node(tree, path),
        verdict: ModelVerdict::Review,
        reason: String::new(),
    }
}

const ONE_BYTE: CandidateLimits = CandidateLimits {
    min_bytes: 1,
    max: 50,
};

/// `parent` holds `mount`, a mount point the scan skips, and a folder with bytes.
fn fs_with_mount(parent: &str, mount: &str) -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file(&format!("{parent}/data/blob"), 5000);
    fs.mount(mount);
    fs.file(&format!("{mount}/photo"), 100);
    fs
}

#[test]
fn a_user_kept_mount_keeps_the_folder_around_it_in_every_kind_of_plan() {
    let fs = fs_with_mount("/home/u/proj", "/home/u/proj/nas");
    let tree = scan_on_one_filesystem(&fs, "/home/u");
    assert_eq!(tree.kind(node(&tree, "proj/nas")), NodeKind::Mount, "control");
    let rules = rules(&[
        (UserVerdict::Keep, "/home/u/proj/nas"),
        (UserVerdict::Safe, "/home/u/proj"),
    ]);

    let marked = plan_marked(
        &tree,
        &env(),
        &rules,
        &[node(&tree, "proj")],
        &fs,
        &CleanupOptions::default(),
    );
    let labelled = plan_labelled(
        &tree,
        &env(),
        &rules,
        &[label(&tree, "proj")],
        &fs,
        &CleanupOptions::default(),
    );
    let by_rule = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["user-safe", "user-keep"],
        &fs,
        &CleanupOptions::default(),
    );

    for (what, plan) in [("marked", &marked), ("labelled", &labelled)] {
        assert!(plan.groups().is_empty(), "{what}: {plan:#?}");
        assert_eq!(refused_for(plan, "/home/u/proj"), Some(&Refusal::Kept), "{what}");
    }
    assert!(by_rule.groups().is_empty(), "{by_rule:#?}");
    assert_eq!(refused_for(&by_rule, "/home/u/proj/nas"), Some(&Refusal::Kept));
}

#[test]
fn a_user_kept_mount_is_a_user_keep_to_the_explainer_and_its_parent_is_no_label_candidate() {
    let fs = fs_with_mount("/home/u/proj", "/home/u/proj/nas");
    let tree = scan_on_one_filesystem(&fs, "/home/u");
    let rules = rules(&[
        (UserVerdict::Keep, "/home/u/proj/nas"),
        (UserVerdict::Safe, "/home/u/proj"),
    ]);
    let explainer = Explainer::new(&tree, &env(), &rules);

    let covering = explainer
        .covering(node(&tree, "proj/nas"))
        .expect("the mount is kept");

    assert_eq!(
        (covering.id, covering.verdict, covering.source),
        ("user-keep", Verdict::Keep, SuggestionSource::User)
    );
    assert!(
        explainer.holds(node(&tree, "proj")),
        "the folder around it holds a keep"
    );
    assert!(
        label_candidates(&tree, &env(), &RuleSet::builtin(), ONE_BYTE).contains(&node(&tree, "proj")),
        "control: without the rule the parent is a candidate"
    );
    assert!(!label_candidates(&tree, &env(), &rules, ONE_BYTE).contains(&node(&tree, "proj")));
}

#[test]
fn a_builtin_keep_on_a_mount_keeps_the_folder_around_it() {
    let steamapps = "/home/u/.local/share/Steam/steamapps";
    let fs = fs_with_mount(steamapps, &format!("{steamapps}/common"));
    let tree = scan_on_one_filesystem(&fs, "/home/u");
    let rules = rules(&[(UserVerdict::Safe, steamapps)]);
    let relative = steamapps.trim_start_matches("/home/u/");
    assert_eq!(
        tree.kind(node(&tree, &format!("{relative}/common"))),
        NodeKind::Mount,
        "control"
    );

    let explainer = Explainer::new(&tree, &env(), &rules);
    let covering = explainer
        .covering(node(&tree, &format!("{relative}/common")))
        .expect("the mount is kept");
    let marked = plan_marked(
        &tree,
        &env(),
        &rules,
        &[node(&tree, relative)],
        &fs,
        &CleanupOptions::default(),
    );
    let by_rule = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["user-safe"],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(
        (covering.id, covering.source),
        ("steam-library", SuggestionSource::Rule)
    );
    assert!(marked.groups().is_empty(), "{marked:#?}");
    assert_eq!(refused_for(&marked, steamapps), Some(&Refusal::Kept));
    assert!(by_rule.groups().is_empty(), "{by_rule:#?}");
    assert!(
        !label_candidates(&tree, &env(), &rules, ONE_BYTE).contains(&node(&tree, relative)),
        "the folder around it is no label candidate"
    );
}

#[test]
fn a_kept_folder_below_a_skipped_mount_still_keeps_the_target_around_it() {
    // The kept folder is not in the tree at all, only its path is known: the guard compares paths.
    let fs = fs_with_mount("/home/u/proj", "/home/u/proj/nas");
    let tree = scan_on_one_filesystem(&fs, "/home/u");
    let rules = rules(&[
        (UserVerdict::Keep, "/home/u/proj/nas/photos"),
        (UserVerdict::Safe, "/home/u/proj"),
    ]);

    let marked = plan_marked(
        &tree,
        &env(),
        &rules,
        &[node(&tree, "proj")],
        &fs,
        &CleanupOptions::default(),
    );
    let labelled = plan_labelled(
        &tree,
        &env(),
        &rules,
        &[label(&tree, "proj")],
        &fs,
        &CleanupOptions::default(),
    );
    let by_rule = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["user-safe"],
        &fs,
        &CleanupOptions::default(),
    );

    for (what, plan) in [("marked", &marked), ("labelled", &labelled), ("rule", &by_rule)] {
        assert!(plan.groups().is_empty(), "{what}: {plan:#?}");
        assert_eq!(refused_for(plan, "/home/u/proj"), Some(&Refusal::Kept), "{what}");
    }
}

#[test]
fn a_folder_beside_a_user_kept_mount_is_not_kept() {
    let fs = fs_with_mount("/home/u/proj", "/home/u/proj/nas");
    let tree = scan_on_one_filesystem(&fs, "/home/u");
    let rules = rules(&[(UserVerdict::Keep, "/home/u/proj/nas")]);

    let marked = plan_marked(
        &tree,
        &env(),
        &rules,
        &[node(&tree, "proj/data")],
        &fs,
        &CleanupOptions::default(),
    );

    let planned: Vec<String> = marked.targets().map(|target| target.display_path()).collect();
    assert_eq!(planned, ["/home/u/proj/data"], "{marked:#?}");
}

// Learning where the kept folders really are costs one look at the disk each, and a hung mount makes each
// look wait out the probe's own limit: the whole of it has one limit, shared by every plan of a request.

/// A probe where everything under `/dead` takes `delay` to answer and then fails, like a hung mount.
struct HungMount<'a> {
    fs: &'a FakeFs,
    delay: Duration,
    stalled: AtomicUsize,
}

impl HungMount<'_> {
    fn stall(&self, path: &[u8]) {
        if path.starts_with(b"/dead") {
            self.stalled.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.delay);
        }
    }
}

impl EntryProbe for HungMount<'_> {
    fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
        self.stall(path);
        if path.starts_with(b"/dead") {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }

        self.fs.probe(path)
    }

    fn resolve(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        self.stall(path);

        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

#[test]
fn keep_folders_on_a_hung_mount_cannot_hold_planning_and_the_rest_still_protect_by_path() {
    let fs = aliased_fs();
    let tree = scan_at(&fs, "/");
    let hung = HungMount {
        fs: &fs,
        delay: Duration::from_millis(100),
        stalled: AtomicUsize::new(0),
    };

    // One folder that is learned in time (by identity), 200 on the hung mount, and one that is only
    // known by its path because the time ran out before it.
    let mut user: Vec<(UserVerdict, String)> = vec![(UserVerdict::Keep, "/home/u/work/proj".to_owned())];
    user.extend((0..200).map(|index| (UserVerdict::Keep, format!("/dead/k{index}"))));
    user.push((UserVerdict::Keep, "/home/u/work/proj/keep".to_owned()));
    let user: Vec<(UserVerdict, &str)> = user
        .iter()
        .map(|(verdict, folder)| (*verdict, folder.as_str()))
        .collect();
    let rules = rules(&user);

    let started = Instant::now();
    let kept = KeptFolders::learn_within(&rules, &hung, Duration::from_millis(300));
    let learned_in = started.elapsed();
    let stalled = hung.stalled.load(Ordering::SeqCst);

    // 200 folders at 200 ms each would take 40 s. Four are looked at at a time and none is started after
    // 300 ms, so each of the four places makes at most three looks.
    assert!(learned_in < Duration::from_secs(2), "{learned_in:?}");
    assert!(stalled <= 12, "{stalled} looks at the hung mount");

    // Both plans of a request use the same learned folders and look at the hung mount no more.
    let by_rule = plan_cleanup_with(
        &tree,
        &env(),
        &rules,
        &["rust-target"],
        &hung,
        &kept,
        &CleanupOptions::default(),
    );
    let labels = [label(&tree, "data/alias/keep"), label(&tree, "data/other/keep")];
    let labelled = plan_labelled_with(
        &tree,
        &env(),
        &rules,
        &labels,
        &hung,
        &kept,
        &CleanupOptions::default(),
    );
    assert_eq!(
        hung.stalled.load(Ordering::SeqCst),
        stalled,
        "nothing is learned again"
    );

    let planned =
        |plan: &CleanupPlan| -> Vec<String> { plan.targets().map(|target| target.display_path()).collect() };
    // Learned in time: another path to the kept folder is refused.
    assert_eq!(
        refused_for(&by_rule, "/data/alias/target"),
        Some(&Refusal::Kept),
        "{by_rule:#?}"
    );
    assert_eq!(planned(&by_rule), ["/data/other/target"]);
    assert_eq!(
        refused_for(&labelled, "/data/alias/keep"),
        Some(&Refusal::Kept),
        "{labelled:#?}"
    );
    assert_eq!(planned(&labelled), ["/data/other/keep"]);

    // Known by path alone: the folder itself is still refused, and what is around it.
    let marked = plan_labelled_with(
        &tree,
        &env(),
        &rules,
        &[label(&tree, "home/u/work/proj/keep")],
        &hung,
        &kept,
        &CleanupOptions::default(),
    );
    assert!(marked.groups().is_empty(), "{marked:#?}");
    assert_eq!(
        refused_for(&marked, "/home/u/work/proj/keep"),
        Some(&Refusal::Kept)
    );
}

// Links and slow mounts among the kept folders and the protected places.

/// A probe with links (`resolve` follows them), and slow answers for what is under `/net`.
struct LinkedProbe<'a> {
    fs: &'a FakeFs,
    links: Vec<(&'static str, &'static str)>,
    /// How long a look at a path under `/net` takes before it fails, like a hung mount.
    net_delay: Duration,
    resolves: AtomicUsize,
}

impl<'a> LinkedProbe<'a> {
    fn new(fs: &'a FakeFs, links: &[(&'static str, &'static str)]) -> Self {
        Self {
            fs,
            links: links.to_vec(),
            net_delay: Duration::ZERO,
            resolves: AtomicUsize::new(0),
        }
    }

    fn stall(&self, path: &[u8]) -> io::Result<()> {
        if path.starts_with(b"/net/") {
            std::thread::sleep(self.net_delay);
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }

        Ok(())
    }
}

impl EntryProbe for LinkedProbe<'_> {
    fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
        self.stall(path)?;

        self.fs.probe(path)
    }

    fn resolve(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        self.resolves.fetch_add(1, Ordering::SeqCst);
        self.stall(path)?;

        let Some((_, to)) = self.links.iter().find(|(from, _)| from.as_bytes() == path) else {
            return Ok(path.to_vec());
        };
        // A link that leads to a place that does not answer takes as long to resolve as that place, and
        // a link that is not absolute (a loop, here) cannot be resolved.
        self.stall(to.as_bytes())?;
        if !to.starts_with('/') {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }

        Ok(to.as_bytes().to_vec())
    }

    fn read_link(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        self.stall(path)?;

        match self.links.iter().find(|(from, _)| from.as_bytes() == path) {
            Some((_, to)) => Ok(to.as_bytes().to_vec()),
            None => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
    }
}

fn linked_fs() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/project/x/blob", 5000);
    fs.file("/home/u/real/y/blob", 5000);
    fs.symlink("/home/u/link", 10);
    fs
}

fn planned(plan: &CleanupPlan) -> Vec<String> {
    plan.targets().map(|target| target.display_path()).collect()
}

#[test]
fn a_keep_link_to_the_top_folder_or_above_the_home_does_not_block_every_plan() {
    let fs = linked_fs();
    let tree = scan_at(&fs, "/home/u");
    let rules = rules(&[
        (UserVerdict::Keep, "/home/u/link"),
        (UserVerdict::Safe, "/home/u/project/x"),
    ]);

    // Where the link leads holds the scan, the home folder or everything.
    for leads_to in ["/", "/home", "/home/u"] {
        let probe = LinkedProbe::new(&fs, &[("/home/u/link", leads_to)]);

        let by_rule = plan_cleanup(
            &tree,
            &env(),
            &rules,
            &["user-safe"],
            &probe,
            &CleanupOptions::default(),
        );
        let marked = plan_marked(
            &tree,
            &env(),
            &rules,
            &[node(&tree, "project/x")],
            &probe,
            &CleanupOptions::default(),
        );

        assert_eq!(
            planned(&by_rule),
            ["/home/u/project/x"],
            "{leads_to}: {by_rule:#?}"
        );
        assert_eq!(planned(&marked), ["/home/u/project/x"], "{leads_to}: {marked:#?}");
    }
}

#[test]
fn a_keep_link_to_the_top_folder_leaves_no_real_folder_and_the_link_itself_stays_kept() {
    let fs = linked_fs();
    let tree = scan_at(&fs, "/home/u");
    let rules = rules(&[(UserVerdict::Keep, "/home/u/link")]);
    let probe = LinkedProbe::new(&fs, &[("/home/u/link", "/")]);

    let kept = KeptFolders::learn(&rules, &probe);
    let marked = plan_marked(
        &tree,
        &env(),
        &rules,
        &[node(&tree, "project/x"), node(&tree, "link")],
        &probe,
        &CleanupOptions::default(),
    );

    assert!(kept.resolved_folders().is_empty());
    assert_eq!(planned(&marked), ["/home/u/project/x"], "{marked:#?}");
    assert_eq!(refused_for(&marked, "/home/u/link"), Some(&Refusal::Kept));
}

#[test]
fn a_keep_that_is_a_bind_mount_of_the_home_folder_over_protects_and_refuses_every_plan_as_kept() {
    let mut fs = linked_fs();
    fs.dir("/mnt/home");
    let (dev, ino) = fs.identity("/home/u");
    fs.set_identity("/mnt/home", dev, ino, 2);
    let tree = scan_at(&fs, "/home/u");
    let rules = rules(&[
        (UserVerdict::Keep, "/mnt/home"),
        (UserVerdict::Safe, "/home/u/project/x"),
    ]);

    let plan = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["user-safe"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert_eq!(
        refused_for(&plan, "/home/u/project/x"),
        Some(&Refusal::Kept),
        "{plan:#?}"
    );
}

#[test]
fn a_keep_link_to_a_real_folder_keeps_that_folder_in_every_kind_of_plan() {
    let fs = linked_fs();
    let tree = scan_at(&fs, "/home/u");
    let rules = rules(&[
        (UserVerdict::Keep, "/home/u/link"),
        (UserVerdict::Safe, "/home/u/real/y"),
    ]);
    let probe = LinkedProbe::new(&fs, &[("/home/u/link", "/home/u/real")]);

    let by_rule = plan_cleanup(
        &tree,
        &env(),
        &rules,
        &["user-safe"],
        &probe,
        &CleanupOptions::default(),
    );
    let marked = plan_marked(
        &tree,
        &env(),
        &rules,
        &[node(&tree, "real/y")],
        &probe,
        &CleanupOptions::default(),
    );

    assert_eq!(
        refused_for(&by_rule, "/home/u/real/y"),
        Some(&Refusal::Kept),
        "{by_rule:#?}"
    );
    assert_eq!(
        refused_for(&marked, "/home/u/real/y"),
        Some(&Refusal::Kept),
        "{marked:#?}"
    );
}

/// A project that the user keeps through `/home/u/link`, with Python caches in it and in a folder of it.
fn kept_through_link_fs() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/data/proj/__pycache__/a.pyc", 500);
    fs.file("/data/proj/sub/__pycache__/b.pyc", 500);
    fs.file("/data/proj/sub/main.py", 5);
    fs.file("/home/u/notes/n.txt", 5);
    fs.symlink("/home/u/link", 10);
    fs
}

/// The Python caches of `kept_through_link_fs` as the scan of `root` sees them.
fn caches(root: &str) -> Vec<&'static str> {
    match root {
        "/data" => vec!["proj/__pycache__", "proj/sub/__pycache__"],
        "/data/proj" => vec!["__pycache__", "sub/__pycache__"],
        _ => vec!["__pycache__"],
    }
}

fn all_refused_as_kept(plan: &CleanupPlan, expected: usize, what: &str) {
    assert!(plan.groups().is_empty(), "{what}: {plan:#?}");
    assert_eq!(plan.refused.len(), expected, "{what}: {plan:#?}");
    assert!(
        plan.refused.iter().all(|refused| refused.reason == Refusal::Kept),
        "{what}: {plan:#?}"
    );
}

#[test]
fn a_keep_link_to_the_scan_root_or_a_folder_holding_it_keeps_the_scan_in_every_plan_and_lists_nothing() {
    let fs = kept_through_link_fs();
    let links = [("/home/u/link", "/data/proj")];
    let probe = LinkedProbe::new(&fs, &links);
    let kept = rules(&[(UserVerdict::Keep, "/home/u/link")]);

    for root in ["/data/proj", "/data/proj/sub"] {
        let tree = scan_at(&fs, root);
        let nodes: Vec<NodeId> = caches(root).iter().map(|path| node(&tree, path)).collect();
        let labels: Vec<ModelLabel> = caches(root).iter().map(|path| label(&tree, path)).collect();
        let count = nodes.len();

        let by_rule = plan_cleanup(
            &tree,
            &env(),
            &kept,
            &["python-cache"],
            &probe,
            &CleanupOptions::default(),
        );
        let marked = plan_marked(&tree, &env(), &kept, &nodes, &probe, &CleanupOptions::default());
        let labelled = plan_labelled(&tree, &env(), &kept, &labels, &probe, &CleanupOptions::default());
        let listed = evaluate(
            &tree,
            &env(),
            &with_real_keeps(kept.clone(), env().home.as_deref(), &probe),
        );

        all_refused_as_kept(&by_rule, count, &format!("{root} by rule"));
        all_refused_as_kept(&marked, count, &format!("{root} marked"));
        all_refused_as_kept(&labelled, count, &format!("{root} labelled"));
        // The kept folder itself, as a scan root, is listed as kept (as a literal keep there is); nothing
        // in it is suggested for cleaning.
        let cleanable = listed.suggestions.iter().any(|one| one.verdict != Verdict::Keep);
        assert!(!cleanable, "{root}: {listed:#?}");
        assert_eq!(
            listed.totals.safe + listed.totals.review,
            0,
            "{root}: {listed:#?}"
        );
        if root != "/data/proj" {
            assert!(listed.suggestions.is_empty(), "{root}: {listed:#?}");
        }

        // The same scan without the keep is the control: the caches are there to clean.
        let control = plan_cleanup(
            &tree,
            &env(),
            &RuleSet::builtin(),
            &["python-cache"],
            &probe,
            &CleanupOptions::default(),
        );
        assert_eq!(
            planned(&control).len(),
            count,
            "{root} without the keep: {control:#?}"
        );
    }
}

#[test]
fn a_keep_link_to_a_folder_below_the_scan_root_still_refuses_what_lies_in_it() {
    let fs = kept_through_link_fs();
    let links = [("/home/u/link", "/data/proj")];
    let probe = LinkedProbe::new(&fs, &links);
    let kept = rules(&[(UserVerdict::Keep, "/home/u/link")]);
    let tree = scan_at(&fs, "/data");

    let by_rule = plan_cleanup(
        &tree,
        &env(),
        &kept,
        &["python-cache"],
        &probe,
        &CleanupOptions::default(),
    );

    all_refused_as_kept(&by_rule, 2, "/data by rule");
}

#[test]
fn a_keep_link_to_the_home_folder_or_above_protects_only_the_link_itself() {
    let fs = kept_through_link_fs();
    let tree = scan_at(&fs, "/data/proj");
    let kept = rules(&[(UserVerdict::Keep, "/home/u/link")]);

    for leads_to in ["/", "/home", "/home/u"] {
        let probe = LinkedProbe::new(&fs, &[("/home/u/link", leads_to)]);

        let by_rule = plan_cleanup(
            &tree,
            &env(),
            &kept,
            &["python-cache"],
            &probe,
            &CleanupOptions::default(),
        );
        let listed = evaluate(
            &tree,
            &env(),
            &with_real_keeps(kept.clone(), env().home.as_deref(), &probe),
        );

        assert_eq!(planned(&by_rule).len(), 2, "{leads_to}: {by_rule:#?}");
        assert!(!listed.suggestions.is_empty(), "{leads_to}: {listed:#?}");
    }
}

#[test]
fn a_hung_keep_first_does_not_cost_a_later_keep_its_real_location() {
    let fs = linked_fs();
    let tree = scan_at(&fs, "/home/u");
    let mut probe = LinkedProbe::new(&fs, &[("/home/u/link", "/home/u/real")]);
    probe.net_delay = Duration::from_millis(600);
    let rules = rules(&[
        (UserVerdict::Keep, "/net/share"),
        (UserVerdict::Keep, "/home/u/link"),
        (UserVerdict::Safe, "/home/u/real/y"),
    ]);

    let started = Instant::now();
    let kept = KeptFolders::learn_within(&rules, &probe, Duration::from_millis(400));
    let learned_in = started.elapsed();
    let plan = plan_cleanup_with(
        &tree,
        &env(),
        &rules,
        &["user-safe"],
        &probe,
        &kept,
        &CleanupOptions::default(),
    );

    assert!(learned_in < Duration::from_millis(1500), "{learned_in:?}");
    assert_eq!(kept.resolved_folders(), [PathBuf::from("/home/u/real")]);
    assert_eq!(
        refused_for(&plan, "/home/u/real/y"),
        Some(&Refusal::Kept),
        "{plan:#?}"
    );
    assert!(plan.groups().is_empty(), "{plan:#?}");
}

/// `count` keeps on a mount that answers after 100 ms with a failure, then `last`.
fn hung_keeps_then(count: usize, last: &str) -> Vec<(UserVerdict, String)> {
    let mut user: Vec<(UserVerdict, String)> = (0..count)
        .map(|index| (UserVerdict::Keep, format!("/net/k{index}")))
        .collect();
    user.push((UserVerdict::Keep, last.to_owned()));

    user
}

fn learned_within(
    user: &[(UserVerdict, String)],
    probe: &LinkedProbe,
    budget: Duration,
) -> (RuleSet, KeptFolders, Duration) {
    let user: Vec<(UserVerdict, &str)> = user
        .iter()
        .map(|(verdict, folder)| (*verdict, folder.as_str()))
        .collect();
    let rules = rules(&user);

    let started = Instant::now();
    let kept = KeptFolders::learn_within(&rules, probe, budget);

    (rules, kept, started.elapsed())
}

#[test]
fn a_link_keep_behind_many_hung_keeps_is_still_read_by_text_and_protects_where_it_leads() {
    let fs = kept_through_link_fs();
    let mut probe = LinkedProbe::new(&fs, &[("/home/u/link", "/data/proj")]);
    probe.net_delay = Duration::from_millis(150);
    let user = hung_keeps_then(8, "/home/u/link");

    // Eight hung keeps (four would have held every worker of the old learning) ahead of the link.
    let (rules, kept, learned_in) = learned_within(&user, &probe, Duration::from_millis(250));

    assert!(learned_in < Duration::from_secs(2), "{learned_in:?}");
    assert_eq!(kept.resolved_folders(), [PathBuf::from("/data/proj")]);
    assert!(!kept.unresolved());
    for root in ["/data", "/data/proj"] {
        let tree = scan_at(&fs, root);

        let plan = plan_cleanup_with(
            &tree,
            &env(),
            &rules,
            &["python-cache"],
            &probe,
            &kept,
            &CleanupOptions::default(),
        );

        all_refused_as_kept(&plan, caches(root).len(), root);
    }
}

#[test]
fn a_keep_that_the_limit_ran_out_before_could_be_a_link_so_nothing_is_planned() {
    let fs = kept_through_link_fs();
    let mut probe = LinkedProbe::new(&fs, &[("/home/u/link", "/data/proj")]);
    probe.net_delay = Duration::from_millis(100);
    // So many hung keeps that the link behind them is never reached: where it leads is not known.
    let user = hung_keeps_then(100, "/home/u/link");

    let (rules, kept, learned_in) = learned_within(&user, &probe, Duration::from_millis(250));

    // 100 hung folders at 100 ms each would take 10 s, sixteen at a time over 600 ms.
    assert!(learned_in < Duration::from_secs(2), "{learned_in:?}");
    assert!(kept.unresolved());
    for root in ["/data", "/data/proj"] {
        let tree = scan_at(&fs, root);
        let labels: Vec<ModelLabel> = caches(root).iter().map(|path| label(&tree, path)).collect();

        let by_rule = plan_cleanup_with(
            &tree,
            &env(),
            &rules,
            &["python-cache"],
            &probe,
            &kept,
            &CleanupOptions::default(),
        );
        let labelled = plan_labelled_with(
            &tree,
            &env(),
            &rules,
            &labels,
            &probe,
            &kept,
            &CleanupOptions::default(),
        );

        for (plan, what) in [(&by_rule, "by rule"), (&labelled, "labelled")] {
            assert!(plan.groups().is_empty(), "{root} {what}: {plan:#?}");
            assert_eq!(plan.refused.len(), labels.len(), "{root} {what}: {plan:#?}");
            assert!(
                plan.refused
                    .iter()
                    .all(|refused| refused.reason == Refusal::KeptUnresolved),
                "{root} {what}: {plan:#?}"
            );
        }
    }
}

#[test]
fn a_link_keep_whose_way_cannot_be_followed_in_time_refuses_every_plan() {
    let fs = kept_through_link_fs();
    let mut probe = LinkedProbe::new(&fs, &[("/home/u/link", "/net/share/deep")]);
    probe.net_delay = Duration::from_millis(50);
    let rules = rules(&[(UserVerdict::Keep, "/home/u/link")]);
    let tree = scan_at(&fs, "/data");

    let kept = KeptFolders::learn(&rules, &probe);
    let plan = plan_cleanup_with(
        &tree,
        &env(),
        &rules,
        &["python-cache"],
        &probe,
        &kept,
        &CleanupOptions::default(),
    );

    // The link leads onto a mount that does not answer: where it really is cannot be told.
    assert!(kept.unresolved());
    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert_eq!(plan.refused.len(), 2, "{plan:#?}");
    assert!(
        plan.refused
            .iter()
            .all(|refused| refused.reason == Refusal::KeptUnresolved),
        "{plan:#?}"
    );
}

#[test]
fn a_hung_keep_that_is_not_a_link_does_not_stop_planning() {
    let fs = kept_through_link_fs();
    let mut probe = LinkedProbe::new(&fs, &[]);
    probe.net_delay = Duration::from_millis(50);
    let rules = rules(&[
        (UserVerdict::Keep, "/net/share"),
        (UserVerdict::Keep, "/data/proj/sub"),
    ]);
    let tree = scan_at(&fs, "/data");

    let kept = KeptFolders::learn(&rules, &probe);
    let plan = plan_cleanup_with(
        &tree,
        &env(),
        &rules,
        &["python-cache"],
        &probe,
        &kept,
        &CleanupOptions::default(),
    );

    assert!(!kept.unresolved());
    assert_eq!(planned(&plan), ["/data/proj/__pycache__"], "{plan:#?}");
}

#[test]
fn links_are_followed_by_text_through_relative_and_chained_links_and_links_on_the_way() {
    let fs = kept_through_link_fs();
    let links = [
        ("/home/u/a", "/home/u/b"),
        ("/home/u/b", "../../data/./proj/../proj"),
        ("/home/u/rel", "../../data"),
    ];
    let probe = LinkedProbe::new(&fs, &links);

    // Each keep is reached by links that are not resolved by the probe's own `resolve` (it follows one).
    for (keep, real) in [
        ("/home/u/a", "/data/proj"),
        ("/home/u/rel/proj/sub", "/data/proj/sub"),
        ("/home/u/a/sub", "/data/proj/sub"),
    ] {
        let rules = rules(&[(UserVerdict::Keep, keep)]);

        let kept = KeptFolders::learn(&rules, &probe);

        assert_eq!(kept.resolved_folders(), [PathBuf::from(real)], "{keep}");
        assert!(!kept.unresolved(), "{keep}");
    }
}

#[test]
fn a_link_that_leads_nowhere_in_a_few_hops_is_not_resolved_and_refuses_every_plan() {
    let fs = kept_through_link_fs();
    let probe = LinkedProbe::new(&fs, &[("/home/u/loop", "loop")]);
    let rules = rules(&[(UserVerdict::Keep, "/home/u/loop")]);
    let tree = scan_at(&fs, "/data");

    let kept = KeptFolders::learn(&rules, &probe);
    let plan = plan_cleanup_with(
        &tree,
        &env(),
        &rules,
        &["python-cache"],
        &probe,
        &kept,
        &CleanupOptions::default(),
    );

    assert!(kept.unresolved());
    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert!(
        plan.refused
            .iter()
            .all(|refused| refused.reason == Refusal::KeptUnresolved),
        "{plan:#?}"
    );
}

#[test]
fn without_hung_keeps_the_same_link_keep_refuses_as_kept() {
    let fs = kept_through_link_fs();
    let probe = LinkedProbe::new(&fs, &[("/home/u/link", "/data/proj")]);
    let user = hung_keeps_then(0, "/home/u/link");
    let (rules, kept, _) = learned_within(&user, &probe, KEPT_LEARNING_BUDGET);

    for root in ["/data", "/data/proj"] {
        let tree = scan_at(&fs, root);

        let plan = plan_cleanup_with(
            &tree,
            &env(),
            &rules,
            &["python-cache"],
            &probe,
            &kept,
            &CleanupOptions::default(),
        );

        all_refused_as_kept(&plan, caches(root).len(), root);
    }
}

#[test]
fn protected_places_share_one_limit_and_the_rest_are_protected_by_path() {
    let fs = linked_fs();
    let slow = SlowProbe {
        fs: &fs,
        delay: Duration::from_millis(100),
    };

    let started = Instant::now();
    let protected =
        Protected::on_disk_within(&env(), &slow, Duration::from_millis(250)).expect("a home is known");
    let took = started.elapsed();

    // About 45 places at two looks of 100 ms each would take 9 s, four at a time over 2 s.
    assert!(took < Duration::from_millis(1200), "{took:?}");
    assert!(protected.refuses(b"/etc/passwd").is_some());
    assert!(protected.refuses(b"/home/u/.ssh/id").is_some());
    assert!(protected.refuses(b"/home/u/stuff").is_none());
}

/// A probe where every look takes `delay`.
struct SlowProbe<'a> {
    fs: &'a FakeFs,
    delay: Duration,
}

impl EntryProbe for SlowProbe<'_> {
    fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
        std::thread::sleep(self.delay);

        self.fs.probe(path)
    }

    fn resolve(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        std::thread::sleep(self.delay);

        Ok(path.to_vec())
    }
}

#[test]
fn the_plans_of_one_request_look_the_protected_places_up_once() {
    let fs = linked_fs();
    let tree = scan_at(&fs, "/home/u");
    let rules = rules(&[(UserVerdict::Safe, "/home/u/real/y")]);
    let probe = LinkedProbe::new(&fs, &[]);
    let kept = KeptFolders::learn(&rules, &probe);

    plan_cleanup_with(
        &tree,
        &env(),
        &rules,
        &["user-safe"],
        &probe,
        &kept,
        &CleanupOptions::default(),
    );
    let after_first = probe.resolves.load(Ordering::SeqCst);
    plan_labelled_with(
        &tree,
        &env(),
        &rules,
        &[label(&tree, "real/y")],
        &probe,
        &kept,
        &CleanupOptions::default(),
    );

    assert!(
        after_first > 10,
        "the protected places were looked up: {after_first}"
    );
    assert_eq!(probe.resolves.load(Ordering::SeqCst), after_first);
}
