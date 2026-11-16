//! The safety boundary of this crate: it labels, it never touches anything. The crate's sources must not
//! use the filesystem, processes, the environment or the clock, and its only dependency must be
//! `platter-core`. A future cleanup feature belongs in another crate; this test fails if code that acts
//! creeps in here.

// This test is the one place that reads files: it reads the crate's own sources to inspect them.
#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::{Path, PathBuf};

/// Paths and calls that read or change the outside world.
const FORBIDDEN: &[&str] = &[
    "std::fs",
    "std::process",
    "std::env",
    "std::net",
    "std::os",
    "std::time::SystemTime",
    "std::time::Instant",
    "SystemTime",
    "Instant::now",
    "use std::fs",
    "fs::",
    "remove_file",
    "remove_dir",
    "rename(",
    "Command::new",
    "unsafe",
    "libc",
    "rustix",
    "tokio",
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();

        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_sources_use_no_filesystem_process_environment_or_clock_api() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    assert!(files.len() >= 8, "the sources were found: {files:?}");

    let mut violations = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();

        for (number, line) in text.lines().enumerate() {
            for word in FORBIDDEN {
                if line.contains(word) {
                    violations.push(format!(
                        "{}:{}: {word}: {}",
                        file.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "read-only crate touches the outside world:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_only_dependency_is_platter_core() {
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();

    let dependencies: Vec<&str> = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a dependencies section")
        .split("\n[")
        .next()
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();

    assert_eq!(dependencies, ["platter-core.workspace = true"]);
}
