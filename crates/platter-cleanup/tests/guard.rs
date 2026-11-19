//! Every refusal of the guard, on the in-memory filesystem. Planning never changes anything.

use std::path::{Path, PathBuf};

use platter_cleanup::{CleanupAction, CleanupOptions, CleanupPlan, Refusal, fenced, plan_cleanup};
use platter_core::dto::Verdict;
use platter_core::testing::{FakeFs, NOW};
use platter_core::{
    CancelToken, Environment, ModelLabel, ModelVerdict, NodeId, Os, ScanConfig, Tree, UserDir, scan,
};
use platter_rules::{Matcher, Rule, RuleSet, evaluate_with};

const DAY: i64 = 86_400;

fn env() -> Environment {
    Environment {
        home: Some(PathBuf::from("/home/u")),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::Linux,
        system_root: None,
    }
}

fn scan_at(fs: &FakeFs, root: &str) -> Tree {
    let config = ScanConfig {
        now: Some(NOW),
        ..ScanConfig::default()
    };
    scan(fs, Path::new(root), &config, &CancelToken::new(), &|_| {}).expect("the fake scans")
}

fn aged(fs: &mut FakeFs, path: &str, size: u64, days: i64) {
    fs.file(path, size).set_mtime(path, NOW - days * DAY);
}

/// An idle Rust project, an idle npm project, a cache, an old download and a pacman cache.
fn laptop() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/code/rs/Cargo.toml", 1);
    aged(&mut fs, "/home/u/code/rs/target/debug/x", 1000, 90);
    aged(&mut fs, "/home/u/code/js/package.json", 1, 90);
    aged(&mut fs, "/home/u/code/js/package-lock.json", 1, 90);
    aged(&mut fs, "/home/u/code/js/node_modules/dep/i.js", 500, 90);
    fs.file("/home/u/.cache/app/blob", 300);
    aged(&mut fs, "/home/u/Downloads/old.iso", 200, 800);
    fs.file("/var/cache/pacman/pkg/a.pkg", 100);
    fs
}

fn reasons(plan: &CleanupPlan) -> Vec<(&str, &str, &Refusal)> {
    plan.refused
        .iter()
        .map(|r| (r.rule_id, r.path.as_str(), &r.reason))
        .collect()
}

fn paths(plan: &CleanupPlan) -> Vec<String> {
    plan.targets().map(|t| t.display_path()).collect()
}

fn one_rule(id: &'static str, matcher: Matcher) -> RuleSet {
    RuleSet::new(vec![Rule::new(id, id, Verdict::Safe, matcher, "r")])
}

#[test]
fn a_dry_run_plans_every_selected_match_with_its_action() {
    let fs = laptop();
    let tree = scan_at(&fs, "/");
    let selected = [
        "rust-target",
        "node-modules",
        "app-cache",
        "old-downloads",
        "pacman-cache",
    ];

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &selected,
        &fs,
        &CleanupOptions::default(),
    );

    let groups: Vec<_> = plan
        .groups()
        .iter()
        .map(|g| (g.rule_id(), g.action(), g.allocated()))
        .collect();
    assert_eq!(groups.len(), 5, "{plan:#?}");
    assert!(groups.contains(&("rust-target", CleanupAction::Delete, 1000)));
    assert!(groups.contains(&("node-modules", CleanupAction::Delete, 500)));
    assert!(groups.contains(&("app-cache", CleanupAction::EmptyFolder, 300)));
    assert!(groups.contains(&("old-downloads", CleanupAction::Trash, 200)));
    assert!(matches!(
        groups.iter().find(|g| g.0 == "pacman-cache").unwrap().1,
        CleanupAction::RunTool(tool) if tool.program() == "paccache" && tool.needs_elevation()
    ));
    assert_eq!(plan.allocated, 1000 + 500 + 300 + 200 + 100);
    assert!(plan.refused.is_empty(), "{plan:#?}");

    let target = plan
        .targets()
        .find(|t| t.display_path() == "/home/u/code/rs/target")
        .unwrap();
    assert!(target.is_dir());
    assert_ne!(
        target.identity(),
        (0, 0),
        "identity comes from the disk at plan time"
    );
}

#[test]
fn only_selected_rules_are_planned_and_unknown_ids_are_reported() {
    let fs = laptop();
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target", "no-such-rule", "apt-cache"],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(paths(&plan), ["/home/u/code/rs/target"]);
    assert_eq!(plan.not_found, ["no-such-rule", "apt-cache"]);
}

#[test]
fn keep_and_unsupported_rules_are_refused() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/Documents/a", 10).file("/.snapshots/1/f", 10);
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["documents", "btrfs-snapshots"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty());
    assert_eq!(
        reasons(&plan),
        [
            ("btrfs-snapshots", "/.snapshots", &Refusal::NotCleaned),
            ("documents", "/home/u/Documents", &Refusal::Kept),
        ]
    );
}

#[test]
fn without_a_home_directory_nothing_is_planned() {
    let fs = laptop();
    let tree = scan_at(&fs, "/");
    let no_home = Environment { home: None, ..env() };

    let plan = plan_cleanup(
        &tree,
        &no_home,
        &RuleSet::builtin(),
        &["rust-target", "pacman-cache"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty());
    assert!(!plan.refused.is_empty());
    assert!(plan.refused.iter().all(|r| r.reason == Refusal::HomeUnknown));
}

#[test]
fn the_scan_root_itself_is_never_a_target() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/a/b", 10);
    let tree = scan_at(&fs, "/home/u/.cache");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["app-cache"],
        &fs,
        &CleanupOptions::default(),
    );

    // No rule matches the scan root, so there is nothing for the guard to refuse and nothing planned.
    assert!(plan.groups().is_empty());
    assert!(plan.refused.is_empty(), "{:?}", plan.refused);
    assert_eq!(plan.not_found, ["app-cache"]);
}

#[test]
fn protected_places_are_refused_whatever_a_rule_says() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.ssh/id_ed25519", 1)
        .file("/home/u/.config/app/settings", 1)
        .file("/etc/x/conf", 1)
        .file("/home/u/Dokumenty/thesis.odt", 1)
        .file("/home/u/notes.txt", 1)
        .file("/var/lib/x", 1);
    let tree = scan_at(&fs, "/");
    let env = Environment {
        user_dirs: vec![(UserDir::Documents, PathBuf::from("/home/u/Dokumenty"))],
        ..env()
    };

    let cases = [
        ("app-cache", Matcher::Home(".ssh"), "/home/u/.ssh"),
        ("app-cache", Matcher::Home(".config/app"), "/home/u/.config"),
        ("coredumps", Matcher::Fixed("/etc/x"), "/etc"),
        ("trash", Matcher::Fixed("/home/u"), "/home/u"),
        ("trash", Matcher::Fixed("/home"), "/home"),
        ("trash", Matcher::Fixed("/var/lib"), "/var/lib"),
        ("npm-cache", Matcher::Home("Dokumenty"), "/home/u/Dokumenty"),
    ];
    for (id, matcher, place) in cases {
        let plan = plan_cleanup(
            &tree,
            &env,
            &one_rule(id, matcher),
            &[id],
            &fs,
            &CleanupOptions::default(),
        );

        assert!(plan.groups().is_empty(), "{matcher:?}: {plan:#?}");
        assert_eq!(
            plan.refused[0].reason,
            Refusal::Protected(place.to_owned()),
            "{matcher:?}"
        );
    }
}

#[test]
fn a_target_swapped_for_a_link_after_the_scan_is_refused() {
    let mut fs = laptop();
    let tree = scan_at(&fs, "/");
    fs.swap_for_symlink("/home/u/code/rs/target");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(
        reasons(&plan),
        [("rust-target", "/home/u/code/rs/target", &Refusal::Symlink)]
    );
}

#[test]
fn a_folder_on_the_way_swapped_for_a_link_is_refused() {
    let mut fs = laptop();
    let tree = scan_at(&fs, "/");
    fs.swap_for_symlink("/home/u/code");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty());
    assert!(matches!(&plan.refused[0].reason, Refusal::Changed(what) if what.contains("link")));
}

#[test]
fn a_target_that_is_gone_or_changed_kind_is_refused() {
    let mut fs = laptop();
    let tree = scan_at(&fs, "/");
    fs.remove("/home/u/code/rs/target");
    fs.remove("/home/u/code/js/node_modules")
        .file("/home/u/code/js/node_modules", 5);

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target", "node-modules"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty());
    assert_eq!(
        reasons(&plan),
        [
            (
                "rust-target",
                "/home/u/code/rs/target",
                &Refusal::Changed("it is gone".to_owned())
            ),
            (
                "node-modules",
                "/home/u/code/js/node_modules",
                &Refusal::Changed("it is now a file".to_owned())
            ),
        ]
    );
}

#[test]
fn a_mount_that_is_the_target_itself_is_refused() {
    let mut fs = FakeFs::new();
    fs.same_filesystem_mount("/home/u/a/target");
    fs.file("/home/u/a/Cargo.toml", 1);
    aged(&mut fs, "/home/u/a/target/x", 10, 90);
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert_eq!(plan.refused.len(), 1);
    assert!(plan.refused.iter().all(|r| r.reason == Refusal::Mount));
}

#[test]
fn a_target_below_a_mount_the_scan_went_into_is_planned_with_that_mount_recorded() {
    // `/home` is a subvolume of the same disk: the scan reads it as any other folder.
    let mut fs = FakeFs::new();
    fs.same_filesystem_mount("/home");
    fs.file("/home/u/a/Cargo.toml", 1);
    aged(&mut fs, "/home/u/a/target/x", 10, 90);
    fs.file("/home/u/.cache/app/blob", 300);
    let tree = scan_at(&fs, "/");
    let (dev, ino) = fs.identity("/home");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target", "app-cache"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.refused.is_empty(), "{:?}", reasons(&plan));
    assert_eq!(paths(&plan), ["/home/u/a/target", "/home/u/.cache"]);
    for target in plan.targets() {
        let mounts = target.entered_mounts();

        assert_eq!(mounts.len(), 1, "{mounts:?}");
        assert_eq!((mounts[0].depth, mounts[0].dev, mounts[0].ino), (0, dev, ino));
    }
}

#[test]
fn a_mount_deeper_on_the_way_is_recorded_at_its_own_place() {
    let mut fs = FakeFs::new();
    fs.same_filesystem_mount("/home/u/vol");
    fs.file("/home/u/vol/b/Cargo.toml", 1);
    aged(&mut fs, "/home/u/vol/b/target/x", 10, 90);
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(paths(&plan), ["/home/u/vol/b/target"]);
    let mounts = plan.targets().next().unwrap().entered_mounts();
    assert_eq!(mounts.len(), 1, "{mounts:?}");
    assert_eq!(mounts[0].depth, 2, "home, u, vol");
}

/// A project with a `target` folder below `/home/u/vol`, for the tests of what may be a mount on the way.
fn project_below_vol() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/vol/b/Cargo.toml", 1);
    aged(&mut fs, "/home/u/vol/b/target/x", 10, 90);
    fs
}

/// Plans the `rust-target` rule on `tree` against what the disk is now (`fs`), and says whether the one
/// target is refused as a mount.
fn refused_as_a_mount(tree: &Tree, fs: &FakeFs) -> bool {
    let plan = plan_cleanup(
        tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert_eq!(plan.refused.len(), 1, "{:?}", reasons(&plan));
    plan.refused[0].reason == Refusal::Mount
}

#[test]
fn a_folder_that_became_a_mount_after_the_scan_is_refused() {
    // A backup disk (or an sshfs, or a tmpfs) mounted over a folder the scan read as an ordinary one.
    let mut fs = project_below_vol();
    let tree = scan_at(&fs, "/");
    fs.mount("/home/u/vol");

    assert!(refused_as_a_mount(&tree, &fs));
}

#[test]
fn a_mount_swapped_for_another_one_after_the_scan_is_refused() {
    let mut fs = project_below_vol();
    fs.same_filesystem_mount("/home/u/vol");
    let tree = scan_at(&fs, "/");
    let (_, ino) = fs.identity("/home/u/vol");

    // Another device at the same place, and another folder of the same device.
    fs.set_identity("/home/u/vol", 77, ino, 1);
    assert!(refused_as_a_mount(&tree, &fs), "another device");
    fs.set_identity("/home/u/vol", 2, ino + 1000, 1);
    assert!(refused_as_a_mount(&tree, &fs), "another root");
}

#[test]
fn a_mount_of_another_filesystem_the_scan_entered_is_not_acted_inside() {
    // Without `one_file_system` the scan goes into other disks too (a USB stick, a partition).
    let mut fs = project_below_vol();
    fs.mount("/home/u/vol");
    let tree = scan_at(&fs, "/");
    assert!(
        tree.entered_mount(tree_node(&tree, "/home/u/vol")).is_some(),
        "the scan went into it"
    );

    assert!(refused_as_a_mount(&tree, &fs));
}

#[test]
fn a_bind_mount_of_a_folder_from_elsewhere_on_the_same_disk_is_not_acted_inside() {
    // Entered with `one_file_system` (same disk, counted once), but what lies below is another folder's
    // data, outside the scan root.
    let mut fs = project_below_vol();
    fs.same_filesystem_bind_mount("/home/u/vol");
    let config = ScanConfig {
        now: Some(NOW),
        one_file_system: true,
        ..ScanConfig::default()
    };
    let tree = scan(&fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).unwrap();

    assert!(refused_as_a_mount(&tree, &fs));
}

/// The node of the folder at `path`.
fn tree_node(tree: &Tree, path: &str) -> platter_core::NodeId {
    (0..tree.len() as u32)
        .map(platter_core::NodeId)
        .find(|id| tree.path_bytes(*id) == path.as_bytes())
        .expect("the folder is in the tree")
}

/// What the `rust-target` rule makes of `fs` scanned from `/`: the one refusal, or the planned paths.
fn rust_target_outcome(fs: &FakeFs) -> Result<Vec<String>, Refusal> {
    let tree = scan_at(fs, "/");
    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        fs,
        &CleanupOptions::default(),
    );

    match plan.refused.first() {
        Some(refused) => Err(refused.reason.clone()),
        None => Ok(paths(&plan)),
    }
}

#[test]
fn nothing_inside_a_folder_of_snapshots_is_cleaned_wherever_it_is() {
    // Snapper keeps `.snapshots` at the root of every subvolume it covers, timeshift its own folder. (The
    // rule for `/.snapshots` itself claims that folder whole, so name rules never look inside it.) The
    // fence of the protected places keeps the name rules out of the others.
    for place in [
        "/home/.snapshots/9/snapshot/u/a",
        "/data/.snapshots/3/snapshot/a",
        "/data/@snapshots/3/snapshot/a",
        "/timeshift-btrfs/snapshots/2024/@/code/a",
    ] {
        let mut fs = FakeFs::new();
        fs.same_filesystem_mount("/data")
            .file(&format!("{place}/Cargo.toml"), 1);
        aged(&mut fs, &format!("{place}/target/x"), 10, 90);

        // Not found at all, so not refused either: nothing in a protected place is offered.
        assert_eq!(rust_target_outcome(&fs), Ok(Vec::new()), "{place}");
    }

    // The same project outside any snapshot is planned.
    let mut fs = FakeFs::new();
    fs.file("/code/a/Cargo.toml", 1);
    aged(&mut fs, "/code/a/target/x", 10, 90);
    assert_eq!(rust_target_outcome(&fs), Ok(vec!["/code/a/target".to_owned()]));
}

#[test]
fn another_path_to_the_folder_of_snapshots_is_refused_by_its_identity() {
    let mut fs = FakeFs::new();
    fs.dir("/.snapshots");
    fs.same_filesystem_mount("/mnt/s")
        .file("/mnt/s/5/snapshot/code/a/Cargo.toml", 1);
    aged(&mut fs, "/mnt/s/5/snapshot/code/a/target/x", 10, 90);
    let (dev, ino) = fs.identity("/.snapshots");
    fs.set_identity("/mnt/s", dev, ino, 1);

    assert_eq!(
        rust_target_outcome(&fs),
        Err(Refusal::Protected("/.snapshots".to_owned()))
    );
}

#[test]
fn nothing_is_planned_when_the_scanned_folder_was_replaced_after_the_scan() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/a/Cargo.toml", 1);
    aged(&mut fs, "/home/u/a/target/x", 10, 90);
    let tree = scan_at(&fs, "/home/u");
    let plan = |fs: &FakeFs| {
        plan_cleanup(
            &tree,
            &env(),
            &RuleSet::builtin(),
            &["rust-target"],
            fs,
            &CleanupOptions::default(),
        )
    };
    assert_eq!(paths(&plan(&fs)), ["/home/u/a/target"]);

    // The same names, another folder: what the scan counted is not what is there.
    let (dev, ino) = fs.identity("/home/u");
    fs.set_identity("/home/u", dev, ino + 500, 1);
    let replaced = plan(&fs);
    assert!(replaced.groups().is_empty(), "{replaced:#?}");
    assert!(
        matches!(&replaced.refused[0].reason, Refusal::Changed(what) if what.contains("Scan again.")),
        "{:?}",
        reasons(&replaced)
    );

    // Mounted over by another disk: another device.
    fs.set_identity("/home/u", dev + 1, ino, 1);
    assert!(plan(&fs).groups().is_empty());
}

#[test]
fn a_target_below_a_mount_the_scan_did_not_go_into_is_not_planned() {
    // The mount is a node of its own in the tree and has nothing below it, so nothing there can match.
    let mut fs = FakeFs::new();
    fs.mount("/home/u/vol");
    fs.skip_mount("/home/u/vol", platter_core::SkipReason::OtherDisk);
    fs.file("/home/u/vol/b/Cargo.toml", 1);
    aged(&mut fs, "/home/u/vol/b/target/x", 10, 90);
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty(), "{plan:#?}");
    assert!(plan.targets().next().is_none());
}

#[test]
fn a_mount_above_the_scan_root_is_fine() {
    let mut fs = FakeFs::new();
    fs.same_filesystem_mount("/home");
    fs.file("/home/u/a/Cargo.toml", 1);
    aged(&mut fs, "/home/u/a/target/x", 10, 90);
    let tree = scan_at(&fs, "/home/u");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(paths(&plan), ["/home/u/a/target"]);
}

#[test]
fn a_review_target_with_an_unreadable_part_is_refused() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/a", 10);
    fs.unreadable_dir("/home/u/.cache/locked");
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["app-cache"],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(
        reasons(&plan),
        [("app-cache", "/home/u/.cache", &Refusal::Unreadable)]
    );
}

#[test]
fn file_rules_plan_single_files() {
    let mut fs = FakeFs::new();
    aged(&mut fs, "/home/u/Downloads/a.iso", 10, 800);
    aged(&mut fs, "/home/u/Downloads/sub/b.zip", 20, 800);
    aged(&mut fs, "/home/u/Downloads/new.zip", 5, 3);
    let tree = scan_at(&fs, "/");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["old-downloads"],
        &fs,
        &CleanupOptions::default(),
    );

    let mut found = paths(&plan);
    found.sort();
    assert_eq!(found, ["/home/u/Downloads/a.iso", "/home/u/Downloads/sub/b.zip"]);
    assert!(plan.targets().all(|t| !t.is_dir()));
}

#[test]
fn a_part_platter_cannot_read_does_not_stop_a_tool_that_cleans_it() {
    let mut fs = FakeFs::new();
    fs.file("/var/cache/pacman/pkg/a.pkg", 100);
    fs.unreadable_dir("/var/cache/pacman/pkg/download-x");
    fs.file("/home/u/.cache/a", 10);
    fs.unreadable_dir("/home/u/.cache/locked");
    let tree = scan_at(&fs, "/var/cache");
    let home_tree = scan_at(&fs, "/");

    let tool = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["pacman-cache"],
        &fs,
        &CleanupOptions::default(),
    );
    let own = plan_cleanup(
        &home_tree,
        &env(),
        &RuleSet::builtin(),
        &["app-cache"],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(paths(&tool), ["/var/cache/pacman/pkg"], "{tool:#?}");
    assert_eq!(
        own.refused[0].reason,
        Refusal::Unreadable,
        "Platter's own deletes still refuse it"
    );
}

#[test]
fn a_file_modified_after_the_scan_or_a_project_whose_marker_is_gone_is_refused() {
    let mut fs = laptop();
    let tree = scan_at(&fs, "/");
    fs.set_mtime("/home/u/Downloads/old.iso", NOW);
    fs.remove("/home/u/code/rs/Cargo.toml");
    fs.remove("/home/u/code/js/package-lock.json");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["old-downloads", "rust-target", "node-modules"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty(), "{plan:#?}");
    let why: Vec<String> = plan
        .refused
        .iter()
        .map(|r| format!("{} {}", r.rule_id, r.reason))
        .collect();
    assert!(
        why.iter()
            .any(|w| w.starts_with("old-downloads") && w.contains("modified after the scan")),
        "{why:?}"
    );
    assert!(
        why.iter()
            .any(|w| w.starts_with("rust-target") && w.contains("(Cargo.toml)")),
        "{why:?}"
    );
    assert!(
        why.iter()
            .any(|w| w.starts_with("node-modules") && w.contains("package-lock.json or")),
        "{why:?}"
    );
}

mod marked {
    use super::*;
    use platter_cleanup::{CleanupOptions, MARKED, plan_marked};
    use platter_core::NodeId;

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

    fn marked_fs() -> FakeFs {
        let mut fs = FakeFs::new();
        aged(&mut fs, "/home/u/old/stuff/a.bin", 100, 10);
        aged(&mut fs, "/home/u/old/stuff/inner/b.bin", 50, 10);
        aged(&mut fs, "/home/u/Documents/thesis/draft.odt", 10, 10);
        aged(
            &mut fs,
            "/home/u/games/.local/share/Steam/steamapps/common/G/g.pak",
            10,
            10,
        );
        fs.symlink("/home/u/link", 1);
        aged(&mut fs, "/home/u/.ssh/id_ed25519", 1, 10);
        fs
    }

    #[test]
    fn a_marked_folder_goes_to_the_trash_as_a_review_item() {
        let fs = marked_fs();
        let tree = scan_at(&fs, "/");

        let plan = plan_marked(
            &tree,
            &env(),
            &RuleSet::builtin(),
            &[node(&tree, "home/u/old/stuff")],
            &fs,
            &CleanupOptions::default(),
        );

        assert_eq!(plan.groups().len(), 1);
        let group = &plan.groups()[0];
        assert_eq!(
            (group.rule_id(), group.verdict(), group.action()),
            (MARKED, Verdict::Review, CleanupAction::Trash)
        );
        assert_eq!(paths(&plan), ["/home/u/old/stuff"]);
        assert_eq!(plan.allocated, 150);
    }

    #[test]
    fn protected_kept_linked_and_nested_marks_are_refused() {
        let fs = marked_fs();
        let tree = scan_at(&fs, "/");
        let env = Environment {
            home: Some(PathBuf::from("/home/u")),
            ..env()
        };
        let marked = [
            node(&tree, "home/u/Documents/thesis"),
            node(&tree, "home/u"),
            node(&tree, "home/u/link"),
            node(&tree, "home/u/.ssh"),
            node(&tree, "home/u/old/stuff"),
            node(&tree, "home/u/old/stuff/inner"),
            NodeId(999_999),
        ];

        let plan = plan_marked(
            &tree,
            &env,
            &RuleSet::builtin(),
            &marked,
            &fs,
            &CleanupOptions::default(),
        );

        assert_eq!(paths(&plan), ["/home/u/old/stuff"]);
        let why = |path: &str| {
            plan.refused
                .iter()
                .find(|refused| refused.path == path)
                .map(|refused| refused.reason.clone())
        };
        // Documents is kept by a rule as well as protected; the keep check comes first.
        assert_eq!(why("/home/u/Documents/thesis"), Some(Refusal::Kept));
        assert_eq!(why("/home/u"), Some(Refusal::Kept), "it holds the kept Documents");
        assert!(matches!(why("/home/u/.ssh"), Some(Refusal::Protected(_))));
        assert_eq!(why("/home/u/link"), Some(Refusal::Symlink));
        assert_eq!(why("/home/u/old/stuff/inner"), Some(Refusal::Nested));
        assert_eq!(plan.refused.len(), 5, "an unknown id is ignored: {plan:#?}");
    }

    #[test]
    fn a_folder_holding_or_inside_a_kept_one_is_refused() {
        let mut fs = FakeFs::new();
        aged(
            &mut fs,
            "/home/u/.local/share/Steam/steamapps/common/G/g.pak",
            10,
            10,
        );
        aged(&mut fs, "/home/u/.local/share/other/x", 10, 10);
        let tree = scan_at(&fs, "/");
        let marked = [
            node(&tree, "home/u/.local/share/Steam"),
            node(&tree, "home/u/.local/share/Steam/steamapps/common/G"),
            node(&tree, "home/u/.local/share/other"),
        ];

        let plan = plan_marked(
            &tree,
            &env(),
            &RuleSet::builtin(),
            &marked,
            &fs,
            &CleanupOptions::default(),
        );

        assert_eq!(paths(&plan), ["/home/u/.local/share/other"]);
        assert!(
            plan.refused.iter().all(|refused| refused.reason == Refusal::Kept),
            "{plan:#?}"
        );
        assert_eq!(plan.refused.len(), 2);
    }
}

/// An idle pnpm workspace at `/home/u/code/ws` with one member that has no lockfile of its own.
fn pnpm_workspace() -> FakeFs {
    let mut fs = FakeFs::new();
    aged(&mut fs, "/home/u/code/ws/package.json", 1, 90);
    aged(&mut fs, "/home/u/code/ws/pnpm-workspace.yaml", 1, 90);
    aged(&mut fs, "/home/u/code/ws/pnpm-lock.yaml", 1, 90);
    aged(&mut fs, "/home/u/code/ws/packages/a/package.json", 1, 90);
    aged(
        &mut fs,
        "/home/u/code/ws/packages/a/node_modules/dep/i.js",
        500,
        90,
    );
    fs
}

fn plan_node_modules(fs: &FakeFs, tree: &Tree) -> CleanupPlan {
    plan_cleanup(
        tree,
        &env(),
        &RuleSet::builtin(),
        &["node-modules"],
        fs,
        &CleanupOptions::default(),
    )
}

#[test]
fn the_node_modules_of_a_workspace_member_is_planned_with_the_workspace_files_as_its_markers() {
    let fs = pnpm_workspace();
    let tree = scan_at(&fs, "/");

    let plan = plan_node_modules(&fs, &tree);

    assert_eq!(paths(&plan), ["/home/u/code/ws/packages/a/node_modules"]);
    assert!(plan.refused.is_empty(), "{:?}", plan.refused);
    let target = plan.targets().next().unwrap();
    let workspace_files: Vec<&[u8]> = vec![
        b"/home/u/code/ws/pnpm-workspace.yaml",
        b"/home/u/code/ws/pnpm-lock.yaml",
    ];
    assert!(
        target
            .markers()
            .iter()
            .flat_map(|group| group.alternatives())
            .any(|alternative| alternative
                .iter()
                .map(Vec::as_slice)
                .eq(workspace_files.iter().copied())),
        "{:?}",
        target.markers()
    );
}

#[test]
fn a_workspace_file_removed_before_planning_refuses_the_member() {
    for gone in ["pnpm-lock.yaml", "pnpm-workspace.yaml"] {
        let mut fs = pnpm_workspace();
        let tree = scan_at(&fs, "/");
        fs.remove(&format!("/home/u/code/ws/{gone}"));

        let plan = plan_node_modules(&fs, &tree);

        assert!(plan.groups().is_empty(), "{gone}");
        let refused = reasons(&plan);
        assert_eq!(refused.len(), 1, "{gone}: {refused:?}");
        assert!(
            matches!(refused[0].2, Refusal::Changed(why) if why.contains(gone)),
            "{gone}: {refused:?}"
        );
    }
}

#[test]
fn a_member_with_a_lockfile_of_its_own_does_not_depend_on_the_workspace_files() {
    let mut fs = pnpm_workspace();
    aged(&mut fs, "/home/u/code/ws/packages/a/pnpm-lock.yaml", 1, 90);
    let tree = scan_at(&fs, "/");
    fs.remove("/home/u/code/ws/pnpm-lock.yaml");

    let plan = plan_node_modules(&fs, &tree);

    assert_eq!(paths(&plan), ["/home/u/code/ws/packages/a/node_modules"]);
}

#[test]
fn the_old_downloads_rule_is_planned_in_the_configured_downloads_folder_and_not_in_a_namesake() {
    let mut fs = FakeFs::new();
    aged(&mut fs, "/home/u/Pobrane/old.iso", 200, 800);
    aged(&mut fs, "/home/u/Downloads/namesake.iso", 300, 800);
    let tree = scan_at(&fs, "/");
    let env = Environment {
        user_dirs: vec![(UserDir::Downloads, PathBuf::from("/home/u/Pobrane"))],
        ..env()
    };

    let plan = plan_cleanup(
        &tree,
        &env,
        &RuleSet::builtin(),
        &["old-downloads"],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(paths(&plan), ["/home/u/Pobrane/old.iso"]);
    assert!(plan.refused.is_empty(), "{:?}", plan.refused);
}

#[test]
fn downloads_is_not_a_protected_personal_folder() {
    let mut fs = FakeFs::new();
    aged(&mut fs, "/home/u/Pobrane/old.iso", 200, 800);
    let tree = scan_at(&fs, "/");
    let env = Environment {
        user_dirs: vec![(UserDir::Downloads, PathBuf::from("/home/u/Pobrane"))],
        ..env()
    };

    let plan = plan_cleanup(
        &tree,
        &env,
        &RuleSet::builtin(),
        &["old-downloads"],
        &fs,
        &CleanupOptions::default(),
    );

    assert!(
        !plan
            .refused
            .iter()
            .any(|r| matches!(r.reason, Refusal::Protected(_))),
        "{:?}",
        plan.refused
    );
    assert_eq!(plan.groups().len(), 1);
}

/// Python caches in a settings folder (Blender's add-ons keep theirs in `~/.config/blender`) and in a project.
fn caches_in_config_and_code() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.config/blender/addons/a/__pycache__/m.pyc", 40)
        .file("/home/u/.config/blender/addons/a/__pycache__/n.pyc", 40)
        .file("/home/u/.config/app/cachey/blob", 70)
        .file("/home/u/code/py/__pycache__/m.pyc", 30)
        .file("/home/u/code/py/scratch/blob", 50);
    fs
}

fn node_at(tree: &Tree, path: &str) -> NodeId {
    (0..u32::try_from(tree.len()).unwrap())
        .map(NodeId)
        .find(|&node| tree.path_bytes(node) == path.as_bytes())
        .unwrap_or_else(|| panic!("{path} is in the tree"))
}

#[test]
fn a_name_match_in_a_protected_place_is_neither_planned_nor_refused() {
    let fs = caches_in_config_and_code();
    let tree = scan_at(&fs, "/home/u");

    let plan = plan_cleanup(
        &tree,
        &env(),
        &RuleSet::builtin(),
        &["python-cache"],
        &fs,
        &CleanupOptions::default(),
    );

    assert_eq!(paths(&plan), ["/home/u/code/py/__pycache__"]);
    assert!(plan.refused.is_empty(), "{:?}", plan.refused);
}

#[test]
fn the_suggestions_leave_out_what_lies_in_a_protected_place() {
    let fs = caches_in_config_and_code();
    let tree = scan_at(&fs, "/home/u");
    let labels = [
        ModelLabel {
            node: node_at(&tree, "/home/u/.config/app/cachey"),
            verdict: ModelVerdict::Review,
            reason: "temporary".to_owned(),
        },
        ModelLabel {
            node: node_at(&tree, "/home/u/code/py/scratch"),
            verdict: ModelVerdict::Review,
            reason: "temporary".to_owned(),
        },
    ];

    let suggestions = evaluate_with(&tree, &env(), &fenced(RuleSet::builtin(), &env()), &labels);
    let shown = |id: &str| -> (u64, u64) {
        suggestions
            .suggestions
            .iter()
            .find(|suggestion| suggestion.id == id)
            .map(|suggestion| (suggestion.allocated, suggestion.items))
            .unwrap_or_default()
    };

    assert_eq!(shown("python-cache"), (30, 1), "only the cache of the project");
    assert_eq!(shown("advisor"), (50, 1), "only the label outside the settings");

    // Without the fence both caches are found (the plan would refuse the one in `~/.config`).
    let unfenced = evaluate_with(&tree, &env(), &RuleSet::builtin(), &[]);
    let python = unfenced
        .suggestions
        .iter()
        .find(|suggestion| suggestion.id == "python-cache")
        .unwrap();
    assert_eq!(python.items, 2);
}
