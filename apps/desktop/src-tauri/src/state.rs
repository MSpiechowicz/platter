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
//! - the scans share one memory budget (see [`ScanRegistry::begin_within`]): a running scan holds the grant
//!   it was given, a finished tree holds its own size, and a new scan gets what is left of the pool. The
//!   grant is given back when the scan finishes, is abandoned or is dropped, and a tree's share when the
//!   tree is evicted or dropped. A tree that a query still holds after that is no longer counted: it
//!   is already missing from the free memory the next pool is made from, and it ends with the query;
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
use platter_core::{CancelToken, DEFAULT_MAX_MEMORY_BYTES, ModelLabel, Tree, Units};
use platter_platform::Platform;
use platter_platform::memory::memory_budget_for;

use crate::commands::advisor::AdvisorService;
use crate::commands::advisor::log::QuestionLog;
use crate::commands::advisor::question::AskedBack;
use crate::commands::cleanup::{CleanupRegistry, Confirm, RefuseAll};
use crate::commands::history::{FixedVolumes, HistoryService};
use crate::error::{CommandError, CommandResult};
use crate::user_rules::UserRulesStore;

/// Scans that may run at the same time.
pub const MAX_RUNNING: usize = 4;

/// Queries (`get_children`, `get_lod`) that run at the same time.
pub const MAX_QUERIES: usize = 32;

/// The least memory a scan may be given while other scans or trees hold memory. With less it is refused
/// instead of started to fail as too large.
pub const MIN_SHARED_GRANT_BYTES: u64 = 128 << 20;

/// The refusal of a scan while the memory it may use is held by others.
pub const MEMORY_IN_USE: &str = "Platter is already using the memory it may use for scans. \
                                 Wait for a scan to finish or cancel one, then try again.";

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
    /// The free questions asked back lately and the choices shown for each, to check a `pick` against.
    pub asked_back: Arc<AskedBack>,
    /// How many `find_entries` calls have started. A walk over the tree stops once a newer call has taken
    /// a higher number, so the walks of keystrokes that were typed over do not pile up.
    pub entry_searches: Arc<AtomicU64>,
    /// How many `find_folders` calls have started, for the same reason as `entry_searches`.
    pub folder_searches: Arc<AtomicU64>,
    /// The walks over a whole tree that the web view can start (searches, asks about a folder) and that run
    /// now: at most [`WalkSlots::DEFAULT_MAX`], so a page that calls in a loop cannot pile them up.
    pub walks: Arc<WalkSlots>,
    /// The same for `get_summary`, which walks a whole subtree: a pool of its own, so a search never keeps the
    /// Overview's summary out.
    pub summaries: Arc<WalkSlots>,
    /// The same for `get_children` and `get_lod`, which sort or walk part of a tree on the blocking pool:
    /// a pool of its own with room for the many calls a view makes at once, so a page that calls in a loop
    /// still cannot fill the blocking pool.
    pub queries: Arc<WalkSlots>,
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
            asked_back: Arc::new(AskedBack::default()),
            entry_searches: Arc::new(AtomicU64::new(0)),
            folder_searches: Arc::new(AtomicU64::new(0)),
            walks: Arc::new(WalkSlots::default()),
            summaries: Arc::new(WalkSlots::default()),
            queries: Arc::new(WalkSlots::new(MAX_QUERIES)),
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
    Running(Running),
    Finished(Finished),
}

/// A running scan.
struct Running {
    cancel: CancelToken,
    /// The disk epoch it began in.
    epoch: u64,
    /// The memory it may use (its `max_memory_bytes`), taken from the shared budget until it ends.
    grant: u64,
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
    /// The sizes in the reasons are written in these units.
    units: Units,
}

/// A finished scan: its tree, the suggestions computed from it, and the model's labels of it.
#[derive(Clone)]
struct Finished {
    tree: Arc<Tree>,
    /// The memory the tree holds ([`Tree::memory_bytes`]), taken from the shared budget while it is kept.
    memory: u64,
    /// The result for the key it was made for. Held while a result is made, so concurrent first calls
    /// wait for it instead of repeating it.
    /// At most one result per size unit (`Units` is the only part of the key two callers may differ in at the
    /// same time: the web view in the settings' units, the advisor in GB), so they do not push each other out.
    suggestions: Arc<Mutex<Vec<(SuggestionsKey, SuggestionsDto)>>>,
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

impl Inner {
    /// The memory the running scans were granted and the memory the finished trees hold.
    fn memory(&self) -> (u64, u64) {
        let (mut running, mut held) = (0u64, 0u64);
        for slot in self.slots.values() {
            match slot {
                // A cancelled scan is stopping: what it holds is already missing from the free memory, and it
                // takes no more, so a scan started right after a cancel is not refused for it.
                Slot::Running(scan) if scan.cancel.is_cancelled() => {}
                Slot::Running(scan) => running = running.saturating_add(scan.grant),
                Slot::Finished(finished) => held = held.saturating_add(finished.memory),
            }
        }

        (running, held)
    }

    /// Allocates the next id and registers a running scan with `grant` under it.
    fn register(&mut self, grant: u64) -> CommandResult<(ScanId, CancelToken, u64)> {
        let running = self
            .slots
            .values()
            .filter(|slot| matches!(slot, Slot::Running(..)))
            .count();
        if running >= MAX_RUNNING {
            return Err(CommandError::invalid(format!(
                "{MAX_RUNNING} scans are already running, so cancel one first"
            )));
        }

        self.last_id += 1;
        let id = self.last_id;
        let cancel = CancelToken::new();
        let epoch = self.epoch;
        self.slots.insert(
            id,
            Slot::Running(Running {
                cancel: cancel.clone(),
                epoch,
                grant,
            }),
        );

        Ok((id, cancel, grant))
    }
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

    /// Allocates the next id and registers a running scan under it, with no memory taken from the budget.
    /// The app itself starts scans with [`Self::begin_within`].
    #[cfg(test)]
    pub fn begin(&self) -> CommandResult<(ScanId, CancelToken)> {
        let (id, cancel, _) = self.lock().register(0)?;

        Ok((id, cancel))
    }

    /// [`Self::begin`] with a share of the memory all scans may use together, which is the grant the scan
    /// must stay within (its `max_memory_bytes`).
    ///
    /// The pool is half of the memory that is free (`available`, `None` where it cannot be read, then the
    /// core default) plus what finished trees hold: that memory is not free, yet it is Platter's to give
    /// to a scan. The grant is the pool less what the finished trees hold and what the running scans were
    /// granted, so a scan takes all that is left and the scans never grant more than the pool together.
    ///
    /// Refused as `invalid` when other scans or trees hold memory and less than
    /// [`MIN_SHARED_GRANT_BYTES`] is left. With nothing held the scan is never refused, however little is
    /// left: it then fails as too large, with a message that says why.
    ///
    /// A running scan holds all that is left of the pool, so in practice one scan runs at a time: a second one
    /// is refused until the first ends or is cancelled (a cancelled scan stops counting at once).
    pub fn begin_within(&self, available: Option<u64>) -> CommandResult<(ScanId, CancelToken, u64)> {
        let mut inner = self.lock();
        let (running, held) = inner.memory();
        let others = running.saturating_add(held);

        let pool = available.map_or(DEFAULT_MAX_MEMORY_BYTES, |available| {
            memory_budget_for(available.saturating_add(held))
        });
        let grant = pool.saturating_sub(others);
        if others > 0 && grant < MIN_SHARED_GRANT_BYTES {
            return Err(CommandError::invalid(MEMORY_IN_USE));
        }

        inner.register(grant)
    }

    /// Stores the tree of a running scan and evicts every other finished tree.
    ///
    /// Returns `None` (and discards the tree) when the scan was dropped while it was running.
    pub fn finish(&self, id: ScanId, tree: Tree) -> Option<Arc<Tree>> {
        let mut inner = self.lock();

        let Some(Slot::Running(scan)) = inner.slots.get(&id) else {
            return None;
        };
        let outdated = scan.epoch < inner.epoch;

        // The grant ends with the running slot and the evicted trees' share with theirs: the tree that
        // is stored holds its own size from now on.
        inner.slots.retain(|_, slot| !matches!(slot, Slot::Finished(_)));

        let memory = tree.memory_bytes() as u64;
        let tree = Arc::new(tree);
        inner.slots.insert(
            id,
            Slot::Finished(Finished {
                tree: Arc::clone(&tree),
                memory,
                suggestions: Arc::default(),
                labels: Arc::default(),
                outdated: Arc::new(AtomicBool::new(outdated)),
            }),
        );

        Some(tree)
    }

    /// Forgets a running scan that ended without a tree (cancelled or failed), giving its grant back.
    pub fn abandon(&self, id: ScanId) {
        let mut inner = self.lock();

        if matches!(inner.slots.get(&id), Some(Slot::Running(..))) {
            inner.slots.remove(&id);
        }
    }

    /// Raises the cancel flag of a running scan. A finished or unknown scan is left alone.
    pub fn cancel(&self, id: ScanId) {
        if let Some(Slot::Running(scan)) = self.lock().slots.get(&id) {
            scan.cancel.cancel();
        }
    }

    /// Frees a scan: cancels it when running, drops the tree when finished. Either gives its memory back
    /// to the budget. Unknown ids are fine.
    pub fn drop_scan(&self, id: ScanId) {
        if let Some(Slot::Running(scan)) = self.lock().slots.remove(&id) {
            scan.cancel.cancel();
        }
    }

    /// The finished tree of a scan.
    pub fn tree(&self, id: ScanId) -> CommandResult<Arc<Tree>> {
        Ok(self.finished(id)?.tree)
    }

    /// [`Self::suggestions_in`] with the sizes in the reasons in binary units (the tests of the cache do not
    /// look at the reasons).
    #[cfg(test)]
    pub fn suggestions(
        &self,
        id: ScanId,
        advisor_enabled: bool,
        user_rules_generation: u64,
        compute: impl FnOnce(&Tree, &[ModelLabel]) -> SuggestionsDto,
    ) -> CommandResult<SuggestionsDto> {
        self.suggestions_in(id, advisor_enabled, user_rules_generation, Units::Binary, compute)
    }

    /// The suggestions of a finished scan. `compute` runs the first time and again whenever the labels, the
    /// advisor option, the user's rules (`user_rules_generation`) or the `units` of the sizes in the
    /// reasons changed since (concurrent calls wait for it instead of repeating it); otherwise
    /// the stored result is returned. It gets the model's labels of the scan, none while the advisor is
    /// switched off, and must write the reasons in `units`. It may take long, so call this off the async
    /// runtime. The result lives and dies with the scan.
    pub fn suggestions_in(
        &self,
        id: ScanId,
        advisor_enabled: bool,
        user_rules_generation: u64,
        units: Units,
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
            units,
        };

        let mut stored = lock(&finished.suggestions);
        if let Some((_, suggestions)) = stored.iter().find(|(made_for, _)| *made_for == key) {
            return Ok(suggestions.clone());
        }

        let labels: &[ModelLabel] = match (&set, advisor_enabled) {
            (Some(set), true) => &set.labels,
            _ => &[],
        };
        let suggestions = compute(&finished.tree, labels);
        // A result made for other labels, options or rules is stale; one in the other unit is kept.
        stored.retain(|(made_for, _)| {
            SuggestionsKey {
                units: key.units,
                ..*made_for
            } == key
        });
        stored.push((key, suggestions.clone()));

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
    fn suggestions_are_made_once_per_unit_and_kept_for_both() {
        let registry = ScanRegistry::default();
        let (id, _) = registry.begin().unwrap();
        registry.finish(id, tiny_tree()).unwrap();
        let runs = std::sync::atomic::AtomicUsize::new(0);
        let compute = |_: &Tree, _: &[ModelLabel]| {
            runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            empty_suggestions()
        };
        let runs_so_far = || runs.load(std::sync::atomic::Ordering::SeqCst);

        registry
            .suggestions_in(id, true, 0, Units::Decimal, compute)
            .unwrap();
        registry
            .suggestions_in(id, true, 0, Units::Decimal, compute)
            .unwrap();
        assert_eq!(runs_so_far(), 1);

        registry
            .suggestions_in(id, true, 0, Units::Binary, compute)
            .unwrap();
        registry
            .suggestions_in(id, true, 0, Units::Binary, compute)
            .unwrap();
        assert_eq!(runs_so_far(), 2, "other units");

        // The web view in GiB and the advisor in GB take turns without making either again.
        for units in [Units::Decimal, Units::Binary, Units::Decimal] {
            registry.suggestions_in(id, true, 0, units, compute).unwrap();
        }
        assert_eq!(runs_so_far(), 2, "both are kept");

        // Anything else that changes makes both stale.
        registry
            .suggestions_in(id, false, 0, Units::Decimal, compute)
            .unwrap();
        registry
            .suggestions_in(id, false, 0, Units::Binary, compute)
            .unwrap();
        assert_eq!(runs_so_far(), 4, "the advisor option changed");
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

    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;

    fn grant_of(registry: &ScanRegistry, available: Option<u64>) -> u64 {
        registry.begin_within(available).unwrap().2
    }

    /// The grant a scan would get now, without keeping it.
    fn probe(registry: &ScanRegistry, available: Option<u64>) -> u64 {
        let (id, _, grant) = registry.begin_within(available).unwrap();
        registry.abandon(id);

        grant
    }

    fn refused(registry: &ScanRegistry, available: Option<u64>) -> bool {
        match registry.begin_within(available) {
            Ok((id, ..)) => {
                registry.abandon(id);
                false
            }
            Err(error) => {
                assert_eq!(error.code(), ApiErrorCode::Invalid);
                assert_eq!(serde_json::to_value(&error).unwrap()["message"], MEMORY_IN_USE);
                true
            }
        }
    }

    #[test]
    fn a_scan_alone_gets_half_of_the_free_memory_or_the_default_when_it_is_unknown() {
        let registry = ScanRegistry::default();

        assert_eq!(grant_of(&registry, Some(8 * GIB)), 4 * GIB);
        assert_eq!(
            grant_of(&ScanRegistry::default(), None),
            DEFAULT_MAX_MEMORY_BYTES,
            "MemAvailable unreadable"
        );
    }

    #[test]
    fn with_no_free_memory_the_first_scan_still_starts_and_fails_as_too_large_instead() {
        let registry = ScanRegistry::default();

        assert_eq!(grant_of(&registry, Some(0)), 0);
        assert_eq!(grant_of(&ScanRegistry::default(), Some(1)), 0);
    }

    #[test]
    fn a_kept_tree_counts_as_free_memory_of_the_pool_and_reduces_the_next_grant() {
        let registry = ScanRegistry::default();
        let (id, ..) = registry.begin_within(Some(8 * GIB)).unwrap();
        let tree = tiny_tree();
        let held = tree.memory_bytes() as u64;
        registry.finish(id, tree).unwrap();

        let grant = grant_of(&registry, Some(8 * GIB));

        assert!(held > 0);
        assert_eq!(grant, memory_budget_for(8 * GIB + held) - held);
        assert!(grant < 4 * GIB, "the tree takes its share of what a scan may use");
        assert!(grant > 4 * GIB - held, "and it is not counted as taken twice");
    }

    #[test]
    fn the_grants_of_running_scans_never_exceed_the_pool_together() {
        let registry = ScanRegistry::default();

        let first = grant_of(&registry, Some(4 * GIB));
        let second = grant_of(&registry, Some(8 * GIB));

        assert_eq!(first, 2 * GIB);
        assert_eq!(
            second,
            2 * GIB,
            "the pool of 4 GiB less the 2 GiB the first holds"
        );
        assert!(refused(&registry, Some(8 * GIB)), "nothing is left for a third");
    }

    #[test]
    fn a_scan_takes_the_whole_pool_so_that_a_second_one_waits_until_it_ends() {
        let registry = ScanRegistry::default();
        let (first, ..) = registry.begin_within(Some(GIB)).unwrap();

        assert!(refused(&registry, Some(GIB)));

        registry.abandon(first);
        assert_eq!(grant_of(&registry, Some(GIB)), 512 * MIB);
    }

    #[test]
    fn a_scan_started_right_after_a_cancel_is_not_refused_for_the_one_that_is_stopping() {
        let registry = ScanRegistry::default();
        let (_, cancel, _) = registry.begin_within(Some(GIB)).unwrap();
        assert!(refused(&registry, Some(GIB)));

        // The user cancels and scans again before the first walk has noticed.
        cancel.cancel();
        assert_eq!(grant_of(&registry, Some(GIB)), 512 * MIB);
    }

    #[test]
    fn a_scan_is_refused_only_when_others_hold_memory_and_less_than_128_mib_is_left() {
        let registry = ScanRegistry::default();
        let (id, ..) = registry.begin_within(Some(4 * GIB)).unwrap();
        registry.finish(id, tiny_tree()).unwrap();

        assert!(refused(&registry, Some(100 * MIB)), "50 MiB of the pool is left");
        assert!(refused(&registry, Some(255 * MIB)), "just under 128 MiB");
        assert!(!refused(&registry, Some(260 * MIB)), "128 MiB is enough");
        assert!(!refused(&registry, None), "the default pool is large");

        let alone = ScanRegistry::default();
        assert!(
            !refused(&alone, Some(100 * MIB)),
            "nothing is held, so nothing is refused"
        );
        assert!(!refused(&alone, Some(0)));
    }

    #[test]
    fn finishing_a_scan_gives_the_grant_back_and_keeps_only_the_tree() {
        let registry = ScanRegistry::default();
        let (id, ..) = registry.begin_within(Some(GIB)).unwrap();
        assert!(refused(&registry, Some(GIB)));
        let tree = tiny_tree();
        let held = tree.memory_bytes() as u64;

        registry.finish(id, tree).unwrap();

        assert_eq!(
            grant_of(&registry, Some(GIB)),
            memory_budget_for(GIB + held) - held
        );
    }

    #[test]
    fn an_evicted_or_dropped_tree_gives_its_memory_back() {
        let registry = ScanRegistry::default();
        let (first, ..) = registry.begin_within(Some(GIB)).unwrap();
        registry.finish(first, tiny_tree()).unwrap();
        let (second, ..) = registry.begin_within(Some(GIB)).unwrap();
        let tree = tiny_tree();
        let held = tree.memory_bytes() as u64;
        let first_tree = registry.tree(first).unwrap();

        registry.finish(second, tree).unwrap();

        assert_eq!(
            probe(&registry, Some(GIB)),
            memory_budget_for(GIB + held) - held,
            "only the newest tree is held, even while a query still has the evicted one"
        );
        drop(first_tree);

        registry.drop_scan(second);

        assert_eq!(probe(&registry, Some(GIB)), 512 * MIB);
    }

    #[test]
    fn abandoning_or_dropping_a_running_scan_gives_its_grant_back() {
        for end in [ScanRegistry::abandon, ScanRegistry::drop_scan] {
            let registry = ScanRegistry::default();
            let (id, ..) = registry.begin_within(Some(GIB)).unwrap();
            assert!(refused(&registry, Some(GIB)));

            end(&registry, id);

            assert_eq!(grant_of(&registry, Some(GIB)), 512 * MIB);
        }
    }

    #[test]
    fn with_unreadable_free_memory_the_default_pool_is_shared_the_same_way() {
        let registry = ScanRegistry::default();
        let (id, ..) = registry.begin_within(None).unwrap();
        registry.finish(id, tiny_tree()).unwrap();
        let held = registry.tree(id).unwrap().memory_bytes() as u64;

        assert_eq!(grant_of(&registry, None), DEFAULT_MAX_MEMORY_BYTES - held);
    }

    #[test]
    fn the_number_of_running_scans_is_bounded_with_the_budget_too() {
        let registry = ScanRegistry::default();
        // Every scan is given a share (a free memory that grows with each), so only the count is left.
        for step in 1..=MAX_RUNNING as u64 {
            registry.begin_within(Some(step * 8 * GIB)).unwrap();
        }

        let error = registry.begin_within(Some(64 * GIB)).unwrap_err();

        assert_eq!(error.code(), ApiErrorCode::Invalid);
        let message = serde_json::to_value(&error).unwrap()["message"].to_string();
        assert!(message.contains("already running"), "{message}");
    }
}

/// The walks over a whole tree that run now (see `AppState::walks`).
#[derive(Debug)]
pub struct WalkSlots {
    max: usize,
    running: std::sync::atomic::AtomicUsize,
}

/// One running walk. Its slot is given back when it is dropped.
#[derive(Debug)]
pub struct WalkSlot(Arc<WalkSlots>);

impl WalkSlots {
    /// The most walks a pool lets run at once unless it is made for more. The web view waits for each
    /// answer, so it never needs more.
    pub const DEFAULT_MAX: usize = 4;

    /// A pool of `max` slots.
    pub fn new(max: usize) -> Self {
        Self {
            max,
            running: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Takes a slot, or refuses (`invalid`) while as many walks run as the pool has slots.
    pub fn take(self: &Arc<Self>) -> crate::error::CommandResult<WalkSlot> {
        use std::sync::atomic::Ordering;

        self.running
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |running| {
                (running < self.max).then_some(running + 1)
            })
            .map_err(|_| crate::error::CommandError::invalid(BUSY_WALKING))?;

        Ok(WalkSlot(Arc::clone(self)))
    }
}

impl Default for WalkSlots {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX)
    }
}

impl Drop for WalkSlot {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The refusal of a walk while every slot of its pool is taken.
pub const BUSY_WALKING: &str = "Platter is busy with other work on this scan. Try again in a moment.";

#[cfg(test)]
mod walk_tests {
    use super::*;

    #[test]
    fn at_most_four_walks_run_and_a_finished_one_gives_its_slot_back() {
        let slots = Arc::new(WalkSlots::default());
        let held: Vec<WalkSlot> = (0..WalkSlots::DEFAULT_MAX)
            .map(|_| slots.take().unwrap())
            .collect();

        let refused = slots.take().expect_err("a fifth walk is refused");
        assert_eq!(serde_json::to_value(&refused).unwrap()["message"], BUSY_WALKING);

        drop(held);
        assert!(slots.take().is_ok());
    }

    #[test]
    fn a_pool_has_as_many_slots_as_it_was_made_with_and_pools_do_not_share() {
        let queries = Arc::new(WalkSlots::new(MAX_QUERIES));
        let walks = Arc::new(WalkSlots::default());
        let held: Vec<WalkSlot> = (0..MAX_QUERIES).map(|_| queries.take().unwrap()).collect();

        assert!(queries.take().is_err(), "the 33rd query is refused");
        assert!(walks.take().is_ok(), "the other pool is not affected");

        drop(held);
        assert!(queries.take().is_ok());
    }
}
