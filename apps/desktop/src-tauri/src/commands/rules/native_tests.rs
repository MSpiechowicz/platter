//! The user's rules over IPC on a real folder of the system the tests run on (a Windows path is `C:\...`).
//! `tests.rs` models a Linux disk (a `FakeFs` with `/mnt` and the pacman cache) and runs on Unix only.

use std::fs;
use std::path::{Path, PathBuf};

use platter_core::dto::{ApiErrorCode, ScanId};
use platter_core::{CancelToken, NodeId, ScanConfig, UserRule, UserVerdict};

use super::*;
use crate::commands::advisor::AdvisorService;
use crate::user_rules::UserRulesStore;

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(future)
}

/// A state with its rules file in a temp folder, and a folder to scan: `photos/2024/a.jpg` and `notes.txt`.
fn setup() -> (AppState, tempfile::TempDir, PathBuf, ScanId) {
    let dir = tempfile::tempdir().unwrap();
    let rules_file = dir.path().join("platter/rules.json");
    let state = AppState::with_parts(
        AdvisorService::unavailable(),
        UserRulesStore::new(Some(rules_file.clone())),
    );

    let data = dir.path().join("data");
    fs::create_dir_all(data.join("photos/2024")).unwrap();
    fs::write(data.join("photos/2024/a.jpg"), vec![1u8; 2_000]).unwrap();
    fs::write(data.join("notes.txt"), b"hello").unwrap();

    let root = platter_platform::root::resolve_root(&data).unwrap().path;
    let (id, cancel) = state.scans.begin().unwrap();
    let tree = platter_core::scan(
        &state.platform.dir_source,
        &root,
        &ScanConfig::default(),
        &CancelToken::new(),
        &|_| {},
    )
    .unwrap();
    drop(cancel);
    state.scans.finish(id, tree).unwrap();

    (state, dir, rules_file, id)
}

fn child(state: &AppState, scan: ScanId, name: &str) -> NodeId {
    let tree = state.scans.tree(scan).unwrap();

    tree.children(tree.root())
        .find(|&node| tree.name(node) == name.as_bytes())
        .unwrap_or_else(|| panic!("{name} is in the scan"))
}

#[test]
fn a_folder_of_the_scan_gets_a_rule_that_is_listed_saved_and_removed() {
    let (state, _dir, rules_file, scan) = setup();
    let photos = child(&state, scan, "photos");

    let added = block_on(add(&state, scan, photos, UserVerdictDto::Keep)).unwrap();

    assert_eq!(added.problem, None);
    assert_eq!(added.rules.len(), 1);
    assert_eq!(
        (added.rules[0].id, added.rules[0].verdict),
        (1, UserVerdictDto::Keep)
    );
    assert!(
        added.rules[0].path.ends_with("photos"),
        "{:?}",
        added.rules[0].path
    );
    assert!(rules_file.is_file(), "the rule is in the file");
    assert_eq!(
        block_on(list(&state)).unwrap(),
        added,
        "it is read back from the file"
    );

    let removed = block_on(remove(&state, 1)).unwrap();

    assert!(removed.rules.is_empty());
}

#[test]
fn only_a_folder_that_the_scan_has_can_get_a_rule_and_safe_is_refused_for_the_scan_root() {
    let (state, _dir, rules_file, scan) = setup();
    let tree = state.scans.tree(scan).unwrap();
    let file = child(&state, scan, "notes.txt");
    let refused = [
        block_on(add(&state, scan, file, UserVerdictDto::Keep)),
        block_on(add(&state, scan, NodeId(999_999), UserVerdictDto::Keep)),
        block_on(add(&state, scan, tree.root(), UserVerdictDto::Safe)),
    ];

    let codes: Vec<ApiErrorCode> = refused
        .into_iter()
        .map(|result| result.unwrap_err().code())
        .collect();

    assert_eq!(
        codes,
        [
            ApiErrorCode::Invalid,
            ApiErrorCode::NotFound,
            ApiErrorCode::Invalid
        ]
    );
    assert!(!rules_file.exists(), "nothing was saved");
}

#[test]
fn a_broken_file_is_listed_with_its_problem_and_start_over_moves_it_aside() {
    let (state, _dir, rules_file, scan) = setup();
    fs::create_dir_all(rules_file.parent().unwrap()).unwrap();
    fs::write(&rules_file, b"{ not json").unwrap();

    let listed = block_on(list(&state)).unwrap();
    let refused = block_on(add(
        &state,
        scan,
        child(&state, scan, "photos"),
        UserVerdictDto::Keep,
    ));
    let started_over = block_on(reset(&state)).unwrap();

    assert!(listed.problem.is_some());
    assert!(listed.rules.is_empty());
    assert_eq!(refused.unwrap_err().code(), ApiErrorCode::Invalid);
    assert_eq!(started_over.problem, None);
    assert_eq!(
        fs::read(rules_file.with_file_name("rules.json.broken")).unwrap(),
        b"{ not json"
    );
}

#[test]
fn a_folder_below_the_home_is_shown_with_a_tilde_and_one_elsewhere_in_full() {
    let home = native("/home/me");
    let rule = |folder: &str| UserRule {
        id: 1,
        verdict: UserVerdict::Keep,
        folder: native(folder),
    };

    assert_eq!(rule_dto(&rule("/home/me"), Some(&home)).path, "~");
    assert!(
        rule_dto(&rule("/home/me/code"), Some(&home))
            .path
            .starts_with("~/")
    );
    assert!(
        rule_dto(&rule("/home/me/code"), Some(&home))
            .path
            .ends_with("code")
    );
    assert_eq!(
        rule_dto(&rule("/data/a"), Some(&home)).path,
        native("/data/a").to_string_lossy()
    );
    assert_eq!(
        rule_dto(&rule("/data/a"), None).path,
        native("/data/a").to_string_lossy()
    );
}

/// A path written the Linux way as the same path on the system the test runs on: `/data/a` is `C:\data\a` on
/// Windows.
fn native(posix: &str) -> PathBuf {
    if cfg!(windows) {
        return Path::new("C:\\").join(posix.trim_start_matches('/').replace('/', "\\"));
    }

    PathBuf::from(posix)
}
