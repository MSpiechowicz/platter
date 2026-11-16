//! Backend state: the platform adapters and the registry of scans addressed by `ScanId`.
//!
//! Retention policy (bounds memory):
//! - suggestions are computed once per finished scan and kept next to its tree, so they are dropped with it;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use platter_core::dto::{ScanId, SuggestionsDto};
use platter_core::{CancelToken, Tree};
use platter_platform::Platform;

use crate::commands::cleanup::{CleanupRegistry, Confirm, RefuseAll};
use crate::error::{CommandError, CommandResult};

/// Scans that may run at the same time.
pub const MAX_RUNNING: usize = 4;

pub struct AppState {
    pub platform: Arc<Platform>,
    pub scans: Arc<ScanRegistry>,
    pub cleanups: Arc<CleanupRegistry>,
    /// The scan roots the backend handed to the UI itself: volume mount points from `list_volumes`
    /// and folders from its own folder dialog. `start_scan` accepts nothing else.
    pub roots: Mutex<HashMap<String, Option<(u64, u64)>>>,
    /// Asks the user before a cleanup that cannot be undone. Set by the app at startup; until then
    /// (and in tests that set none) nothing is approved.
    confirm: OnceLock<Arc<dyn Confirm>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            platform: Arc::new(Platform::current()),
            scans: Arc::new(ScanRegistry::default()),
            cleanups: Arc::new(CleanupRegistry::default()),
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

/// A finished scan: its tree and the suggestions computed from it (once, on first request).
#[derive(Clone)]
struct Finished {
    tree: Arc<Tree>,
    suggestions: Arc<OnceLock<SuggestionsDto>>,
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
                "{MAX_RUNNING} scans are already running; cancel one first"
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

    /// The suggestions of a finished scan. `compute` runs the first time only (concurrent first calls
    /// wait for it instead of repeating it); later calls return the stored result. It may take long, so
    /// call this off the async runtime. The result lives and dies with the scan.
    pub fn suggestions(
        &self,
        id: ScanId,
        compute: impl FnOnce(&Tree) -> SuggestionsDto,
    ) -> CommandResult<SuggestionsDto> {
        let finished = self.finished(id)?;

        // The registry lock is not held while computing: other commands keep working.
        Ok(finished
            .suggestions
            .get_or_init(|| compute(&finished.tree))
            .clone())
    }

    /// A cleanup or an undo changed the disk: every finished scan is outdated, and so is every scan
    /// still running, when it finishes (it may have read the disk before the change).
    pub fn disk_changed(&self) {
        let mut inner = self.lock();
        inner.epoch += 1;
        for slot in inner.slots.values() {
            if let Slot::Finished(finished) = slot {
                finished.outdated.store(true, Ordering::SeqCst);
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

fn unknown_scan(id: ScanId) -> CommandError {
    CommandError::not_found(format!("scan {id} does not exist"))
}

#[cfg(test)]
mod tests {
    use platter_core::dto::ApiErrorCode;
    use platter_core::testing::{FakeFs, scan_fake};

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
        let compute = |_: &Tree| {
            runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            empty_suggestions()
        };

        let first = registry.suggestions(id, compute).unwrap();
        let second = registry.suggestions(id, compute).unwrap();

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
                .suggestions(id, |_| unreachable!("no tree to judge"))
                .unwrap_err();

            assert_eq!(error.code(), ApiErrorCode::NotFound, "scan {id}");
        }
    }

    #[test]
    fn a_new_scan_does_not_inherit_the_suggestions_of_the_old_one() {
        let registry = ScanRegistry::default();
        let (first, _) = registry.begin().unwrap();
        registry.finish(first, tiny_tree()).unwrap();
        registry.suggestions(first, |_| empty_suggestions()).unwrap();
        let (second, _) = registry.begin().unwrap();
        registry.finish(second, tiny_tree()).unwrap();
        let mut computed = false;

        registry
            .suggestions(second, |_| {
                computed = true;
                empty_suggestions()
            })
            .unwrap();

        assert!(computed);
        assert!(registry.suggestions(first, |_| empty_suggestions()).is_err());
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
        let state = AppState::new();

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
