//! `platter-cli suggest` against a temp-dir home. HOME points at the fixture, so the home rules apply to it.

// The rules of the two systems differ (the trash and `~/.cache` are Linux's, `AppData\Local\Temp` is
// Windows'), so what depends on them is marked for its own system; the rest runs on both.

use std::fs;
#[cfg(unix)]
use std::fs::{File, FileTimes};
use std::path::Path;
use std::process::{Command, Output};
#[cfg(unix)]
use std::time::{Duration, SystemTime};

use serde_json::Value;

fn suggest(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_platter-cli"))
        .arg("suggest")
        .arg(home)
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .expect("binary runs")
}

fn write(home: &Path, path: &str, size: usize) {
    let file = home.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, vec![0u8; size]).unwrap();
}

/// A home with an application cache, a Rust project (recently touched), a trash and a Documents folder.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    write(home, ".cache/app/blob", 50_000);
    write(home, ".local/share/Trash/files/old", 20_000);
    write(home, "code/proj/Cargo.toml", 100);
    write(home, "code/proj/target/debug/obj", 30_000);
    write(home, "Documents/thesis.odt", 10_000);
    write(home, "notes.txt", 500);
    dir
}

fn stdout_text(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[cfg(unix)]
#[test]
fn json_is_the_suggestions_wire_format() {
    let dir = fixture();

    let output = suggest(dir.path(), &["--json"]);
    let report: Value = serde_json::from_str(&stdout_text(&output)).expect("stdout is JSON");

    let suggestions = report["suggestions"].as_array().unwrap();
    let ids: Vec<&str> = suggestions.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["trash", "app-cache", "rust-target-recent", "documents"]);

    let cache = &suggestions[1];
    assert_eq!(cache["verdict"], "review");
    assert_eq!(cache["path"], "~/.cache");
    assert_eq!(cache["items"], 1);
    assert!(cache["allocated"].as_u64().unwrap() >= 50_000);
    assert!(cache["nodes"].as_array().unwrap().len() == 1);

    assert_eq!(suggestions[2]["path"], "~/code/proj/target");
    assert_eq!(suggestions[2]["verdict"], "review");
    assert_eq!(suggestions[3]["verdict"], "keep");

    let totals = &report["totals"];
    assert!(totals["safe"].as_u64().unwrap() >= 20_000);
    assert!(totals["review"].as_u64().unwrap() >= 80_000);
    assert!(totals["keep"].as_u64().unwrap() >= 10_000);
}

#[cfg(unix)]
#[test]
fn the_table_lists_each_suggestion_and_a_total_per_verdict_without_the_real_home() {
    let dir = fixture();

    let text = stdout_text(&suggest(dir.path(), &[]));

    let header = text.lines().next().unwrap();
    for column in ["VERDICT", "TITLE", "PATH", "ITEMS", "SIZE"] {
        assert!(header.contains(column), "{header}");
    }
    let cache = text
        .lines()
        .find(|line| line.contains("Application caches"))
        .unwrap();
    assert!(
        cache.starts_with("review") && cache.contains("~/.cache"),
        "{cache}"
    );

    for verdict in ["safe", "review", "keep"] {
        assert!(
            text.lines()
                .any(|line| line.starts_with(verdict) && line.ends_with("KiB")),
            "a totals line for {verdict}: {text}"
        );
    }
    assert!(
        !text.contains(dir.path().to_str().unwrap()),
        "home is shown as ~: {text}"
    );
}

#[test]
fn a_folder_with_nothing_to_suggest_says_so() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "plain.txt", 100);

    let text = stdout_text(&suggest(dir.path(), &[]));

    assert!(text.contains("Nothing found"), "{text}");
}

#[cfg(unix)]
#[test]
fn a_path_from_a_hostile_folder_name_cannot_inject_terminal_sequences() {
    let dir = tempfile::tempdir().unwrap();
    let project = "evil\x1b[2J\u{202e}proj";
    write(dir.path(), &format!("{project}/Cargo.toml"), 10);
    write(dir.path(), &format!("{project}/target/obj"), 5_000);

    let text = stdout_text(&suggest(dir.path(), &[]));

    assert!(
        !text.chars().any(|ch| ch.is_control() && ch != '\n'),
        "no raw control character: {text:?}"
    );
    assert!(!text.contains('\u{202e}'));
    assert!(text.contains("evil\\u{1b}[2J"), "{text:?}");
}

#[test]
fn stats_go_to_stderr_and_leave_stdout_alone() {
    let dir = fixture();

    let output = suggest(dir.path(), &["--json", "--stats"]);

    serde_json::from_str::<Value>(&stdout_text(&output)).expect("stdout is still JSON");
    assert!(String::from_utf8_lossy(&output.stderr).contains("rules evaluated in"));
}

#[test]
fn a_missing_path_fails_with_a_message() {
    let output = Command::new(env!("CARGO_BIN_EXE_platter-cli"))
        .args(["suggest", "/definitely/not/here"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("platter-cli:"));
}

/// Sets the access and modification time of a file or directory.
#[cfg(unix)]
fn set_times(path: &Path, accessed: SystemTime, modified: SystemTime) {
    File::open(path)
        .unwrap()
        .set_times(FileTimes::new().set_accessed(accessed).set_modified(modified))
        .unwrap();
}

#[cfg(unix)]
fn ids_of(report: &Value) -> Vec<String> {
    report["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|suggestion| suggestion["id"].as_str().unwrap().to_owned())
        .collect()
}

#[cfg(unix)]
#[test]
fn the_age_of_a_rust_target_is_its_newest_modification_and_a_mere_read_does_not_make_it_recent() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write(home, "old/Cargo.toml", 100);
    write(home, "old/target/debug/obj", 30_000);
    write(home, "worked-on/Cargo.toml", 100);
    write(home, "worked-on/target/debug/obj", 30_000);

    let now = SystemTime::now();
    let long_ago = now - Duration::from_secs(120 * 86_400);

    // The old project: nothing in it was modified for 120 days, but a file manager listed `target`
    // (and looked at its files) just now, which moves the access time and nothing else.
    for path in [
        "old/target/debug/obj",
        "old/target/debug",
        "old/target",
        "old/Cargo.toml",
    ] {
        set_times(&home.join(path), now, long_ago);
    }
    // A control: the same shape with a file that was written a minute ago.
    for path in [
        "worked-on/target/debug",
        "worked-on/target",
        "worked-on/Cargo.toml",
    ] {
        set_times(&home.join(path), long_ago, long_ago);
    }
    set_times(&home.join("worked-on/target/debug/obj"), long_ago, now);

    let output = suggest(home, &["--json"]);
    let report: Value = serde_json::from_str(&stdout_text(&output)).unwrap();

    let found: Vec<&Value> = report["suggestions"].as_array().unwrap().iter().collect();
    let idle = found
        .iter()
        .find(|s| s["id"] == "rust-target")
        .expect("the idle project is found");
    let recent = found
        .iter()
        .find(|s| s["id"] == "rust-target-recent")
        .expect("the project that was written to is found");

    assert_eq!(idle["items"], 1, "only the old project, whatever its atime");
    assert_eq!(idle["path"], "~/old/target");
    assert_eq!(idle["verdict"], "safe");
    assert_eq!(recent["items"], 1);
    assert_eq!(recent["path"], "~/worked-on/target");
    assert_eq!(recent["verdict"], "review");
}

#[cfg(unix)]
#[test]
fn files_with_more_than_one_hard_link_are_counted_in_shared_files() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    write(home, ".local/share/Trash/files/shared", 10_000);
    write(home, ".local/share/Trash/files/alone", 2_000);
    write(home, "elsewhere/placeholder", 1);
    fs::hard_link(
        home.join(".local/share/Trash/files/shared"),
        home.join("elsewhere/kept-copy"),
    )
    .unwrap();

    let output = suggest(home, &["--json"]);
    let report: Value = serde_json::from_str(&stdout_text(&output)).unwrap();

    assert_eq!(ids_of(&report), ["trash"]);
    let trash = &report["suggestions"][0];
    assert_eq!(trash["sharedFiles"], 1);
    assert_eq!(trash["items"], 1);
}

#[cfg(unix)]
#[test]
fn an_excluded_folder_produces_no_suggestion() {
    let dir = fixture();
    let target = dir.path().join("code/proj/target");

    let output = suggest(dir.path(), &["--json", "--exclude", target.to_str().unwrap()]);
    let report: Value = serde_json::from_str(&stdout_text(&output)).expect("stdout is JSON");

    let ids: Vec<&str> = report["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["trash", "app-cache", "documents"]);
}

#[cfg(windows)]
#[test]
fn the_temp_folder_of_the_profile_is_a_safe_suggestion_on_windows() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "AppData/Local/Temp/installer.tmp", 20_000);
    write(dir.path(), "notes.txt", 500);

    let output = suggest(dir.path(), &["--json"]);
    let report: Value = serde_json::from_str(&stdout_text(&output)).expect("stdout is JSON");

    let temp = report["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|suggestion| suggestion["id"] == "win-temp")
        .expect("the Temp folder is suggested");
    assert_eq!(temp["verdict"], "safe");
    assert_eq!(temp["path"], "~\\AppData\\Local\\Temp");
}

#[cfg(unix)]
#[test]
fn the_rules_of_windows_are_not_offered_on_linux() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "AppData/Local/Temp/installer.tmp", 20_000);
    write(dir.path(), "AppData/Local/CrashDumps/x.dmp", 20_000);

    let text = stdout_text(&suggest(dir.path(), &["--json"]));

    assert!(!text.contains("win-"), "{text}");
}
