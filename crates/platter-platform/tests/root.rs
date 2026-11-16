//! Resolving the scan root and pinning the scan to the directory that was resolved.

#![cfg(unix)]

use std::fs;

use platter_core::{CancelToken, ScanConfig, ScanError, scan};
use platter_platform::DefaultDirSource;
use platter_platform::root::resolve_root;
use platter_platform::testing::symlink_dir;

fn scan_pinned(
    path: &std::path::Path,
    identity: Option<(u64, u64)>,
) -> Result<platter_core::Tree, ScanError> {
    let config = ScanConfig {
        root_identity: identity,
        ..ScanConfig::default()
    };

    scan(
        &DefaultDirSource::default(),
        path,
        &config,
        &CancelToken::new(),
        &|_| {},
    )
}

#[test]
fn a_symlinked_root_is_resolved_to_the_real_directory() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("photos")).unwrap();
    symlink_dir(&dir.path().join("photos"), &dir.path().join("link")).unwrap();

    let resolved = resolve_root(&dir.path().join("link")).unwrap();

    assert_eq!(
        resolved.path,
        fs::canonicalize(dir.path().join("photos")).unwrap()
    );
    assert!(resolved.identity.is_some());
}

#[test]
fn a_missing_path_and_a_file_are_refused_with_distinct_errors() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file"), b"x").unwrap();

    let missing = resolve_root(&dir.path().join("nope")).unwrap_err();
    let file = resolve_root(&dir.path().join("file")).unwrap_err();

    assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(file.kind(), std::io::ErrorKind::NotADirectory);
}

#[test]
fn the_identity_resolve_reports_is_the_one_the_reader_sees() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("f"), b"hello").unwrap();

    let resolved = resolve_root(dir.path()).unwrap();
    let tree = scan_pinned(&resolved.path, resolved.identity).expect("the pinned scan starts");

    assert_eq!(tree.totals().files, 1);
}

#[test]
fn a_root_replaced_after_resolving_is_not_scanned() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("target");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("original"), b"x").unwrap();

    let resolved = resolve_root(&path).unwrap();

    // Another folder takes the path while the first one still exists (so inode numbers differ).
    fs::create_dir(dir.path().join("imposter")).unwrap();
    fs::write(dir.path().join("imposter/other"), b"y").unwrap();
    fs::rename(&path, dir.path().join("moved")).unwrap();
    fs::rename(dir.path().join("imposter"), &path).unwrap();

    let result = scan_pinned(&resolved.path, resolved.identity);

    assert!(matches!(result, Err(ScanError::Root { .. })), "{result:?}");
    assert!(
        scan_pinned(&resolved.path, None).is_ok(),
        "without a pin it would scan the imposter"
    );
}
