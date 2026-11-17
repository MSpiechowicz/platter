//! A cleanup from the scan to the Recycle Bin and back, on a real Windows volume with a tiny temporary
//! profile. Run in CI only (`windows-latest`): the Recycle Bin of the runner's system drive is used.

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};

use platter_cleanup::{CleanupOptions, Disk, TargetOutcome, execute, plan_cleanup, plan_marked, undo};
use platter_core::{CancelToken, Environment, Os, ScanConfig, Tree, scan};
use platter_platform::Platform;
use platter_platform::root::resolve_root;
use platter_rules::RuleSet;

/// A profile with a Temp folder (a file and a folder with a file in it) and a Documents folder, and the
/// environment that goes with it.
fn profile() -> (tempfile::TempDir, PathBuf, Environment) {
    let dir = tempfile::tempdir().unwrap();
    let home = resolve_root(dir.path()).unwrap().path;
    let temp = home.join("AppData").join("Local").join("Temp");
    fs::create_dir_all(temp.join("sub")).unwrap();
    fs::write(temp.join("a.tmp"), vec![1u8; 4_000]).unwrap();
    fs::write(temp.join("sub").join("b.tmp"), vec![2u8; 4_000]).unwrap();
    fs::create_dir_all(home.join("Documents")).unwrap();
    fs::write(home.join("Documents").join("thesis.docx"), b"mine").unwrap();

    let env = Environment {
        home: Some(home.clone()),
        user_dirs: Vec::new(),
        now: 0,
        os: Os::Windows,
        system_root: Platform::current().environment().system_root,
    };

    (dir, home, env)
}

fn scan_of(root: &Path) -> Tree {
    scan(
        &Platform::current().dir_source,
        root,
        &ScanConfig::default(),
        &CancelToken::new(),
        &|_| {},
    )
    .unwrap()
}

#[test]
fn the_temp_folder_is_emptied_into_the_recycle_bin_and_every_child_can_be_put_back() {
    let (_guard, home, env) = profile();
    let temp = home.join("AppData").join("Local").join("Temp");
    let platform = Platform::current();
    let tree = scan_of(&home);

    let plan = plan_cleanup(
        &tree,
        &env,
        &RuleSet::builtin(),
        &["win-temp"],
        &platform.entry_probe,
        &CleanupOptions::default(),
    );

    assert_eq!(plan.groups().len(), 1, "{:?}", plan.refused);
    let disk = Disk {
        probe: &platform.entry_probe,
        remover: &platform.entry_remover,
        trash_dir: None,
        tools: None,
    };
    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    assert!(temp.is_dir(), "the folder itself stays");
    assert_eq!(fs::read_dir(&temp).unwrap().count(), 0);
    assert!(home.join("Documents").join("thesis.docx").exists());
    assert_eq!(report.results.len(), 2, "{:?}", report.results);
    for result in &report.results {
        let TargetOutcome::Trashed(to) = &result.outcome else {
            panic!("{:?}", result)
        };
        assert!(String::from_utf8_lossy(to).contains("$Recycle.Bin"));
    }

    let back = undo(&report.results, &platform.entry_remover);

    assert!(back.iter().all(|(_, outcome)| outcome.is_ok()), "{back:?}");
    assert_eq!(fs::read(temp.join("a.tmp")).unwrap(), vec![1u8; 4_000]);
    assert_eq!(
        fs::read(temp.join("sub").join("b.tmp")).unwrap(),
        vec![2u8; 4_000]
    );
}

#[test]
fn the_places_windows_keeps_for_itself_are_never_planned_even_when_marked_by_hand() {
    let (_guard, home, env) = profile();
    let platform = Platform::current();
    let tree = scan_of(&home);
    let node = |name: &[u8]| {
        tree.children(tree.root())
            .find(|&child| tree.name(child) == name)
            .expect("the folder is in the scan")
    };
    let documents = node(b"Documents");
    let thesis = tree.children(documents).next().expect("the file is in the scan");

    let plan = plan_marked(
        &tree,
        &env,
        &RuleSet::builtin(),
        &[documents, thesis],
        &platform.entry_probe,
        &CleanupOptions::default(),
    );

    assert!(plan.groups().is_empty(), "{:?}", plan.groups());
    assert_eq!(plan.refused.len(), 2);
    assert!(
        plan.refused
            .iter()
            .all(|refused| refused.reason.to_string().contains("protected")),
        "{:?}",
        plan.refused
    );
    assert!(home.join("Documents").join("thesis.docx").exists());
}
