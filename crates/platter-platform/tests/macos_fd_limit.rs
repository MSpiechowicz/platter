//! A scan must be complete even when the process has far fewer file descriptors than the tree is deep.
//!
//! This is its own test binary because it lowers `RLIMIT_NOFILE` for the whole process while it runs.

#![cfg(target_os = "macos")]

use std::fs;
use std::path::Path;

use platter_core::{CancelToken, ScanConfig, scan};
use platter_platform::macos::MacDirSource;
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

const SOFT_LIMIT: u64 = 64;
// A path on macOS is at most 1024 bytes, and the chain is built by path.
const DEPTH: usize = 200;
const SIDE_DIRS: [&str; 3] = ["x", "y", "z"];

/// A chain of `DEPTH` directories, each with three side directories holding a file, so every level
/// has several pending children while its first child is explored. Returns the directories created.
fn build_tree(root: &Path) -> u64 {
    let mut path = root.to_path_buf();
    let mut dirs = 0;

    for _ in 0..DEPTH {
        path.push("a");
        fs::create_dir(&path).unwrap();
        dirs += 1;

        for side in SIDE_DIRS {
            let side_dir = path.join(side);
            fs::create_dir(&side_dir).unwrap();
            fs::write(side_dir.join("f"), b"x").unwrap();
            dirs += 1;
        }
    }
    dirs
}

#[test]
fn a_deep_tree_is_scanned_completely_with_a_tiny_descriptor_limit() {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let dirs = build_tree(dir.path());

    let original = getrlimit(Resource::Nofile);
    setrlimit(
        Resource::Nofile,
        Rlimit {
            current: Some(SOFT_LIMIT),
            maximum: original.maximum,
        },
    )
    .expect("lower the soft limit");

    // One worker explores the chain `a` first, so without the fallback it keeps one descriptor per level
    // open (the side directories of every level are still pending): 200 descriptors against a limit
    // of 64.
    let config = ScanConfig {
        threads: Some(1),
        ..ScanConfig::default()
    };
    let result = scan(
        &MacDirSource::new(),
        dir.path(),
        &config,
        &CancelToken::new(),
        &|_| {},
    );

    setrlimit(Resource::Nofile, original).expect("restore the soft limit");

    let tree = result.expect("scan succeeds");
    assert_eq!(tree.totals().dirs, dirs, "every directory was read");
    assert_eq!(tree.totals().files, (DEPTH * SIDE_DIRS.len()) as u64);
    assert_eq!(
        tree.totals().errors,
        0,
        "nothing was lost to descriptor exhaustion"
    );
}
