//! Backend state: the platform adapters and the registry of scans addressed by `ScanId`.
//!
//! Retention policy (bounds memory):
//! - suggestions are computed once per finished scan and kept next to its tree, so they are dropped with it;
//!   the stored result is valid for one [`SuggestionsKey`] (the labels it was made from, the advisor
//!   option and the generation of the user's rules) and is made again when that changes;
//! - the local model's labels of a scan live and die with it, and are also dropped when a cleanup (or an
//!   undo) outdates the scan: labels of a tree that no longer describes the disk would name folders that
//!   may be gone;
//! - at most one finished tree is kept. When a scan finishes, every other finished tree is dropped, so
//!   the UI always has the latest result and a rescan does not hold two multi-hundred-MB trees;
//! - at most [`MAX_RUNNING`] scans run at once; `start` beyond that is rejected as `invalid`;
//! - a scan that is cancelled or fails leaves nothing behind;
//! - `cancel` is a no-op for an unknown or already ended scan, like `drop_scan`: a cancel can race with
//!   the scan finishing, and neither outcome is an error for the UI;
//! - `drop_scan` is idempotent: dropping an id that is already gone (evicted, never existed) is fine,
//!   because eviction is invisible to the UI and it drops the previous scan after starting a new one.
//!
//! Ids start at 1 and only grow; an id is never reused, so a stale id can never address a newer scan.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use platter_ai::Stopped;
use platter_core::dto::{ScanId, SuggestionsDto};
use platter_core::{CancelToken, ModelLabel, Tree};
use platter_platform::Platform;

use crate::commands::advisor::AdvisorService;
use crate::commands::advisor::log::QuestionLog;
use crate::commands::cleanup::{CleanupRegistry, Confirm, RefuseAll};
use crate::commands::history::{FixedVolumes, HistoryService};
use crate::error::{CommandError, CommandResult};
use crate::user_rules::UserRulesStore;

/// Scans that may run at the same time.
pub const MAX_RUNNING: usize = 4;

pub struct AppState {
    pub platform: Arc<Platform>,
    pub scans: Arc<ScanRegistry>,
    pub cleanups: Arc<CleanupRegistry>,
    /// The history file: usage per day, scans and cleanups (`history.sqlite`).
    pub history: Arc<HistoryService>,
    /// The local model: its download, its memory, its options.
    pub advisor: Arc<AdvisorService>,
    /// The user's own rules (`rules.json`): what they say now, and the only way they change.
    pub user_rules: Arc<UserRulesStore>,
    /// The local log of free questions (`advisor-questions.jsonl`), written only when the user's switch is on.
    pub question_log: Arc<QuestionLog>,
    /// How many `find_entries` calls have started. A walk over the tree stops once a newer call has taken
    /// a higher number, so the walks of keystrokes that were typed over do not pile up.
    pub entry_searches: Arc<AtomicU64>,
    /// The scan roots the backend handed to the UI itself: volume mount points from `list_volumes`
    /// and folders from its own folder dialog. `start_scan` accepts nothing else.
    pub roots: Mutex<HashMap<String, Option<(u64, u64)>>>,
    /// Asks the user before a cleanup that cannot be undone. Set by the app at startup; until then
    /// (and in tests that set none) nothing is approved.
    confirm: OnceLock<Arc<dyn Confirm>>,
}

impl AppState {
    pub fn new() -> Self {
        let mut state = Self::with_parts(AdvisorService::production(), UserRulesStore::production());
        state.question_log = Arc::new(QuestionLog::production());
        state.history = Arc::new(HistoryService::production());

        state
    }

    /// The state with another advisor (tests give it a fake model and a fake server) and no rules file:
    /// the user's rules are empty and cannot be added.
    #[cfg(test)]
    pub fn with_advisor(advisor: AdvisorService) -> Self {
        Self::with_parts(advisor, UserRulesStore::new(None)).with_test_history()
    }

    /// What most tests use: an advisor with no model runtime and no graphics card problem log, and no rules
    /// file (the user's real rules and logs are never read, and no helper process is ever started).
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self::with_parts(AdvisorService::for_test(), UserRulesStore::new(None)).with_test_history()
    }

    /// The same state with a history in memory on a disk that holds every path (`/`, 1000 bytes, 400 free).
    #[cfg(test)]
    pub fn with_test_history(mut self) -> Self {
        self.history = Arc::new(HistoryService::in_memory(FixedVolumes::root()));
        self
    }

    /// The state with another advisor and another place for the user's rules, no question log (a test
    /// that looks at the log sets `question_log`) and a history in memory that lists no disk (production
    /// sets its own).
    pub fn with_parts(advisor: AdvisorService, user_rules: UserRulesStore) -> Self {
        Self {
            platform: Arc::new(Platform::current()),
            scans: Arc::new(ScanRegistry::default()),
            cleanups: Arc::new(CleanupRegistry::default()),
            history: Arc::new(HistoryService::in_memory(FixedVolumes::default())),
            advisor: Arc::new(advisor),
            user_rules: Arc::new(user_rules),
            question_log: Arc::new(QuestionLog::new(None)),
            entry_searches: Arc::new(AtomicU64::new(0)),
            roots: Mutex::new(HashMap::new()),
            confirm: OnceLock::new(),
        }
    }

    /// Sets the confirmation once; later calls are ignored.
    pub fn set_confirm(&self, confirm: Arc<dyn Confirm>) {
        let _ = self.confirm.set(confirm);
    }

    pub fn confirm(&self) -> Arc<dyn Confirm> {
        self.confirm.get().cloned().unwrap_or_else(|| Arc::new(RefuseAll))
    }

    /// Remembers roots the backend offered (mount points).
    pub fn allow_roots(&self, roots: impl IntoIterator<Item = String>) {
        let mut allowed = self.roots.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for root in roots {
            allowed.entry(root).or_insert(None);
        }
    }

    /// Remembers a folder the user picked, with the identity (device, inode) it had then: a scan of it
    /// must find the same folder there.
    pub fn allow_picked(&self, root: String, identity: Option<(u64, u64)>) {
        let mut allowed = self.roots.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        allowed.insert(root, identity);
    }

    /// `None` when `root` was not offered; else the identity it must have (`None` inside: any).
    pub fn allowed_root(&self, root: &str) -> Option<Option<(u64, u64)>> {
        self.roots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(root)
            .copied()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

enum Slot {
    /// A running scan and the disk epoch it began in.
    Running(CancelToken, u64),
    Finished(Finished),
}

/// What the local model said about the folders of one scan in its last labelling run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LabelSet {
    pub labels: Vec<ModelLabel>,
    /// How long the model ran, in milliseconds.
    pub model_ms: u64,
    /// Why the run ended before every candidate was asked about.
    pub stopped: Option<Stopped>,
}

impl LabelSet {
    /// The folders the model answered about.
    pub fn labelled(&self) -> usize {
        self.labels.len()
    }
}

/// The labels of a scan and the number of times they changed.
#[derive(Default)]
struct ScanLabels {
    generation: u64,
    current: Option<Arc<LabelSet>>,
}

impl ScanLabels {
    fn replace(&mut self, current: Option<Arc<LabelSet>>) {
        self.generation += 1;
        self.current = current;
    }
}

/// What a stored result of [`ScanRegistry::suggestions`] was made from. A result is used again only for
/// the same key; any difference makes it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SuggestionsKey {
    labels_generation: u64,
    advisor_enabled: bool,
    user_rules_generation: u64,
}

/// A finished scan: its tree, the suggestions computed from it, and the model's labels of it.
#[derive(Clone)]
struct Finished {
    tree: Arc<Tree>,
    /// The result for the key it was made for. Held while a result is made, so concurrent first calls
    /// wait for it instead of repeating it.
    suggestions: Arc<Mutex<Option<(SuggestionsKey, SuggestionsDto)>>>,
    labels: Arc<Mutex<ScanLabels>>,
    /// A cleanup ran on this scan's disk: the tree no longer describes it.
    outdated: Arc<AtomicBool>,
}

#[derive(Default)]
struct Inner {
    last_id: ScanId,
    /// Counts the times a cleanup (or an undo) changed the disk. A scan that began before the latest
    /// change finishes outdated.
    epoch: u64,
    slots: HashMap<ScanId, Slot>,
}

#[derive(Default)]
pub struct ScanRegistry {
    inner: Mutex<Inner>,
}

impl ScanRegistry {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave `Inner` half-updated (each method does one
        // insert/remove), so recover from poisoning instead of taking every command down.
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Allocates the next id and registers a running scan under it.
    pub fn begin(&self) -> CommandResult<(ScanId, CancelToken)> {
        let mut inner = self.lock();

        let running = inner
            .slots
            .values()
            .filter(|slot| matches!(slot, Slot::Running(..)))
            .count();
        if running >= MAX_RUNNING {
            return Err(CommandError::invalid(format!(
                "{MAX_RUNNING} scans are already running, so cancel one first"
            )));
        }

        inner.last_id += 1;
        let id = inner.last_id;
        let cancel = CancelToken::new();
        let epoch = inner.epoch;
        inner.slots.insert(id, Slot::Running(cancel.clone(), epoch));

        Ok((id, cancel))
    }

    /// Stores the tree of a running scan and evicts every other finished tree.
    ///
    /// Returns `None` (and discards the tree) when the scan was dropped while it was running.
    pub fn finish(&self, id: ScanId, tree: Tree) -> Option<Arc<Tree>> {
        let mut inner = self.lock();

        let Some(Slot::Running(_, began)) = inner.slots.get(&id) else {
            return None;
        };
        let outdated = *began < inner.epoch;

        inner.slots.retain(|_, slot| !matches!(slot, Slot::Finished(_)));

        let tree = Arc::new(tree);
        inner.slots.insert(
            id,
            Slot::Finished(Finished {
                tree: Arc::clone(&tree),
                suggestions: Arc::default(),
                labels: Arc::default(),
                outdated: Arc::new(AtomicBool::new(outdated)),
            }),
        );

        Some(tree)
    }

    /// Forgets a running scan that ended without a tree (cancelled or failed).
    pub fn abandon(&self, id: ScanId) {
        let mut inner = self.lock();

        if matches!(inner.slots.get(&id), Some(Slot::Running(..))) {
            inner.slots.remove(&id);
        }
    }

    /// Raises the cancel flag of a running scan. A finished or unknown scan is left alone.
    pub fn cancel(&self, id: ScanId) {
        if let Some(Slot::Running(cancel, _)) = self.lock().slots.get(&id) {
            cancel.cancel();
        }
    }

    /// Frees a scan: cancels it when running, drops the tree when finished. Unknown ids are fine.
    pub fn drop_scan(&self, id: ScanId) {
        if let Some(Slot::Running(cancel, _)) = self.lock().slots.remove(&id) {
            cancel.cancel();
        }
    }

    /// The finished tree of a scan.
    pub fn tree(&self, id: ScanId) -> CommandResult<Arc<Tree>> {
        Ok(self.finished(id)?.tree)
    }

    /// The suggestions of a finished scan. `compute` runs the first time and again whenever the labels, the
    /// advisor option or the user's rules (`user_rules_generation`) changed since (concurrent calls wait for it instead of repeating it); otherwise
    /// the stored result is returned. It gets the model's labels of the scan, none while the advisor is
    /// switched off. It may take long, so call this off the async runtime. The result lives and dies with
    /// the scan.
    pub fn suggestions(
        &self,
        id: ScanId,
        advisor_enabled: bool,
        user_rules_generation: u64,
        compute: impl FnOnce(&Tree, &[ModelLabel]) -> SuggestionsDto,
    ) -> CommandResult<SuggestionsDto> {
        let finished = self.finished(id)?;

        // The key and the labels come from one look, so a result is never stored under a key whose
        // labels it was not made from. The registry lock is not held while computing: other commands
        // keep working.
        let (generation, set) = {
            let labels = lock(&finished.labels);
            (labels.generation, labels.current.clone())
        };
        let key = SuggestionsKey {
            labels_generation: generation,
            advisor_enabled,
            user_rules_generation,
        };

        let mut stored = lock(&finished.suggestions);
        if let Some((made_for, suggestions)) = stored.as_ref()
            && *made_for == key
        {
            return Ok(suggestions.clone());
        }

        let labels: &[ModelLabel] = match (&set, advisor_enabled) {
            (Some(set), true) => &set.labels,
            _ => &[],
        };
        let suggestions = compute(&finished.tree, labels);
        *stored = Some((key, suggestions.clone()));

        Ok(suggestions)
    }

    /// Stores the labels of a labelling run for a finished scan, replacing the earlier ones. `false`
    /// (and nothing stored) when the scan is gone, still running or outdated since the run began.
    ///
    /// A run that stopped early (cancelled, out of time) before it made a single label says nothing about
    /// the folders, so it does not replace the labels of an earlier run: they stay, with their own
    /// `stopped` and `model_ms`. A run that was asked about every folder replaces them, even with none.
    pub fn set_labels(&self, id: ScanId, set: LabelSet) -> bool {
        // The registry lock is held, so a cleanup cannot outdate the scan between the check and the store.
        let inner = self.lock();
        let Some(Slot::Finished(finished)) = inner.slots.get(&id) else {
            return false;
        };
        if finished.outdated.load(Ordering::SeqCst) {
            return false;
        }

        let mut labels = lock(&finished.labels);
        if set.labels.is_empty() && set.stopped.is_some() && labels.current.is_some() {
            return true;
        }

        labels.replace(Some(Arc::new(set)));
        true
    }

    /// The labels of a finished scan; `None` when it has none (no run yet, or dropped as outdated).
    pub fn labels(&self, id: ScanId) -> CommandResult<Option<Arc<LabelSet>>> {
        let finished = self.finished(id)?;
        let labels = lock(&finished.labels).current.clone();

        Ok(labels)
    }

    /// Whether the scan is registered and has not finished yet.
    pub fn is_running(&self, id: ScanId) -> bool {
        matches!(self.lock().slots.get(&id), Some(Slot::Running(..)))
    }

    /// A cleanup or an undo changed the disk: every finished scan is outdated, and so is every scan
    /// still running, when it finishes (it may have read the disk before the change).
    pub fn disk_changed(&self) {
        let mut inner = self.lock();
        inner.epoch += 1;
        for slot in inner.slots.values() {
            if let Slot::Finished(finished) = slot {
                finished.outdated.store(true, Ordering::SeqCst);
                // Labels name folders of a tree that no longer describes the disk.
                lock(&finished.labels).replace(None);
            }
        }
    }

    /// Whether a cleanup ran on this scan since it finished.
    pub fn is_outdated(&self, id: ScanId) -> bool {
        matches!(self.lock().slots.get(&id), Some(Slot::Finished(finished)) if finished.outdated.load(Ordering::SeqCst))
    }

    fn finished(&self, id: ScanId) -> CommandResult<Finished> {
        match self.lock().slots.get(&id) {
            Some(Slot::Finished(finished)) => Ok(finished.clone()),
            Some(Slot::Running(..)) => Err(CommandError::not_found(format!("scan {id} has not finished"))),
            None => Err(unknown_scan(id)),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unknown_scan(id: ScanId) -> CommandError {
    CommandError::not_found(format!("scan {id} does not exist"))
}

#[cfg(test)]
mod tests {
    use platter_core::dto::ApiErrorCode;
    use platter_core::testing::{FakeFs, scan_fake};
    use platter_core::{ModelVerdict, NodeId};

    use super::*;

    fn tiny_tree() -> Tree {
        scan_fake(&FakeFs::new(), false)
    }

    #[test]
    fn ids_start_at_one_and_only_grow() {
        let registry = ScanRegistry::default();

        let (first, _) = registry.begin().unwrap();
        registry.abandon(first);
        let (second, _) = registry.begin().unwrap();

        assert_eq!(first, 1);
        assert_eq!(second, 2);
    }

    #[test]
    fn cancel_raises_the_flag_of_a_running_scan() {
        let registry = ScanRegistry::default();
        let (id, cancel) = registry.begin().unwrap();

        registry.cancel(id);

        assert!(cancel.is_cancelled());
    }

    #[test]
    fn cancel_of_an_unknown_or_finished_scan_is_a_no_op() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();
        registry.finish(id, tiny_tree()).unwrap();

        registry.cancel(id);
        registry.cancel(99);

        assert!(
            registry.tree(id).is_ok(),
            "cancelling a finished scan keeps its tree"
        );
        assert_eq!(registry.tree(99).unwrap_err().code(), ApiErrorCode::NotFound);
    }

    #[test]
    fn a_running_scan_has_no_tree_yet() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();

        assert_eq!(registry.tree(id).unwrap_err().code(), ApiErrorCode::NotFound);
    }

    #[test]
    fn finishing_a_scan_evicts_the_previous_finished_scan() {
        let registry = ScanRegistry::default();
        let (first, _) = registry.begin().unwrap();
        registry.finish(first, tiny_tree()).unwrap();
        let (second, _) = registry.begin().unwrap();

        assert!(
            registry.tree(first).is_ok(),
            "the old scan stays until the new one finishes"
        );

        registry.finish(second, tiny_tree()).unwrap();

        assert_eq!(registry.tree(first).unwrap_err().code(), ApiErrorCode::NotFound);
        assert!(registry.tree(second).is_ok());
    }

    #[test]
    fn finishing_does_not_touch_other_running_scans() {
        let registry = ScanRegistry::default();
        let (first, _) = registry.begin().unwrap();
        let (second, second_cancel) = registry.begin().unwrap();

        registry.finish(first, tiny_tree()).unwrap();

        assert!(!second_cancel.is_cancelled());
        registry.cancel(second);
        assert!(second_cancel.is_cancelled());
    }

    #[test]
    fn a_scan_dropped_while_running_is_cancelled_and_its_tree_discarded() {
        let registry = ScanRegistry::default();
        let (id, cancel) = registry.begin().unwrap();

        registry.drop_scan(id);

        assert!(cancel.is_cancelled());
        assert!(registry.finish(id, tiny_tree()).is_none());
        assert_eq!(registry.tree(id).unwrap_err().code(), ApiErrorCode::NotFound);
    }

    #[test]
    fn drop_scan_frees_a_finished_tree_and_is_idempotent() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();
        registry.finish(id, tiny_tree()).unwrap();

        registry.drop_scan(id);
        registry.drop_scan(id);
        registry.drop_scan(1234);

        assert_eq!(registry.tree(id).unwrap_err().code(), ApiErrorCode::NotFound);
    }

    #[test]
    fn abandon_removes_a_running_scan_but_keeps_a_finished_one() {
        let registry = ScanRegistry::default();
        let (running, _) = registry.begin().unwrap();
        let (done, _) = registry.begin().unwrap();
        registry.finish(done, tiny_tree()).unwrap();

        registry.abandon(running);
        registry.abandon(done);

        assert_eq!(registry.tree(running).unwrap_err().code(), ApiErrorCode::NotFound);
        assert!(registry.tree(done).is_ok());
    }

    fn empty_suggestions() -> SuggestionsDto {
        SuggestionsDto {
            suggestions: Vec::new(),
            totals: Default::default(),
        }
    }

    #[test]
    fn suggestions_are_computed_once_per_finished_scan() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();
        registry.finish(id, tiny_tree()).unwrap();
        let runs = std::sync::atomic::AtomicUsize::new(0);
        let compute = |_: &Tree, _: &[ModelLabel]| {
            runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            empty_suggestions()
        };

        let first = registry.suggestions(id, true, 0, compute).unwrap();
        let second = registry.suggestions(id, true, 0, compute).unwrap();

        assert_eq!(first, second);
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn suggestions_of_an_unknown_running_or_dropped_scan_are_not_found() {
        let registry = ScanRegistry::default();
        let (running, _) = registry.begin().unwrap();
        let (done, _) = registry.begin().unwrap();
        registry.finish(done, tiny_tree()).unwrap();
        registry.drop_scan(done);

        for id in [running, done, 99] {
            let error = registry
                .suggestions(id, true, 0, |_, _| unreachable!("no tree to judge"))
                .unwrap_err();

            assert_eq!(error.code(), ApiErrorCode::NotFound, "scan {id}");
        }
    }

    #[test]
    fn a_new_scan_does_not_inherit_the_suggestions_of_the_old_one() {
        let registry = ScanRegistry::default();
        let (first, _) = registry.begin().unwrap();
        registry.finish(first, tiny_tree()).unwrap();
        registry
            .suggestions(first, true, 0, |_, _| empty_suggestions())
            .unwrap();
        let (second, _) = registry.begin().unwrap();
        registry.finish(second, tiny_tree()).unwrap();
        let mut computed = false;

        registry
            .suggestions(second, true, 0, |_, _| {
                computed = true;
                empty_suggestions()
            })
            .unwrap();

        assert!(computed);
        assert!(
            registry
                .suggestions(first, true, 0, |_, _| empty_suggestions())
                .is_err()
        );
    }

    fn label(node: u32, verdict: ModelVerdict) -> ModelLabel {
        ModelLabel {
            node: NodeId(node),
            verdict,
            reason: "because".to_owned(),
        }
    }

    fn label_set(labels: Vec<ModelLabel>) -> LabelSet {
        LabelSet {
            labels,
            model_ms: 7,
            stopped: Some(Stopped::Cancelled),
        }
    }

    /// The labels `compute` was given, by the number of runs so far.
    fn suggestions_seeing(registry: &ScanRegistry, id: ScanId, enabled: bool) -> Vec<ModelLabel> {
        let mut seen = Vec::new();
        registry
            .suggestions(id, enabled, 0, |_, labels| {
                seen = labels.to_vec();
                empty_suggestions()
            })
            .unwrap();

        seen
    }

    #[test]
    fn the_labels_of_a_scan_are_stored_replaced_and_read_back() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();
        registry.finish(id, tiny_tree()).unwrap();
        assert_eq!(registry.labels(id).unwrap(), None);

        assert!(registry.set_labels(id, label_set(vec![label(1, ModelVerdict::Review)])));
        let first = registry.labels(id).unwrap().unwrap();
        assert_eq!((first.labelled(), first.model_ms), (1, 7));
        assert_eq!(first.stopped, Some(Stopped::Cancelled));

        assert!(registry.set_labels(id, label_set(vec![label(2, ModelVerdict::Keep)])));
        let second = registry.labels(id).unwrap().unwrap();
        assert_eq!(second.labels, vec![label(2, ModelVerdict::Keep)]);

        let finished_with_none = LabelSet {
            labels: Vec::new(),
            model_ms: 9,
            stopped: None,
        };
        assert!(registry.set_labels(id, finished_with_none));
        let third = registry.labels(id).unwrap().unwrap();
        assert_eq!(
            (third.labelled(), third.model_ms),
            (0, 9),
            "a finished run replaces, even with none"
        );
    }

    #[test]
    fn a_run_that_stopped_early_with_no_label_keeps_the_labels_of_the_earlier_run() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();
        registry.finish(id, tiny_tree()).unwrap();
        let earlier = LabelSet {
            labels: vec![label(1, ModelVerdict::Review)],
            model_ms: 7,
            stopped: Some(Stopped::TimeBudget),
        };
        registry.set_labels(id, earlier.clone());

        for stopped in [Stopped::Cancelled, Stopped::TimeBudget] {
            let empty = LabelSet {
                labels: Vec::new(),
                model_ms: 1,
                stopped: Some(stopped),
            };
            assert!(registry.set_labels(id, empty));

            assert_eq!(*registry.labels(id).unwrap().unwrap(), earlier, "{stopped:?}");
        }

        let (other, _) = registry.begin().unwrap();
        registry.finish(other, tiny_tree()).unwrap();
        registry.set_labels(other, label_set(Vec::new()));
        assert!(
            registry.labels(other).unwrap().is_some(),
            "with no earlier run the empty, stopped one is what there is"
        );
    }

    #[test]
    fn labels_are_refused_for_a_running_unknown_or_outdated_scan_and_go_with_the_scan() {
        let registry = ScanRegistry::default();
        let (running, _) = registry.begin().unwrap();
        let (done, _) = registry.begin().unwrap();
        registry.finish(done, tiny_tree()).unwrap();

        assert!(!registry.set_labels(running, LabelSet::default()));
        assert!(!registry.set_labels(99, LabelSet::default()));
        assert!(registry.set_labels(done, label_set(vec![label(1, ModelVerdict::Keep)])));

        registry.disk_changed();

        assert_eq!(
            registry.labels(done).unwrap(),
            None,
            "labels of an outdated scan are dropped"
        );
        assert!(
            !registry.set_labels(done, LabelSet::default()),
            "a run that ends after the cleanup is not stored"
        );

        registry.drop_scan(done);
        assert_eq!(registry.labels(done).unwrap_err().code(), ApiErrorCode::NotFound);
    }

    #[test]
    fn a_new_scan_does_not_inherit_the_labels_of_the_old_one() {
        let registry = ScanRegistry::default();
        let (first, _) = registry.begin().unwrap();
        registry.finish(first, tiny_tree()).unwrap();
        registry.set_labels(first, label_set(vec![label(1, ModelVerdict::Review)]));
        let (second, _) = registry.begin().unwrap();
        registry.finish(second, tiny_tree()).unwrap();

        assert_eq!(registry.labels(second).unwrap(), None);
    }

    #[test]
    fn suggestions_are_made_again_when_the_labels_or_the_advisor_option_change() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();
        registry.finish(id, tiny_tree()).unwrap();
        let runs = std::sync::atomic::AtomicUsize::new(0);
        let compute = |_: &Tree, _: &[ModelLabel]| {
            runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            empty_suggestions()
        };
        let runs_so_far = || runs.load(std::sync::atomic::Ordering::SeqCst);

        registry.suggestions(id, true, 0, compute).unwrap();
        registry.suggestions(id, true, 0, compute).unwrap();
        assert_eq!(runs_so_far(), 1);

        registry.set_labels(id, label_set(vec![label(1, ModelVerdict::Review)]));
        registry.suggestions(id, true, 0, compute).unwrap();
        registry.suggestions(id, true, 0, compute).unwrap();
        assert_eq!(runs_so_far(), 2, "new labels");

        registry.suggestions(id, false, 0, compute).unwrap();
        registry.suggestions(id, false, 0, compute).unwrap();
        assert_eq!(runs_so_far(), 3, "the advisor was switched off");

        registry.suggestions(id, true, 0, compute).unwrap();
        assert_eq!(runs_so_far(), 4, "and on again");

        registry.disk_changed();
        registry.suggestions(id, true, 0, compute).unwrap();
        assert_eq!(runs_so_far(), 5, "dropping the labels is a change too");
    }

    #[test]
    fn suggestions_are_made_again_when_the_user_rules_change() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();
        registry.finish(id, tiny_tree()).unwrap();
        let runs = std::sync::atomic::AtomicUsize::new(0);
        let compute = |_: &Tree, _: &[ModelLabel]| {
            runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            empty_suggestions()
        };
        let runs_so_far = || runs.load(std::sync::atomic::Ordering::SeqCst);

        registry.suggestions(id, true, 0, compute).unwrap();
        registry.suggestions(id, true, 0, compute).unwrap();
        assert_eq!(runs_so_far(), 1);

        registry.suggestions(id, true, 1, compute).unwrap();
        registry.suggestions(id, true, 1, compute).unwrap();
        assert_eq!(runs_so_far(), 2, "the user's rules changed");

        registry.suggestions(id, true, 0, compute).unwrap();
        assert_eq!(runs_so_far(), 3, "any other generation is a change too");
    }

    #[test]
    fn the_labels_reach_the_suggestions_only_while_the_advisor_is_on() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();
        registry.finish(id, tiny_tree()).unwrap();
        let labels = vec![label(1, ModelVerdict::Review), label(2, ModelVerdict::Keep)];
        registry.set_labels(id, label_set(labels.clone()));

        assert_eq!(suggestions_seeing(&registry, id, true), labels);
        assert_eq!(suggestions_seeing(&registry, id, false), Vec::new());
        assert_eq!(suggestions_seeing(&registry, id, true), labels);
    }

    #[test]
    fn a_scan_is_running_until_it_finishes() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();

        assert!(registry.is_running(id));
        registry.finish(id, tiny_tree()).unwrap();

        assert!(!registry.is_running(id));
        assert!(!registry.is_running(99));
    }

    #[test]
    fn a_change_to_the_disk_outdates_finished_scans_and_scans_that_were_running() {
        let registry = ScanRegistry::default();
        let (done, _) = registry.begin().unwrap();
        registry.finish(done, tiny_tree()).unwrap();
        let (running, _) = registry.begin().unwrap();

        registry.disk_changed();
        assert!(registry.is_outdated(done));
        registry.finish(running, tiny_tree()).unwrap();
        assert!(
            registry.is_outdated(running),
            "it may have read the disk before the change"
        );

        let (after, _) = registry.begin().unwrap();
        registry.finish(after, tiny_tree()).unwrap();
        assert!(
            !registry.is_outdated(after),
            "a scan begun after the change is current"
        );
    }

    #[test]
    fn a_picked_folder_is_allowed_with_its_identity_and_a_volume_without() {
        let state = AppState::with_advisor(AdvisorService::unavailable());

        state.allow_roots(["/".to_owned()]);
        state.allow_picked("/home/u/x".to_owned(), Some((1, 2)));

        assert_eq!(state.allowed_root("/"), Some(None));
        assert_eq!(state.allowed_root("/home/u/x"), Some(Some((1, 2))));
        assert_eq!(state.allowed_root("/home/u/x/"), None, "exact text only");
    }

    #[test]
    fn the_number_of_running_scans_is_bounded() {
        let registry = ScanRegistry::default();
        let started: Vec<_> = (0..MAX_RUNNING).map(|_| registry.begin().unwrap().0).collect();

        let rejected = registry.begin().unwrap_err();
        registry.abandon(started[0]);

        assert_eq!(rejected.code(), ApiErrorCode::Invalid);
        assert!(registry.begin().is_ok());
    }
}
