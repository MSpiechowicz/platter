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
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use platter_core::dto::{ScanId, SuggestionsDto};
use platter_core::{CancelToken, Tree};
use platter_platform::Platform;

use crate::error::{CommandError, CommandResult};

/// Scans that may run at the same time.
pub const MAX_RUNNING: usize = 4;

pub struct AppState {
    pub platform: Arc<Platform>,
    pub scans: Arc<ScanRegistry>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            platform: Arc::new(Platform::current()),
            scans: Arc::new(ScanRegistry::default()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

enum Slot {
    Running(CancelToken),
    Finished(Finished),
}

/// A finished scan: its tree and the suggestions computed from it (once, on first request).
#[derive(Clone)]
struct Finished {
    tree: Arc<Tree>,
    suggestions: Arc<OnceLock<SuggestionsDto>>,
}

#[derive(Default)]
struct Inner {
    last_id: ScanId,
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
            .filter(|slot| matches!(slot, Slot::Running(_)))
            .count();
        if running >= MAX_RUNNING {
            return Err(CommandError::invalid(format!(
                "{MAX_RUNNING} scans are already running; cancel one first"
            )));
        }

        inner.last_id += 1;
        let id = inner.last_id;
        let cancel = CancelToken::new();
        inner.slots.insert(id, Slot::Running(cancel.clone()));

        Ok((id, cancel))
    }

    /// Stores the tree of a running scan and evicts every other finished tree.
    ///
    /// Returns `None` (and discards the tree) when the scan was dropped while it was running.
    pub fn finish(&self, id: ScanId, tree: Tree) -> Option<Arc<Tree>> {
        let mut inner = self.lock();

        if !matches!(inner.slots.get(&id), Some(Slot::Running(_))) {
            return None;
        }

        inner.slots.retain(|_, slot| !matches!(slot, Slot::Finished(_)));

        let tree = Arc::new(tree);
        inner.slots.insert(
            id,
            Slot::Finished(Finished {
                tree: Arc::clone(&tree),
                suggestions: Arc::default(),
            }),
        );

        Some(tree)
    }

    /// Forgets a running scan that ended without a tree (cancelled or failed).
    pub fn abandon(&self, id: ScanId) {
        let mut inner = self.lock();

        if matches!(inner.slots.get(&id), Some(Slot::Running(_))) {
            inner.slots.remove(&id);
        }
    }

    /// Raises the cancel flag of a running scan. A finished or unknown scan is left alone.
    pub fn cancel(&self, id: ScanId) {
        if let Some(Slot::Running(cancel)) = self.lock().slots.get(&id) {
            cancel.cancel();
        }
    }

    /// Frees a scan: cancels it when running, drops the tree when finished. Unknown ids are fine.
    pub fn drop_scan(&self, id: ScanId) {
        if let Some(Slot::Running(cancel)) = self.lock().slots.remove(&id) {
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

    fn finished(&self, id: ScanId) -> CommandResult<Finished> {
        match self.lock().slots.get(&id) {
            Some(Slot::Finished(finished)) => Ok(finished.clone()),
            Some(Slot::Running(_)) => Err(CommandError::not_found(format!("scan {id} has not finished"))),
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
    fn the_number_of_running_scans_is_bounded() {
        let registry = ScanRegistry::default();
        let started: Vec<_> = (0..MAX_RUNNING).map(|_| registry.begin().unwrap().0).collect();

        let rejected = registry.begin().unwrap_err();
        registry.abandon(started[0]);

        assert_eq!(rejected.code(), ApiErrorCode::Invalid);
        assert!(registry.begin().is_ok());
    }
}
