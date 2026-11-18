//! The user's rules in the CLI: `suggest` and `clean` read the same `rules.json` the app writes
//! (`$XDG_CONFIG_HOME/platter/rules.json`, here in a temp folder). A broken file makes `clean` refuse.

// Linux only. These tests move the rules file with `XDG_CONFIG_HOME` and the home folder with `HOME`, and
// look in the freedesktop trash. On macOS the file is `~/Library/Application Support/platter/config/rules.json`
// and `XDG_*` is ignored, so the file would follow `HOME` only, and cleanup moves to `~/.Trash`. On
// Windows the file is `%APPDATA%\platter\rules.json`, found through the profile's known folder, which no
// environment variable moves: a test there would read and write the real user's rules. The same rules in
// the same file are covered on Windows by `platter-platform` (`user_rules`) and by the desktop's
// `commands::rules` tests.
#![cfg(target_os = "linux")]

use std::fs::{self, File, FileTimes};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime};

use serde_json::Value;

struct Setup {
    home_dir: tempfile::TempDir,
    config_dir: tempfile::TempDir,
}

impl Setup {
    /// A home with an idle Rust project and a folder of the user's own, and an empty config folder.
    fn new() -> Self {
        let home_dir = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(home_dir.path()).unwrap();
        write(&home, "code/p/Cargo.toml", 10);
        write(&home, "code/p/target/debug/obj", 40_000);
        write(&home, "stuff/data/blob", 30_000);
        write(&home, "stuff/other/blob", 20_000);
        age_folders(&home);

        Self {
            home_dir,
            config_dir: tempfile::tempdir().unwrap(),
        }
    }

    fn home(&self) -> PathBuf {
        fs::canonicalize(self.home_dir.path()).unwrap()
    }

    fn rules_file(&self) -> PathBuf {
        self.config_dir.path().join("platter/rules.json")
    }

    /// Writes the rules file with `rules` as (id, verdict, folder below the home).
    fn rules(&self, rules: &[(u32, &str, &str)]) {
        let list: Vec<String> = rules
            .iter()
            .map(|(id, verdict, folder)| {
                format!(
                    r#"{{"id":{id},"verdict":"{verdict}","folder":"{}"}}"#,
                    self.home().join(folder).display()
                )
            })
            .collect();

        self.raw(&format!(
            r#"{{"version":1,"next_id":{},"rules":[{}]}}"#,
            rules.len() + 1,
            list.join(",")
        ));
    }

    fn raw(&self, text: &str) {
        let path = self.rules_file();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn run(&self, command: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_platter-cli"))
            .arg(command)
            .arg(self.home())
            .args(args)
            .env("HOME", self.home())
            .env("XDG_CONFIG_HOME", self.config_dir.path())
            .env_remove("XDG_DATA_HOME")
            .output()
            .expect("binary runs")
    }
}

fn write(home: &Path, path: &str, size: usize) {
    let file = home.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, vec![0u8; size]).unwrap();
}

/// Makes every file and folder below `dir` look untouched for 400 days.
fn age_folders(dir: &Path) {
    let old = SystemTime::now() - Duration::from_secs(400 * 86_400);
    let mut all = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path.clone());
            }
            all.push(path);
        }
    }
    all.sort_by_key(|path| std::cmp::Reverse(path.components().count()));

    for path in all {
        File::open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(old))
            .unwrap();
    }
}

fn out(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn err(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn suggest_lists_the_users_rules_with_their_source() {
    let setup = Setup::new();
    setup.rules(&[(1, "keep", "stuff/data"), (2, "safe", "stuff/other")]);

    let output = setup.run("suggest", &["--json"]);

    assert!(output.status.success(), "{}", err(&output));
    let report: Value = serde_json::from_str(&out(&output)).unwrap();
    let find = |id: &str| {
        report["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|suggestion| suggestion["id"] == id)
            .unwrap_or_else(|| panic!("{id} is suggested: {report}"))
    };
    assert_eq!(
        (
            find("user-keep")["verdict"].clone(),
            find("user-keep")["source"].clone()
        ),
        ("keep".into(), "user".into())
    );
    assert_eq!(
        (
            find("user-safe")["verdict"].clone(),
            find("user-safe")["source"].clone()
        ),
        ("safe".into(), "user".into())
    );
    assert!(
        find("user-keep")["reason"]
            .as_str()
            .unwrap()
            .contains("always keep")
    );
}

#[test]
fn suggest_leaves_what_a_user_keep_holds_out() {
    let setup = Setup::new();
    let before = out(&setup.run("suggest", &["--json"]));
    setup.rules(&[(1, "keep", "code/p")]);

    let after = out(&setup.run("suggest", &["--json"]));

    assert!(before.contains("rust-target"), "control: {before}");
    assert!(
        !after.contains(r#""id": "rust-target"#) && !after.contains(r#""id":"rust-target"#),
        "{after}"
    );
    assert!(after.contains("user-keep"), "{after}");
}

#[test]
fn clean_never_plans_what_a_user_keep_protects() {
    let setup = Setup::new();
    let target = setup.home().join("code/p/target");
    let control = setup.run("clean", &["--rule", "rust-target", "--dry-run"]);
    assert!(
        out(&control).contains("rust-target  delete"),
        "control: {}",
        out(&control)
    );

    for keep in ["code/p/target", "code/p"] {
        setup.rules(&[(1, "keep", keep)]);

        let dry = setup.run("clean", &["--rule", "rust-target", "--dry-run"]);
        let yes = setup.run("clean", &["--rule", "rust-target", "--yes"]);

        assert!(
            !out(&dry).contains("rust-target  delete"),
            "{keep}: {}",
            out(&dry)
        );
        assert!(!yes.status.success(), "{keep}: nothing could be cleaned");
        assert!(target.join("debug/obj").exists(), "{keep}: nothing was deleted");
    }
}

#[test]
fn a_user_safe_folder_is_cleaned_with_safe_and_goes_to_the_trash() {
    let setup = Setup::new();
    setup.rules(&[(1, "safe", "stuff/data"), (2, "keep", "stuff/other")]);
    let home = setup.home();

    let dry = setup.run("clean", &["--safe", "--dry-run"]);
    assert!(dry.status.success(), "{}", err(&dry));
    let plan = out(&dry);
    let line = plan
        .lines()
        .find(|line| line.starts_with("user-safe"))
        .expect("user-safe is planned");
    assert!(line.contains("move to trash"), "never a direct delete: {plan}");
    assert!(home.join("stuff/data/blob").exists(), "a dry run changes nothing");

    let yes = setup.run("clean", &["--rule", "user-safe", "--yes"]);

    assert!(yes.status.success(), "{}{}", out(&yes), err(&yes));
    assert!(!home.join("stuff/data").exists());
    assert!(
        home.join(".local/share/Trash/files/data/blob").exists(),
        "trashed, not deleted"
    );
    assert!(
        home.join("stuff/other/blob").exists(),
        "the kept folder is untouched"
    );
    assert!(
        home.join("code/p/target/debug/obj").exists(),
        "a rule not chosen is untouched"
    );
}

#[test]
fn a_broken_rules_file_refuses_clean_and_is_left_as_it_is() {
    let setup = Setup::new();
    setup.raw("{ this is not json");
    let home = setup.home();

    for args in [
        &["--safe", "--dry-run"][..],
        &["--rule", "rust-target", "--yes"][..],
    ] {
        let output = setup.run("clean", args);

        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty(), "no plan is printed: {}", out(&output));
        assert!(err(&output).contains("rules file"), "{}", err(&output));
    }
    assert!(home.join("code/p/target/debug/obj").exists());
    assert_eq!(fs::read(setup.rules_file()).unwrap(), b"{ this is not json");
}

#[test]
fn suggest_with_a_broken_file_warns_and_shows_the_builtin_suggestions() {
    let setup = Setup::new();
    setup.raw(r#"{"version":2,"next_id":1,"rules":[]}"#);

    let output = setup.run("suggest", &["--json"]);

    assert!(output.status.success());
    assert!(
        err(&output).contains("rules file cannot be used"),
        "{}",
        err(&output)
    );
    assert!(out(&output).contains("rust-target"), "{}", out(&output));
}

#[test]
fn a_link_in_place_of_the_rules_file_is_not_followed() {
    let setup = Setup::new();
    let real = setup.config_dir.path().join("real.json");
    fs::write(&real, br#"{"version":1,"next_id":1,"rules":[]}"#).unwrap();
    fs::create_dir_all(setup.rules_file().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, setup.rules_file()).unwrap();

    let output = setup.run("clean", &["--safe", "--dry-run"]);

    assert!(!output.status.success());
    assert!(err(&output).contains("rules file"), "{}", err(&output));
}
