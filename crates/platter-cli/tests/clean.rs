//! `platter-cli clean --dry-run` against a temp-dir home: it plans and never changes the disk.

use std::fs::{self, File, FileTimes};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime};

fn clean(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_platter-cli"))
        .arg("clean")
        .arg(home)
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .expect("binary runs")
}

fn write_old(home: &Path, path: &str, size: usize) {
    let file = home.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, vec![0u8; size]).unwrap();
    let old = SystemTime::now() - Duration::from_secs(400 * 86_400);
    File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(FileTimes::new().set_modified(old))
        .unwrap();
}

/// Every path below `dir`, sorted: what "nothing changed" is checked against.
fn listing(dir: &Path) -> Vec<PathBuf> {
    let mut all = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            }
            all.push(entry.path());
        }
    }
    all.sort();
    all
}

fn text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write_old(home, "code/p/Cargo.toml", 10);
    write_old(home, "code/p/target/debug/obj", 40_000);
    write_old(home, "Documents/q/Cargo.toml", 10);
    write_old(home, "Documents/q/target/obj", 40_000);
    write_old(home, ".cache/app/blob", 5_000);
    symlink(home.join("code/p/target"), home.join("code/link")).unwrap();
    age_folders(home);
    dir
}

/// Makes every folder below `dir` (the deepest first) look untouched for 400 days, as in an idle project.
fn age_folders(dir: &Path) {
    let old = SystemTime::now() - Duration::from_secs(400 * 86_400);
    let mut folders: Vec<PathBuf> = listing(dir)
        .into_iter()
        .filter(|path| fs::symlink_metadata(path).unwrap().is_dir())
        .collect();
    folders.sort_by_key(|path| std::cmp::Reverse(path.components().count()));

    for folder in folders {
        File::open(&folder)
            .unwrap()
            .set_times(FileTimes::new().set_modified(old))
            .unwrap();
    }
}

#[test]
fn a_dry_run_lists_the_plan_and_changes_nothing() {
    let dir = fixture();
    let home = fs::canonicalize(dir.path()).unwrap();
    let before = listing(&home);

    let output = clean(&home, &["--safe", "--dry-run", "--verbose"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = text(&output);
    assert!(out.starts_with("Dry run: nothing was changed."), "{out}");
    assert!(out.contains("rust-target  delete"), "{out}");
    assert!(
        out.contains(&format!("{}/code/p/target", home.display())),
        "{out}"
    );
    assert!(
        !out.contains("Documents/q/target"),
        "a kept folder is never planned: {out}"
    );
    assert!(!out.contains("code/link"), "a link is never followed: {out}");
    assert_eq!(listing(&home), before);
}

#[test]
fn a_named_rule_that_found_nothing_is_reported() {
    let dir = fixture();

    let out = text(&clean(dir.path(), &["--rule", "python-cache", "--dry-run"]));

    assert!(out.contains("Nothing to clean."), "{out}");
    assert!(out.contains("Found nothing for: python-cache"), "{out}");
}

#[test]
fn with_yes_it_cleans_what_the_plan_lists_and_nothing_else() {
    let dir = fixture();
    let home = fs::canonicalize(dir.path()).unwrap();
    write_old(&home, "Downloads/old.iso", 1_000);
    age_folders(&home);

    let output = Command::new(env!("CARGO_BIN_EXE_platter-cli"))
        .args(["clean"])
        .arg(&home)
        .args(["--rule", "rust-target", "--rule", "old-downloads", "--yes"])
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();

    let out = text(&output);
    assert!(
        output.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.contains("Cleaned 2 of 2 items."), "{out}");
    assert!(!home.join("code/p/target").exists());
    assert!(home.join("code/p/Cargo.toml").exists());
    assert!(
        home.join("Documents/q/target/obj").exists(),
        "a kept folder is untouched"
    );
    assert!(
        home.join(".cache/app/blob").exists(),
        "a rule not chosen is untouched"
    );
    assert!(
        fs::symlink_metadata(home.join("code/link")).is_ok(),
        "the link is untouched"
    );
    assert!(!home.join("Downloads/old.iso").exists());
    assert!(
        home.join(".local/share/Trash/files/old.iso").exists(),
        "personal files go to the trash"
    );
    assert!(home.join(".local/share/Trash/info/old.iso.trashinfo").exists());
}

#[test]
fn without_dry_run_or_a_selection_it_refuses_and_changes_nothing() {
    let dir = fixture();
    let before = listing(dir.path());

    for args in [
        &["--safe"][..],
        &["--dry-run"][..],
        &["--safe", "--dry-run", "--yes"][..],
    ] {
        let output = clean(dir.path(), args);

        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
    assert_eq!(listing(dir.path()), before);
}

fn clean_yes(home: &Path, rules: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_platter-cli"));
    command.arg("clean").arg(home);
    for rule in rules {
        command.args(["--rule", rule]);
    }
    command
        .arg("--yes")
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap()
}

#[test]
fn emptying_the_trash_keeps_its_folders() {
    let dir = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(dir.path()).unwrap();
    let trash = home.join(".local/share/Trash");
    write_old(&home, ".local/share/Trash/files/old.txt", 100);
    write_old(&home, ".local/share/Trash/files/folder/inner", 100);
    write_old(&home, ".local/share/Trash/info/old.txt.trashinfo", 10);
    write_old(&home, ".local/share/Trash/directorysizes", 10);

    let output = clean_yes(&home, &["trash"]);

    assert!(
        output.status.success(),
        "{}{}",
        text(&output),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(trash.join("files").is_dir() && trash.join("info").is_dir());
    assert_eq!(fs::read_dir(trash.join("files")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(trash.join("info")).unwrap().count(), 0);
    assert!(!trash.join("directorysizes").exists());
}

#[test]
fn a_trash_part_that_is_a_link_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(dir.path()).unwrap();
    write_old(&home, "elsewhere/precious", 100);
    write_old(&home, ".local/share/Trash/info/x.trashinfo", 10);
    symlink(home.join("elsewhere"), home.join(".local/share/Trash/files")).unwrap();

    let output = clean_yes(&home, &["trash"]);

    assert!(
        !output.status.success(),
        "a refused part is reported as a problem"
    );
    assert!(text(&output).contains("Problems:"), "{}", text(&output));
    assert!(home.join("elsewhere/precious").exists());
    assert_eq!(
        fs::read_dir(home.join(".local/share/Trash/info"))
            .unwrap()
            .count(),
        0
    );
}

const NAMESPACE_CHILD: &str = "PLATTER_TEST_NAMESPACE_CHILD";

#[test]
fn a_second_path_to_the_home_does_not_get_past_the_protected_places() {
    if let Some(dir) = std::env::var_os(NAMESPACE_CHILD) {
        let root = PathBuf::from(dir);
        let home = root.join("home");
        write_old(&home, ".config/tool/p/Cargo.toml", 10);
        write_old(&home, ".config/tool/p/target/obj", 1_000);
        write_old(&home, "Documents/q/Cargo.toml", 10);
        write_old(&home, "Documents/q/target/obj", 1_000);
        write_old(&home, "code/r/Cargo.toml", 10);
        write_old(&home, "code/r/target/obj", 1_000);
        age_folders(&home);
        let alt = root.join("alt");
        fs::create_dir(&alt).unwrap();
        let bound = Command::new("mount")
            .arg("--bind")
            .arg(&home)
            .arg(&alt)
            .status()
            .unwrap();
        assert!(bound.success());

        let output = Command::new(env!("CARGO_BIN_EXE_platter-cli"))
            .arg("clean")
            .arg(&alt)
            .args(["--safe", "--dry-run", "--verbose"])
            .env("HOME", &home)
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .unwrap();

        let out = text(&output);
        assert!(output.status.success(), "{out}");
        assert!(
            out.contains(&format!("{}/code/r/target", alt.display())),
            "an ordinary project is still planned: {out}"
        );
        assert!(
            !out.contains("rust-target  ") || !out.contains("/.config/tool/p/target\n"),
            "{out}"
        );
        let planned: Vec<&str> = out
            .lines()
            .filter(|line| line.starts_with("  rust-target  ") && !line.contains(": "))
            .collect();
        assert_eq!(planned.len(), 1, "only the ordinary project: {out}");
        assert!(
            out.contains("protected"),
            "the other two are refused as protected: {out}"
        );

        // The suggestions know the second path too: Documents there is kept, not a place to clean.
        let suggested = Command::new(env!("CARGO_BIN_EXE_platter-cli"))
            .arg("suggest")
            .arg(&alt)
            .arg("--json")
            .env("HOME", &home)
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .unwrap();
        let json = text(&suggested);
        assert!(
            json.contains("\"id\": \"documents\"") || json.contains("\"id\":\"documents\""),
            "{json}"
        );
        assert!(
            json.contains("\"verdict\": \"keep\"") || json.contains("\"verdict\":\"keep\""),
            "{json}"
        );
        return;
    }

    let usable = Command::new("unshare")
        .args(["-Urm", "true"])
        .status()
        .is_ok_and(|s| s.success());
    if !usable {
        eprintln!("skipped: unprivileged user namespaces are not available here");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let status = Command::new("unshare")
        .arg("-Urm")
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_second_path_to_the_home_does_not_get_past_the_protected_places",
            "--nocapture",
        ])
        .env(NAMESPACE_CHILD, fs::canonicalize(dir.path()).unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "the test in the namespace failed");
}

#[test]
fn with_yes_a_run_that_cleans_nothing_or_misses_a_named_rule_fails() {
    let dir = fixture();
    let home = fs::canonicalize(dir.path()).unwrap();

    let nothing = clean_yes(&home, &["python-cache"]);
    let kept_only = clean_yes(&home, &["documents"]);

    assert!(!nothing.status.success(), "a named rule found nothing");
    assert!(!kept_only.status.success(), "everything was refused");
}

#[test]
fn a_protected_folder_reached_through_a_link_is_protected_where_it_really_is() {
    let dir = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(dir.path()).unwrap();
    write_old(&home, "dotfiles/config/tool/p/Cargo.toml", 10);
    write_old(&home, "dotfiles/config/tool/p/target/obj", 1_000);
    write_old(&home, "code/r/Cargo.toml", 10);
    write_old(&home, "code/r/target/obj", 1_000);
    symlink(home.join("dotfiles/config"), home.join(".config")).unwrap();
    age_folders(&home);

    for root in [home.clone(), home.join(".config")] {
        let output = Command::new(env!("CARGO_BIN_EXE_platter-cli"))
            .arg("clean")
            .arg(&root)
            .args(["--rule", "rust-target", "--dry-run", "--verbose"])
            .env("HOME", &home)
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .unwrap();

        let out = text(&output);
        let planned: Vec<&str> = out
            .lines()
            .filter(|line| line.starts_with("  rust-target  ") && !line.contains(": "))
            .collect();
        assert!(
            planned.iter().all(|line| !line.contains("dotfiles/config")),
            "scanning {}: {out}",
            root.display()
        );
        assert!(
            out.contains("dotfiles/config/tool/p/target: protected"),
            "scanning {}: {out}",
            root.display()
        );
    }
}
