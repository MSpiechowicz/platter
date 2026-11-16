use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use platter_core::dto::{ApiErrorCode, ScanEvent, ScanOptions, SuggestionDto, SuggestionSource, Verdict};
use platter_core::testing::{FakeFs, scan_fake};
use platter_core::{MAX_USER_RULES, ModelLabel, ModelVerdict};
use serde_json::json;

use super::*;
use crate::commands::advisor::{AdvisorService, AnswerSourceDto, advisor_labels, ask_about, label_scan};
use crate::commands::{scan, suggestions};
use crate::state::LabelSet;
use crate::user_rules::UserRulesStore;

const MIB: u64 = 1 << 20;

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(future)
}

/// A state with its rules file in a temp folder.
fn state() -> (AppState, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("platter/rules.json");
    let state = AppState::with_parts(
        AdvisorService::unavailable(),
        UserRulesStore::new(Some(path.clone())),
    );

    (state, dir, path)
}

fn register(state: &AppState, fs: &FakeFs) -> ScanId {
    let (id, _) = state.scans.begin().unwrap();
    state.scans.finish(id, scan_fake(fs, false)).unwrap();

    id
}

fn node_at(state: &AppState, scan: ScanId, path: &str) -> NodeId {
    let tree = state.scans.tree(scan).unwrap();
    let mut current = tree.root();

    for part in path.split('/').filter(|part| !part.is_empty()) {
        current = tree
            .children(current)
            .find(|&child| tree.name(child) == part.as_bytes())
            .unwrap_or_else(|| panic!("{path} is in the scan"));
    }

    current
}

fn add_at(
    state: &AppState,
    scan: ScanId,
    path: &str,
    verdict: UserVerdictDto,
) -> CommandResult<UserRulesDto> {
    block_on(add(state, scan, node_at(state, scan, path), verdict))
}

fn disk() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/mnt/cache/blob", 400 * MIB)
        .file("/mnt/photos/img", 300 * MIB)
        .file("/mnt/data/readme.txt", 10)
        .file("/var/cache/pacman/pkg/a.zst", 500 * MIB)
        .file("/etc/big/q", 300 * MIB);

    fs
}

fn code_of<T: std::fmt::Debug>(result: CommandResult<T>) -> ApiErrorCode {
    result.unwrap_err().code()
}

fn suggestions_of(state: &AppState, scan: ScanId) -> Vec<SuggestionDto> {
    let view = state.user_rules.view();

    suggestions::suggestions(&state.scans, &state.platform, scan, false, &view)
        .unwrap()
        .suggestions
}

#[test]
fn the_list_has_the_wire_shape_of_the_contract() {
    let (state, _dir, _) = state();
    let scan = register(&state, &disk());

    let empty = block_on(list(&state)).unwrap();
    assert_eq!(serde_json::to_value(&empty).unwrap(), json!({ "rules": [] }));

    add_at(&state, scan, "/mnt/cache", UserVerdictDto::Keep).unwrap();
    let listed = add_at(&state, scan, "/mnt/photos", UserVerdictDto::Safe).unwrap();

    assert_eq!(
        serde_json::to_value(&listed).unwrap(),
        json!({ "rules": [
            { "id": 1, "path": "/mnt/cache", "verdict": "keep" },
            { "id": 2, "path": "/mnt/photos", "verdict": "safe" },
        ] })
    );
}

#[test]
fn a_rule_for_a_folder_that_has_one_replaces_the_verdict() {
    let (state, _dir, _) = state();
    let scan = register(&state, &disk());

    add_at(&state, scan, "/mnt/cache", UserVerdictDto::Keep).unwrap();
    let again = add_at(&state, scan, "/mnt/cache", UserVerdictDto::Safe).unwrap();

    assert_eq!(again.rules.len(), 1);
    assert_eq!(
        (again.rules[0].id, again.rules[0].verdict),
        (1, UserVerdictDto::Safe)
    );
}

#[test]
fn only_a_folder_that_the_scan_has_can_get_a_rule() {
    let (state, _dir, path) = state();
    let scan = register(&state, &disk());
    let file = node_at(&state, scan, "/mnt/data/readme.txt");

    assert_eq!(
        code_of(block_on(add(&state, scan, file, UserVerdictDto::Keep))),
        ApiErrorCode::Invalid,
        "a file"
    );
    assert_eq!(
        code_of(block_on(add(&state, scan, NodeId(999_999), UserVerdictDto::Keep))),
        ApiErrorCode::NotFound
    );
    assert_eq!(
        code_of(block_on(add(&state, 99, NodeId(0), UserVerdictDto::Keep))),
        ApiErrorCode::NotFound,
        "an unknown scan"
    );
    let root = state.scans.tree(scan).unwrap().root();
    for verdict in [UserVerdictDto::Keep, UserVerdictDto::Safe] {
        assert_eq!(
            code_of(block_on(add(&state, scan, root, verdict))),
            ApiErrorCode::Invalid,
            "`/` never gets a rule ({verdict:?})"
        );
    }

    assert!(!path.exists(), "no refusal wrote the file");
}

#[test]
fn a_folder_whose_name_is_not_text_is_refused() {
    use std::sync::Arc;

    let (state, _dir, path) = state();
    let tree = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(tree.path()).unwrap();
    let odd = platter_platform::testing::non_utf8_path(&root);
    fs::create_dir(&odd).unwrap();
    fs::write(odd.join("x"), b"x").unwrap();

    let (sender, receiver) = mpsc::channel();
    let scan = block_on(scan::start(
        &state,
        ScanOptions {
            root: root.to_string_lossy().into_owned(),
            one_file_system: false,
        },
        Arc::new(move |event| {
            let _ = sender.send(event);
        }),
    ))
    .unwrap();
    while !matches!(
        receiver.recv_timeout(Duration::from_secs(30)).unwrap(),
        ScanEvent::Finished { .. }
    ) {}

    let node = state
        .scans
        .tree(scan)
        .unwrap()
        .children(state.scans.tree(scan).unwrap().root())
        .next()
        .unwrap();

    for verdict in [UserVerdictDto::Keep, UserVerdictDto::Safe] {
        assert_eq!(
            code_of(block_on(add(&state, scan, node, verdict))),
            ApiErrorCode::Invalid
        );
    }
    assert!(!path.exists());
}

#[test]
fn a_safe_rule_is_refused_for_the_scan_root_the_home_folder_and_protected_places() {
    let (state, _dir, path) = state();
    let home = state.platform.environment().home;
    let mut fs = disk();
    fs.file("/usr/lib/x/y", MIB);
    if let Some(home) = &home {
        fs.file(&format!("{}/.ssh/keys/id", home.display()), 1);
        fs.file(&format!("{}/stuff/big/f", home.display()), MIB);
    }
    let scan = register(&state, &fs);

    let mut refused = vec!["/etc/big".to_owned(), "/usr/lib/x".to_owned(), "/etc".to_owned()];
    if let Some(home) = &home {
        refused.push(home.display().to_string());
        refused.push(format!("{}/.ssh/keys", home.display()));
    }
    for folder in &refused {
        assert_eq!(
            code_of(add_at(&state, scan, folder, UserVerdictDto::Safe)),
            ApiErrorCode::Invalid,
            "{folder}"
        );
    }
    assert!(!path.exists(), "nothing was written");

    // Keeping is always allowed, wherever the folder is.
    add_at(&state, scan, "/etc/big", UserVerdictDto::Keep).unwrap();
    if let Some(home) = &home {
        add_at(
            &state,
            scan,
            &format!("{}/.ssh/keys", home.display()),
            UserVerdictDto::Keep,
        )
        .unwrap();
        add_at(
            &state,
            scan,
            &format!("{}/stuff/big", home.display()),
            UserVerdictDto::Safe,
        )
        .unwrap();
    }

    // The scan root, when it is a folder inside the disk.
    let inner = {
        let config = platter_core::ScanConfig {
            now: Some(platter_core::testing::NOW),
            ..platter_core::ScanConfig::default()
        };
        let tree = platter_core::scan(
            &fs,
            Path::new("/mnt/cache"),
            &config,
            &platter_core::CancelToken::new(),
            &|_| {},
        )
        .unwrap();
        let (id, _) = state.scans.begin().unwrap();
        state.scans.finish(id, tree).unwrap();
        id
    };
    let root = state.scans.tree(inner).unwrap().root();
    assert_eq!(
        code_of(block_on(add(&state, inner, root, UserVerdictDto::Safe))),
        ApiErrorCode::Invalid,
        "the scan root"
    );
    block_on(add(&state, inner, root, UserVerdictDto::Keep)).expect("keeping the scan root is fine");
}

#[test]
fn the_list_is_capped_at_256_rules() {
    let (state, _dir, _) = state();
    let mut fs = FakeFs::new();
    for index in 0..=MAX_USER_RULES {
        fs.file(&format!("/mnt/d{index}/f"), 1);
    }
    let scan = register(&state, &fs);

    for index in 0..MAX_USER_RULES {
        add_at(&state, scan, &format!("/mnt/d{index}"), UserVerdictDto::Keep).unwrap();
    }

    assert_eq!(
        code_of(add_at(
            &state,
            scan,
            &format!("/mnt/d{MAX_USER_RULES}"),
            UserVerdictDto::Keep
        )),
        ApiErrorCode::Invalid
    );
    assert_eq!(block_on(list(&state)).unwrap().rules.len(), MAX_USER_RULES);
    add_at(&state, scan, "/mnt/d0", UserVerdictDto::Safe).expect("an existing folder can still change");
}

#[test]
fn removing_a_rule_and_an_unknown_id() {
    let (state, _dir, _) = state();
    let scan = register(&state, &disk());
    add_at(&state, scan, "/mnt/cache", UserVerdictDto::Keep).unwrap();

    let same = block_on(remove(&state, 42)).unwrap();
    let removed = block_on(remove(&state, 1)).unwrap();

    assert_eq!(same.rules.len(), 1);
    assert!(removed.rules.is_empty());
}

#[test]
fn the_suggestions_follow_the_rules_and_are_made_again_when_a_rule_changes() {
    let (state, _dir, _) = state();
    let scan = register(&state, &disk());
    let find = |suggestions: &[SuggestionDto], id: &str| suggestions.iter().find(|s| s.id == id).cloned();

    let before = suggestions_of(&state, scan);
    assert!(find(&before, "user-keep").is_none() && find(&before, "user-safe").is_none());
    assert_eq!(
        find(&before, "pacman-cache").unwrap().source,
        SuggestionSource::Rule
    );

    add_at(&state, scan, "/var/cache/pacman/pkg", UserVerdictDto::Keep).unwrap();
    let kept = suggestions_of(&state, scan);
    assert!(
        find(&kept, "pacman-cache").is_none(),
        "the user's keep wins over the built-in rule"
    );
    let keep = find(&kept, "user-keep").unwrap();
    assert_eq!(
        (keep.verdict, keep.source),
        (Verdict::Keep, SuggestionSource::User)
    );
    assert_eq!(keep.allocated, 500 * MIB);

    add_at(&state, scan, "/mnt/cache", UserVerdictDto::Safe).unwrap();
    let safe = find(&suggestions_of(&state, scan), "user-safe").unwrap();
    assert_eq!(
        (safe.verdict, safe.source),
        (Verdict::Safe, SuggestionSource::User)
    );
    assert_eq!(safe.allocated, 400 * MIB);

    block_on(remove(&state, 1)).unwrap();
    let removed = suggestions_of(&state, scan);
    assert!(find(&removed, "user-keep").is_none());
    assert!(
        find(&removed, "pacman-cache").is_some(),
        "the built-in rule is back"
    );
}

/// The disk of a [`FakeFs`] where `/mnt/link` leads to `/var/cache`.
struct Linked<'a>(&'a FakeFs);

impl platter_core::EntryProbe for Linked<'_> {
    fn probe(&self, path: &[u8]) -> std::io::Result<Vec<platter_core::ProbedEntry>> {
        self.0.probe(path)
    }

    fn resolve(&self, path: &[u8]) -> std::io::Result<Vec<u8>> {
        if path == b"/mnt/link" {
            return Ok(b"/var/cache".to_vec());
        }

        Ok(path.to_vec())
    }
}

#[test]
fn a_suggestion_is_not_listed_inside_a_folder_kept_through_a_link() {
    let (state, _dir, _) = state();
    let mut fs = disk();
    fs.symlink("/mnt/link", 10);
    let keep_the_link = crate::user_rules::UserRulesView {
        generation: 1,
        rules: vec![platter_core::UserRule {
            id: 1,
            verdict: platter_core::UserVerdict::Keep,
            folder: "/mnt/link".into(),
        }],
        problem: None,
    };
    fn ids(
        state: &AppState,
        fs: &FakeFs,
        probe: &impl platter_core::EntryProbe,
        view: &crate::user_rules::UserRulesView,
    ) -> Vec<String> {
        // A scan of its own each time: the suggestions are kept with the scan.
        let scan = register(state, fs);

        suggestions::suggestions_with(&state.scans, &state.platform, probe, scan, false, view)
            .unwrap()
            .suggestions
            .into_iter()
            .map(|suggestion| suggestion.id)
            .collect()
    }

    // Where the link leads is not known (the disk cannot say): kept by its own path alone, as before.
    let unknown = ids(&state, &fs, &fs, &keep_the_link);
    assert!(unknown.contains(&"pacman-cache".to_owned()), "{unknown:?}");

    let known = ids(&state, &fs, &Linked(&fs), &keep_the_link);
    assert!(
        !known.contains(&"pacman-cache".to_owned()),
        "{known:?}: a plan would refuse it as kept"
    );
    assert!(known.contains(&"user-keep".to_owned()), "{known:?}");
}

#[test]
fn a_user_safe_never_beats_a_builtin_rule_on_the_same_folder() {
    let (state, _dir, _) = state();
    let scan = register(&state, &disk());

    add_at(&state, scan, "/var/cache/pacman/pkg", UserVerdictDto::Safe).unwrap();
    let suggestions = suggestions_of(&state, scan);

    assert!(suggestions.iter().all(|s| s.id != "user-safe"));
    assert!(suggestions.iter().any(|s| s.id == "pacman-cache"));
}

#[test]
fn a_change_of_the_rules_drops_the_waiting_plans() {
    let (state, _dir, path) = state();
    let scan = register(&state, &disk());
    let waiting = || {
        state
            .cleanups
            .store_made_with(scan, platter_cleanup::CleanupPlan::default(), 0, |_| true)
            .unwrap()
    };

    let plan = waiting();
    add_at(&state, scan, "/mnt/cache", UserVerdictDto::Keep).unwrap();
    assert!(!state.cleanups.has_plan(plan), "adding a rule");

    let plan = waiting();
    block_on(remove(&state, 1)).unwrap();
    assert!(!state.cleanups.has_plan(plan), "removing a rule");

    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{ nope").unwrap();
    let plan = waiting();
    block_on(reset(&state)).unwrap();
    assert!(!state.cleanups.has_plan(plan), "starting over");
}

#[test]
fn a_plan_made_from_rules_that_have_changed_since_is_not_stored() {
    let (state, _dir, _) = state();
    let scan = register(&state, &disk());
    let made_from = state.user_rules.view().generation;
    add_at(&state, scan, "/mnt/cache", UserVerdictDto::Keep).unwrap();

    let stale =
        state
            .cleanups
            .store_made_with(scan, platter_cleanup::CleanupPlan::default(), made_from, |made| {
                state.user_rules.generation() == made
            });

    assert_eq!(code_of(stale), ApiErrorCode::Invalid);
}

#[test]
fn the_rules_are_not_read_while_the_registry_is_locked_and_a_late_change_takes_the_plan_back() {
    let (state, _dir, _) = state();
    let scan = register(&state, &disk());
    let asked = std::sync::atomic::AtomicU32::new(0);

    // Asking the registry from inside the question would wait for its own lock if it were held.
    let stored = state
        .cleanups
        .store_made_with(scan, platter_cleanup::CleanupPlan::default(), 0, |_| {
            asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            !state.cleanups.has_plan(u64::MAX)
        })
        .unwrap();
    assert!(state.cleanups.has_plan(stored));
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "before and after storing"
    );

    // The rules change after the first look: the plan is taken out again, not left stored.
    let calls = std::sync::atomic::AtomicU32::new(0);
    let late = state
        .cleanups
        .store_made_with(scan, platter_cleanup::CleanupPlan::default(), 0, |_| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
        });

    assert_eq!(code_of(late), ApiErrorCode::Invalid);
    assert!(state.cleanups.has_plan(stored), "the earlier plan is untouched");
    assert!(!state.cleanups.has_plan(stored + 1), "the late plan is gone");
}

#[test]
fn a_broken_file_is_listed_with_its_problem_and_start_over_moves_it_aside() {
    let (state, _dir, path) = state();
    let scan = register(&state, &disk());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{ nope").unwrap();

    let listed = block_on(list(&state)).unwrap();
    let wire = serde_json::to_value(&listed).unwrap();
    assert!(wire["rules"].as_array().unwrap().is_empty());
    assert!(wire["problem"].as_str().unwrap().contains("rules file"), "{wire}");

    assert_eq!(
        code_of(add_at(&state, scan, "/mnt/cache", UserVerdictDto::Keep)),
        ApiErrorCode::Invalid
    );
    assert_eq!(code_of(block_on(remove(&state, 1))), ApiErrorCode::Invalid);
    assert_eq!(fs::read(&path).unwrap(), b"{ nope", "never overwritten");

    let fresh = block_on(reset(&state)).unwrap();
    assert_eq!(serde_json::to_value(&fresh).unwrap(), json!({ "rules": [] }));
    assert_eq!(
        fs::read(path.with_file_name("rules.json.broken")).unwrap(),
        b"{ nope"
    );
    assert_eq!(
        code_of(block_on(reset(&state))),
        ApiErrorCode::Invalid,
        "only when broken"
    );
    add_at(&state, scan, "/mnt/cache", UserVerdictDto::Keep).unwrap();
}

#[test]
fn asking_about_a_folder_a_user_rule_covers_answers_in_the_users_words() {
    let (state, _dir, _) = state();
    let mut fs = disk();
    fs.file("/mnt/cache/deep/f", MIB);
    let scan = register(&state, &fs);
    add_at(&state, scan, "/mnt/cache", UserVerdictDto::Keep).unwrap();
    add_at(&state, scan, "/mnt/photos", UserVerdictDto::Safe).unwrap();
    let ask = |path: &str| block_on(ask_about(&state, scan, node_at(&state, scan, path))).unwrap();

    let kept = ask("/mnt/cache");
    assert_eq!(
        (kept.source, kept.verdict),
        (AnswerSourceDto::User, Verdict::Keep)
    );
    assert!(
        kept.detail.starts_with("Your rule: Folders you keep. "),
        "{}",
        kept.detail
    );
    assert!(kept.reason.starts_with("Chosen by you: "), "{}", kept.reason);
    let inside = ask("/mnt/cache/deep");
    assert_eq!(
        (inside.source, inside.verdict),
        (AnswerSourceDto::User, Verdict::Keep)
    );
    let safe = ask("/mnt/photos");
    assert_eq!(
        (safe.source, safe.verdict),
        (AnswerSourceDto::User, Verdict::Safe)
    );
    assert!(
        safe.detail.starts_with("Your rule: Folders you marked safe. "),
        "{}",
        safe.detail
    );

    let wire = serde_json::to_value(&kept).unwrap();
    assert_eq!(wire["source"], "user");
    let builtin = ask("/var/cache/pacman/pkg");
    assert_eq!(
        builtin.source,
        AnswerSourceDto::Rule,
        "built-in rules still answer as rules"
    );
}

#[test]
fn a_label_says_what_the_user_decided_about_its_folder() {
    let (state, _dir, _) = state();
    let scan = register(&state, &disk());
    let labels = ["/mnt/cache", "/mnt/photos", "/mnt/data"]
        .iter()
        .map(|path| ModelLabel {
            node: node_at(&state, scan, path),
            verdict: ModelVerdict::Review,
            reason: "because".to_owned(),
        })
        .collect();
    state.scans.set_labels(
        scan,
        LabelSet {
            labels,
            model_ms: 1,
            stopped: None,
        },
    );
    add_at(&state, scan, "/mnt/cache", UserVerdictDto::Keep).unwrap();
    add_at(&state, scan, "/mnt/photos", UserVerdictDto::Safe).unwrap();

    let shown = block_on(advisor_labels(&state, scan)).unwrap();

    let decided = |path: &str| {
        shown
            .labels
            .iter()
            .find(|label| label.path == path)
            .map(|label| label.decided)
    };
    assert_eq!(decided("/mnt/cache"), Some(Some(UserVerdictDto::Keep)));
    assert_eq!(decided("/mnt/photos"), Some(Some(UserVerdictDto::Safe)));
    assert_eq!(decided("/mnt/data"), Some(None));
    let wire = serde_json::to_value(&shown.labels).unwrap();
    assert!(wire.to_string().contains(r#""decided":"keep""#), "{wire}");
    assert!(
        wire.as_array()
            .unwrap()
            .iter()
            .any(|label| label.get("decided").is_none()),
        "no `decided` when the user has no rule"
    );
}

#[test]
fn asking_about_a_kept_folder_with_no_bytes_answers_that_it_is_kept() {
    let (state, _dir, _) = state();
    let home = state.platform.environment().home;
    let mut fs = disk();
    // A folder of empty files and links, and one of hard links charged to another folder, have no bytes.
    fs.file("/mnt/zero/.gitkeep", 0)
        .symlink("/mnt/zero/link", 0)
        .file("/mnt/snapshots/2024/pack", 4 * MIB)
        .hardlink("/mnt/snapshots/2024/pack", "/mnt/snapshots/2025/pack");
    if let Some(home) = &home {
        fs.file(
            &format!(
                "{}/.local/share/Steam/steamapps/common/game/empty",
                home.display()
            ),
            0,
        );
    }
    let scan = register(&state, &fs);
    for folder in ["/mnt/zero", "/mnt/snapshots/2025"] {
        add_at(&state, scan, folder, UserVerdictDto::Keep).unwrap();
    }
    let ask = |path: &str| {
        block_on(ask_about(&state, scan, node_at(&state, scan, path)))
            .unwrap_or_else(|error| panic!("{path}: {error:?}"))
    };

    for folder in ["/mnt/zero", "/mnt/snapshots/2025"] {
        let answer = ask(folder);
        assert_eq!(
            (answer.source, answer.verdict),
            (AnswerSourceDto::User, Verdict::Keep),
            "{folder}"
        );
        assert!(
            answer.detail.starts_with("Your rule: Folders you keep. "),
            "{folder}: {}",
            answer.detail
        );
    }
    if let Some(home) = &home {
        let library = ask(&format!("{}/.local/share/Steam/steamapps/common", home.display()));
        assert_eq!(
            (library.source, library.verdict),
            (AnswerSourceDto::Rule, Verdict::Keep),
            "a built-in keep with no bytes is still kept"
        );
    }
}

#[test]
fn while_the_rules_file_is_broken_nothing_is_labelled_or_asked_but_stored_labels_are_shown() {
    let (state, _dir, path) = state();
    let scan = register(&state, &disk());
    let label = ModelLabel {
        node: node_at(&state, scan, "/mnt/photos"),
        verdict: ModelVerdict::Review,
        reason: "because".to_owned(),
    };
    state.scans.set_labels(
        scan,
        LabelSet {
            labels: vec![label],
            model_ms: 1,
            stopped: None,
        },
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{ nope").unwrap();

    let labelling = block_on(label_scan(&state, scan)).unwrap_err();
    let asking = block_on(ask_about(&state, scan, node_at(&state, scan, "/mnt/photos"))).unwrap_err();
    let shown = block_on(advisor_labels(&state, scan)).unwrap();

    for error in [labelling, asking] {
        assert_eq!(error.code(), ApiErrorCode::Invalid);
        let wire = serde_json::to_value(&error).unwrap();
        assert!(
            wire["message"]
                .as_str()
                .is_some_and(|text| text.contains("Your rules file cannot be used")),
            "{wire}"
        );
    }
    assert_eq!(shown.labels.len(), 1, "what was labelled before is still shown");
}

#[test]
fn a_label_on_the_parent_of_a_kept_folder_with_no_bytes_is_not_listed_for_a_plan() {
    let (state, _dir, _) = state();
    let mut fs = disk();
    fs.file("/mnt/snapshots/2024/pack", 4 * MIB)
        .hardlink("/mnt/snapshots/2024/pack", "/mnt/snapshots/2025/pack");
    let scan = register(&state, &fs);
    add_at(&state, scan, "/mnt/snapshots/2025", UserVerdictDto::Keep).unwrap();
    let tree = state.scans.tree(scan).unwrap();
    let labels: Vec<ModelLabel> = ["/mnt/snapshots", "/mnt/photos"]
        .iter()
        .map(|path| ModelLabel {
            node: node_at(&state, scan, path),
            verdict: ModelVerdict::Review,
            reason: String::new(),
        })
        .collect();

    let review = crate::commands::advisor::Review::new(&tree, &state.platform, state.user_rules.view().rules);
    let listed: Vec<NodeId> = review
        .listed_labels(&labels)
        .iter()
        .map(|label| label.node)
        .collect();

    assert_eq!(listed, [node_at(&state, scan, "/mnt/photos")]);
}
