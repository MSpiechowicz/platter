//! Runs the built binary against a temp directory fixture.

use std::fs;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_platter-cli"))
        .args(args)
        .output()
        .expect("binary runs")
}

fn stdout_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is JSON")
}

/// `big/` holds 3 x 100 kB, `small/` one 1 kB file, plus one loose 10 kB file.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("big")).unwrap();
    fs::create_dir_all(root.join("small")).unwrap();
    for i in 0..3 {
        fs::write(root.join(format!("big/f{i}.mp4")), vec![0u8; 100_000]).unwrap();
    }
    fs::write(root.join("small/note.txt"), vec![0u8; 1_000]).unwrap();
    fs::write(root.join("loose.bin"), vec![0u8; 10_000]).unwrap();
    dir
}

fn path_arg(dir: &tempfile::TempDir) -> &str {
    dir.path().to_str().unwrap()
}

#[test]
fn scan_json_reports_totals_and_largest_children() {
    let dir = fixture();
    let report = stdout_json(&cli(&["scan", path_arg(&dir), "--json", "--top", "2", "--stats"]));

    let totals = &report["totals"];
    assert_eq!(totals["files"], 5);
    assert_eq!(totals["dirs"], 2);
    assert_eq!(totals["errors"], 0);
    assert_eq!(totals["apparent"], 311_000);
    assert!(totals["allocated"].as_u64().unwrap() >= 311_000);

    let children = report["children"].as_array().unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0]["name"], "big");
    assert_eq!(children[0]["kind"], "dir");
    assert_eq!(children[0]["category"], "video");

    let stats = &report["stats"];
    assert_eq!(stats["entries"], 8, "root + 2 dirs + 5 files");
    assert!(stats["bytesPerEntry"].as_u64().unwrap() > 0);
}

#[test]
fn scan_text_prints_totals_top_and_stats() {
    let dir = fixture();
    let output = cli(&["scan", path_arg(&dir), "--top", "3", "--stats"]);
    assert!(output.status.success());

    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("files 5, folders 2, errors 0"), "{text}");
    assert!(text.contains("Largest items"));
    assert!(text.contains("big"));
    assert!(text.contains("entries/sec"));
    assert!(text.contains("per entry"));
}

#[test]
fn scan_text_writes_the_share_like_the_app_does() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("huge.bin"), vec![0u8; 4_000_000]).unwrap();
    fs::write(dir.path().join("tiny.txt"), b"x").unwrap();

    let output = cli(&["scan", path_arg(&dir)]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();

    let share_of = |name: &str| {
        let line = text
            .lines()
            .find(|line| line.ends_with(name))
            .unwrap_or_else(|| panic!("a row for {name}: {text}"));
        line.split_whitespace().nth(2).unwrap().to_owned()
    };

    // The big one holds almost everything but never reads 100%, the tiny one is not nothing.
    assert_eq!(share_of("huge.bin"), "99%", "{text}");
    assert_eq!(share_of("tiny.txt"), "<1%", "{text}");
}

#[test]
fn scanning_a_missing_path_fails_with_a_message() {
    let dir = fixture();
    let missing = dir.path().join("not-here");

    let output = cli(&["scan", missing.to_str().unwrap()]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("platter-cli:"));
}

#[test]
fn export_fixture_writes_the_mock_file() {
    let dir = fixture();
    let out_dir = tempfile::tempdir().unwrap();
    let out = out_dir.path().join("nested/scan.json");
    let out = out.to_str().unwrap();

    let output = cli(&["export-fixture", path_arg(&dir), "--out", out, "--max-nodes", "6"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let fixture: Value = serde_json::from_slice(&fs::read(out).unwrap()).unwrap();

    assert_eq!(fixture["totals"]["files"], 5);
    assert_eq!(fixture["root"]["node"]["id"], 0);
    assert!(fixture["root"]["children"].as_array().unwrap().len() >= 2);
    assert_eq!(fixture["summary"]["node"]["id"], 0);
    assert_eq!(fixture["summary"]["byAge"].as_array().unwrap().len(), 6);
    assert_eq!(fixture["summary"]["byCategory"][0]["category"], "video");
}

#[test]
fn volumes_json_is_an_array_of_volume_dtos() {
    let volumes = stdout_json(&cli(&["volumes", "--json"]));

    for volume in volumes.as_array().unwrap() {
        assert!(volume["mountPoint"].is_string());
        assert!(volume["fsType"].is_string());
        assert!(volume["total"].is_u64());
    }
}

// A Windows file name cannot hold the ASCII control characters this name has. The escaping is covered by
// `format.rs` and, with the characters Windows allows (C1, DEL, bidi), by the next test.
#[cfg(unix)]
const HOSTILE_NAME: &str = "evil\x1b[2J\x1b]0;pwned\x07\u{202e}gpj.exe";

#[cfg(unix)]
#[test]
fn hostile_file_names_cannot_inject_terminal_sequences() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(HOSTILE_NAME), vec![0u8; 100]).unwrap();

    let text = cli(&["scan", path_arg(&dir)]);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();

    assert!(
        !text.chars().any(|ch| ch.is_control() && ch != '\n'),
        "no raw control character reaches the terminal: {text:?}"
    );
    assert!(!text.contains('\u{202e}'), "no bidi override either");
    assert!(
        text.contains("evil\\u{1b}[2J\\u{1b}]0;pwned\\u{7}\\u{202e}gpj.exe"),
        "the name stays readable, with visible escapes: {text:?}"
    );

    let report = stdout_json(&cli(&["scan", path_arg(&dir), "--json"]));
    assert_eq!(
        report["children"][0]["name"], HOSTILE_NAME,
        "--json output is exact"
    );
}

#[test]
fn json_output_escapes_del_c1_and_bidi_controls_and_round_trips_the_exact_name() {
    let name = "a\u{9b}b\u{9d}c\u{7f}d\u{202e}e.txt";
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(name), vec![0u8; 100]).unwrap();

    let output = cli(&["scan", path_arg(&dir), "--json"]);
    assert!(output.status.success());
    let raw = String::from_utf8(output.stdout).unwrap();

    for ch in ['\u{9b}', '\u{9d}', '\u{7f}', '\u{202e}'] {
        assert!(
            !raw.contains(ch),
            "U+{:04X} reached the terminal raw",
            u32::from(ch)
        );
    }
    assert!(
        raw.contains("\\u009b")
            && raw.contains("\\u009d")
            && raw.contains("\\u007f")
            && raw.contains("\\u202e")
    );

    let report: Value = serde_json::from_str(&raw).expect("still valid JSON");
    assert_eq!(
        report["children"][0]["name"], name,
        "the name is exact once parsed"
    );
}

#[test]
fn a_hostile_argument_is_not_echoed_raw_in_the_usage_error() {
    // The escape sequence of the report; and a C1 control, DEL and a literal backslash for good measure
    // (clap itself drops the escape sequence and DEL when it renders the message).
    for argument in ["--x\x1b]0;PWNED\x07y", "--x\u{9b}2Jy\u{7f}z\\u{1b}"] {
        let output = cli(&["scan", argument]);

        assert_eq!(output.status.code(), Some(2), "clap's usage error code is kept");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            !stderr.chars().any(|ch| ch.is_control() && ch != '\n'),
            "no raw control character in {stderr:?}"
        );
        assert!(
            stderr.starts_with("error: unexpected argument '--x"),
            "{stderr:?}"
        );
    }

    let output = cli(&["scan", "--x\u{9b}2Jy\u{7f}z\\u{1b}"]);
    // The backslash is doubled on Unix only: on Windows it is the path separator and prints as it is.
    let quoted = if cfg!(windows) {
        "'--x\\u{9b}2Jyz\\u{1b}'"
    } else {
        "'--x\\u{9b}2Jyz\\\\u{1b}'"
    };
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(quoted),
        "what clap quotes is shown with visible escapes and the host's backslash form"
    );
}

#[test]
fn help_and_version_still_work() {
    let help = cli(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: platter-cli"));

    let scan_help = cli(&["scan", "--help"]);
    assert!(scan_help.status.success());
    assert!(String::from_utf8_lossy(&scan_help.stdout).contains("--max-entries"));

    let version = cli(&["--version"]);
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("platter-cli "));

    let missing = cli(&[]);
    assert_eq!(missing.status.code(), Some(2), "no subcommand is a usage error");
}

#[test]
fn a_closed_pipe_ends_the_command_quietly_with_success() {
    let dir = fixture();
    let mut child = Command::new(env!("CARGO_BIN_EXE_platter-cli"))
        .args(["scan", path_arg(&dir), "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary runs");

    // `platter-cli scan ... | head -0`: the reader is gone before anything is written.
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stderr: {stderr}");
    assert!(!stderr.contains("panicked"), "no panic message: {stderr}");
    assert!(stderr.trim().is_empty(), "nothing is printed: {stderr}");
}

#[test]
fn a_scan_over_the_entry_limit_fails_in_plain_words() {
    let dir = fixture();

    let output = cli(&["scan", path_arg(&dir), "--max-entries", "3"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "This folder has more entries than Platter can hold in memory (limit 3 entries). Scan a smaller folder."
        ),
        "{stderr}"
    );

    let fits = cli(&["scan", path_arg(&dir), "--max-entries", "100"]);
    assert!(fits.status.success());
}

#[test]
fn the_default_memory_budget_fits_the_free_memory_and_leaves_normal_scans_alone() {
    let dir = fixture();

    let report = stdout_json(&cli(&["scan", path_arg(&dir), "--json"]));

    assert_eq!(report["totals"]["files"], 5);
}

#[test]
fn scan_leaves_out_an_excluded_folder_and_counts_nothing_in_it() {
    let dir = fixture();
    let big = dir.path().join("big");
    let report = stdout_json(&cli(&[
        "scan",
        path_arg(&dir),
        "--json",
        "--exclude",
        big.to_str().unwrap(),
    ]));

    assert_eq!(report["totals"]["files"], 2, "small/note.txt and loose.bin");
    assert_eq!(report["totals"]["apparent"], 11_000);

    let children = report["children"].as_array().unwrap();
    let left_out = children.iter().find(|child| child["name"] == "big").unwrap();
    assert_eq!(left_out["kind"], "mount");
    assert_eq!(left_out["skipped"], "excluded");
    assert_eq!(left_out["allocated"], 0);
}

#[test]
fn an_excluded_folder_that_does_not_exist_is_ignored_with_a_warning() {
    let dir = fixture();
    let missing = dir.path().join("not-here");
    let report = cli(&[
        "scan",
        path_arg(&dir),
        "--json",
        "--exclude",
        missing.to_str().unwrap(),
    ]);

    assert!(
        report.status.success(),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    assert!(String::from_utf8_lossy(&report.stderr).contains("not left out"));

    let report: Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(report["totals"]["files"], 5);
}

#[test]
fn excluding_the_scan_root_still_scans_it() {
    let dir = fixture();
    let report = stdout_json(&cli(&[
        "scan",
        path_arg(&dir),
        "--json",
        "--exclude",
        path_arg(&dir),
    ]));

    assert_eq!(report["totals"]["files"], 5);
}

#[test]
fn more_excluded_folders_than_the_app_allows_is_refused() {
    let dir = fixture();
    let mut args = vec!["scan", path_arg(&dir)];
    for _ in 0..65 {
        args.extend(["--exclude", "/tmp"]);
    }

    let output = cli(&args);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("at most 64"));
}
