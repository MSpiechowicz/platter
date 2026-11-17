use std::fs;

use platter_core::dto::ApiErrorCode;
use platter_core::{MAX_USER_RULES, UserVerdict};

use super::*;

/// A path written the Linux way as the same path on the system the test runs on: `/data/a` is `C:\data\a` on
/// Windows. A relative path stays as it is.
fn native(posix: &str) -> PathBuf {
    if cfg!(windows) && posix.starts_with('/') {
        return PathBuf::from(format!("C:{}", posix.replace('/', "\\")));
    }

    PathBuf::from(posix)
}

fn native_text(posix: &str) -> String {
    native(posix).to_string_lossy().into_owned()
}

fn store_in(dir: &tempfile::TempDir) -> (UserRulesStore, PathBuf) {
    let path = dir.path().join("platter/rules.json");

    (UserRulesStore::new(Some(path.clone())), path)
}

fn folders(view: &UserRulesView) -> Vec<(String, UserVerdict)> {
    view.rules
        .iter()
        .map(|rule| (rule.folder.to_string_lossy().into_owned(), rule.verdict))
        .collect()
}

#[test]
fn no_file_means_no_rules_and_generation_zero() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store_in(&dir);

    let view = store.view();

    assert_eq!((view.generation, view.rules.len(), view.problem), (0, 0, None));
    assert!(!path.exists(), "looking does not create the file");
}

#[test]
fn a_rule_is_saved_to_the_file_and_counts_as_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store_in(&dir);

    let view = store.set(&native("/data/a"), UserVerdict::Keep).unwrap();

    assert_eq!(view.generation, 1);
    assert_eq!(folders(&view), [(native_text("/data/a"), UserVerdict::Keep)]);
    let again = UserRulesStore::new(Some(path));
    assert_eq!(
        folders(&again.view()),
        folders(&view),
        "it is read back at the next start"
    );
}

#[test]
fn a_folder_that_has_a_rule_gets_the_new_verdict_and_the_same_verdict_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = store_in(&dir);
    let first = store.set(&native("/data/a"), UserVerdict::Keep).unwrap();

    let same = store.set(&native("/data/a"), UserVerdict::Keep).unwrap();
    let replaced = store.set(&native("/data/a"), UserVerdict::Safe).unwrap();

    assert_eq!(same.generation, first.generation, "nothing changed");
    assert_eq!(replaced.generation, first.generation + 1);
    assert_eq!(folders(&replaced), [(native_text("/data/a"), UserVerdict::Safe)]);
    assert_eq!(
        replaced.rules[0].id, first.rules[0].id,
        "the rule keeps its number"
    );
}

#[test]
fn removing_a_rule_changes_the_generation_and_an_unknown_id_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = store_in(&dir);
    let added = store.set(&native("/data/a"), UserVerdict::Keep).unwrap();

    let unknown = store.remove(99).unwrap();
    let removed = store.remove(added.rules[0].id).unwrap();

    assert_eq!(unknown.generation, added.generation);
    assert_eq!(removed.generation, added.generation + 1);
    assert!(removed.rules.is_empty());
}

#[test]
fn a_folder_a_rule_cannot_name_and_a_full_list_are_refused_and_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store_in(&dir);
    for index in 0..MAX_USER_RULES {
        store
            .set(&native(&format!("/data/{index}")), UserVerdict::Keep)
            .unwrap();
    }
    let before = fs::read(&path).unwrap();
    let generation = store.view().generation;

    for folder in ["/", "relative", "/a/../b", "/data/more"] {
        let error = store.set(&native(folder), UserVerdict::Keep).unwrap_err();

        assert_eq!(error.code(), ApiErrorCode::Invalid, "{folder}");
    }

    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(store.view().generation, generation);
}

#[test]
fn an_edit_by_hand_while_the_app_runs_is_seen_as_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store_in(&dir);
    let before = store.view();

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = serde_json::json!({
        "version": 1,
        "next_id": 2,
        "rules": [{ "id": 1, "verdict": "keep", "folder": native_text("/data/hand") }]
    });
    fs::write(&path, file.to_string()).unwrap();
    let after = store.view();

    assert_eq!(after.generation, before.generation + 1);
    assert_eq!(folders(&after), [(native_text("/data/hand"), UserVerdict::Keep)]);
    assert_eq!(
        store.view().generation,
        after.generation,
        "looking again is no change"
    );
}

fn broken(dir: &tempfile::TempDir) -> (UserRulesStore, PathBuf, &'static [u8]) {
    let path = dir.path().join("platter/rules.json");
    let garbage: &[u8] = b"{ not json";
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, garbage).unwrap();

    (UserRulesStore::new(Some(path.clone())), path, garbage)
}

#[test]
fn a_broken_file_is_reported_refuses_every_change_and_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path, garbage) = broken(&dir);

    let view = store.view();
    let added = store.set(&native("/data/a"), UserVerdict::Keep).unwrap_err();
    let removed = store.remove(1).unwrap_err();

    assert!(view.problem.is_some());
    assert!(view.rules.is_empty());
    assert_eq!(view.usable().unwrap_err().code(), ApiErrorCode::Invalid);
    assert_eq!(
        (added.code(), removed.code()),
        (ApiErrorCode::Invalid, ApiErrorCode::Invalid)
    );
    assert_eq!(fs::read(&path).unwrap(), garbage);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn a_file_that_breaks_while_the_app_runs_refuses_planning_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (store, path) = store_in(&dir);
    store.set(&native("/data/a"), UserVerdict::Keep).unwrap();
    let before = store.view();

    fs::write(&path, b"oops").unwrap();
    let after = store.view();

    assert!(after.usable().is_err());
    assert_eq!(after.generation, before.generation + 1);
}

#[test]
fn start_over_moves_the_file_aside_and_only_when_it_is_broken() {
    let dir = tempfile::tempdir().unwrap();
    let (fine, _) = store_in(&dir);
    assert_eq!(fine.reset().unwrap_err().code(), ApiErrorCode::Invalid);

    let other = tempfile::tempdir().unwrap();
    let (store, path, garbage) = broken(&other);
    let before = store.view().generation;

    let view = store.reset().unwrap();

    assert!(view.problem.is_none() && view.rules.is_empty());
    assert_eq!(view.generation, before + 1);
    assert_eq!(
        fs::read(path.with_file_name("rules.json.broken")).unwrap(),
        garbage
    );
    assert!(!path.exists());
    store.set(&native("/data/a"), UserVerdict::Safe).unwrap();
}

/// A symbolic link to a file (on Windows this needs the privilege to make links).
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
}

#[test]
fn a_link_at_the_path_is_a_broken_file() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.json");
    fs::write(&real, br#"{"version":1,"next_id":1,"rules":[]}"#).unwrap();
    let path = dir.path().join("rules.json");
    symlink_file(&real, &path).unwrap();

    let store = UserRulesStore::new(Some(path));

    assert!(store.view().problem.is_some());
}

#[test]
fn without_a_settings_folder_there_are_no_rules_and_none_can_be_added() {
    let store = UserRulesStore::new(None);

    assert!(store.view().rules.is_empty());
    assert_eq!(
        store
            .set(&native("/data/a"), UserVerdict::Keep)
            .unwrap_err()
            .code(),
        ApiErrorCode::Io
    );
}
