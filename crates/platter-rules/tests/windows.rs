//! The rules on a Windows environment. The fake disk has `/` paths (the tree takes its separator from the
//! system the tests run on), so the Windows part is the environment: `os`, the system folder and the case
//! rule. What needs real Windows paths is tested in CI on Windows.

mod common;

use std::path::PathBuf;

use common::*;
use platter_core::dto::Verdict;
use platter_core::testing::FakeFs;
use platter_rules::{Environment, Os, RuleSet, evaluate};

fn windows(home: &str) -> Environment {
    Environment {
        os: Os::Windows,
        system_root: Some(PathBuf::from("/Windows")),
        ..env_with_home(Some(home))
    }
}

fn suggested(fs: &FakeFs, env: &Environment) -> Vec<(String, Verdict)> {
    let tree = scan_at(fs, "/", false);

    evaluate(&tree, env, &RuleSet::builtin())
        .suggestions
        .into_iter()
        .map(|s| (s.id, s.verdict))
        .collect()
}

fn disk() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/AppData/Local/Temp/installer.tmp", 5_000)
        .file("/home/u/AppData/Local/npm-cache/_cacache/x", 4_000)
        .file("/home/u/AppData/Local/pip/Cache/y", 3_000)
        .file("/home/u/AppData/Local/NuGet/http-cache/z", 2_000)
        .file("/home/u/AppData/Local/NuGet/v3-cache/z", 1_900)
        .file("/home/u/AppData/Local/CrashDumps/a.dmp", 1_800)
        .file("/Windows/WinSxS/amd64_x/f.dll", 9_000)
        .file("/home/u/.local/share/Trash/files/old", 600)
        .file("/home/u/.cache/app/blob", 700)
        .file("/home/u/.docker/desktop/vms/disk.img", 800)
        .file("/home/u/.local/share/Steam/steamapps/shadercache/1/c", 900);
    fs
}

#[test]
fn the_rules_of_windows_are_offered_there_and_with_the_verdicts_the_plan_gave() {
    let found = suggested(&disk(), &windows("/home/u"));
    let verdict = |id: &str| {
        found
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, verdict)| *verdict)
    };

    for id in [
        "win-temp",
        "win-npm-cache",
        "win-pip-cache",
        "win-nuget-http-cache",
        "win-nuget-v3-cache",
        "win-crash-dumps",
    ] {
        assert_eq!(verdict(id), Some(Verdict::Safe), "{id}");
    }
    assert_eq!(verdict("win-component-store"), Some(Verdict::Review));
}

#[test]
fn the_rules_of_linux_are_not_offered_on_windows_and_the_rules_of_windows_not_on_linux() {
    let on_windows = suggested(&disk(), &windows("/home/u"));
    let on_linux = suggested(&disk(), &env());

    for id in ["trash", "app-cache", "docker-desktop-vm", "steam-shader-cache"] {
        assert!(on_windows.iter().all(|(known, _)| known != id), "{id} on Windows");
        assert!(on_linux.iter().any(|(known, _)| known == id), "{id} on Linux");
    }
    assert!(
        on_linux.iter().all(|(known, _)| !known.starts_with("win-")),
        "{on_linux:?}"
    );
}

#[test]
fn windows_names_are_not_case_sensitive_when_a_rule_finds_its_folder() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/appdata/LOCAL/temp/installer.tmp", 5_000);

    let windows = suggested(&fs, &windows("/home/u"));
    let linux = suggested(&fs, &env());

    assert!(windows.iter().any(|(id, _)| id == "win-temp"), "{windows:?}");
    assert!(linux.is_empty(), "{linux:?}");
}

#[test]
fn a_scan_started_with_another_spelling_of_the_profile_still_finds_the_rules_folders() {
    let mut fs = FakeFs::new();
    fs.file("/Home/U/AppData/Local/Temp/installer.tmp", 5_000);

    let found = suggested(&fs, &windows("/home/u"));

    assert!(found.iter().any(|(id, _)| id == "win-temp"), "{found:?}");
}

#[test]
fn dependencies_that_a_program_installed_below_program_files_are_not_a_project() {
    let mut fs = FakeFs::new();
    for project in ["/Program Files/Tool", "/home/u/code/web"] {
        aged_file(&mut fs, &format!("{project}/package.json"), 1, 100);
        aged_file(&mut fs, &format!("{project}/package-lock.json"), 1, 100);
        aged_file(&mut fs, &format!("{project}/node_modules/dep/i.js"), 1_500, 100);
    }
    let tree = scan_at(&fs, "/", false);

    let windows = evaluate(&tree, &windows("/home/u"), &RuleSet::builtin());
    let linux = evaluate(&tree, &env(), &RuleSet::builtin());

    let modules = |suggestions: &platter_rules::Suggestions| {
        suggestions
            .suggestions
            .iter()
            .find(|s| s.id == "node-modules")
            .map(|s| s.items)
    };
    assert_eq!(modules(&windows), Some(1), "only the project in the home folder");
    assert_eq!(modules(&linux), Some(2), "Linux does not know Program Files");
}
