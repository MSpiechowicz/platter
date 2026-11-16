//! The process-wide budget of directory descriptors kept open as parents of pending children.
//!
//! A scan keeps a directory's descriptor open until the last pending child has been opened under it
//! (one `openat` per child). That is cheap but grows with depth times workers, and several scans can run
//! at once in one process (the desktop app allows four). The budget is therefore one counter for the
//! whole process, not one per scan: otherwise four scans could each take half the soft `RLIMIT_NOFILE`
//! and leave the process without a descriptor for anything else. A directory that cannot claim a slot
//! keeps no descriptor, and its children are reopened from the scan root instead (slower, but it holds
//! at most two descriptors whatever the depth).

use std::sync::atomic::{AtomicUsize, Ordering};

use rustix::process::{Resource, getrlimit};

/// Soft limit assumed when `RLIMIT_NOFILE` is unlimited.
const UNLIMITED_FD_BUDGET: usize = 1 << 20;

/// Slots in use right now, over every scan of the process.
static HELD: AtomicUsize = AtomicUsize::new(0);

/// Default size of the budget: half the soft `RLIMIT_NOFILE`, the rest is left to the workers that are
/// reading and to the rest of the process.
pub(super) fn default_limit() -> usize {
    match getrlimit(Resource::Nofile).current {
        Some(soft) => usize::try_from(soft / 2).unwrap_or(usize::MAX),
        None => UNLIMITED_FD_BUDGET,
    }
}

/// Directory descriptors that scans of this process are holding open right now as parents of pending
/// children. Never more than the budget (see [`default_limit`]) across all scans together.
pub fn descriptors_held() -> usize {
    HELD.load(Ordering::Relaxed)
}

/// One claimed slot of the budget, given back when dropped.
#[derive(Debug)]
pub(super) struct Slot(());

impl Slot {
    /// Claims a slot unless `limit` slots are in use already.
    pub fn claim(limit: usize) -> Option<Slot> {
        HELD.try_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
            (held < limit).then_some(held + 1)
        })
        .ok()
        .map(|_| Slot(()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        HELD.fetch_sub(1, Ordering::Relaxed);
    }
}
