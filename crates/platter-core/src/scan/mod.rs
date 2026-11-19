//! The scanner: a parallel walk over a [`DirSource`] that produces a finished [`Tree`].
//!
//! # Concurrency design
//!
//! One rayon task reads one directory and then fans its subdirectories out as nested `par_iter`s, so
//! work-stealing balances deep and wide trees alike. A task owns what it builds (a per-directory
//! chunk with a private name buffer); nothing is shared per file. The only shared state are an atomic
//! cancel flag (checked per directory), the entry budget (one lock per batch of a thousand entries, see
//! [`crate::budget`]) and relaxed atomic counters flushed once per directory. After
//! the walk, a single-threaded pass copies the chunks into the arena in a fixed breadth-first,
//! name-sorted order and deduplicates hardlinks there, then one reverse pass computes aggregates. Totals
//! and node ids therefore do not depend on thread scheduling.
//!
//! The scan runs on its own thread pool (so it cannot starve, or be starved by, the caller's rayon
//! pool) with large stacks for deep directory trees.

mod assemble;
mod cancel;
mod chunk;
mod progress;
mod walker;

use std::io;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use cancel::CancelToken;
pub use progress::ProgressFn;
pub(crate) use walker::MAX_DEPTH;

use crate::budget::{BudgetLimits, EntryBudget, LimitHit};
use crate::error::{ScanError, ScanLimit};
use crate::model::{EntryKind, ScanWarning, Tree};
use crate::ports::{DirSource, SourceOptions};
use assemble::{Assembly, assemble, root_name_len};
use progress::{Delta, PathChain, Reporter};
use walker::{MAX_ENTRIES, MAX_NAME_BYTES, Walker};

const WORKER_STACK_BYTES: usize = 32 * 1024 * 1024;

/// Knobs for [`scan`].
#[derive(Debug, Clone)]
pub struct ScanConfig {
    /// Stay on the filesystem of the scan root. The source decides what that means (see
    /// [`SourceOptions::one_file_system`]); a mount it does not enter becomes a `mount` node of size 0.
    /// Without it every mount is entered, except those the source always leaves alone (pseudo and
    /// network filesystems on Linux).
    pub one_file_system: bool,
    /// The `(dev, ino)` the root directory was seen with when the caller resolved its path. If the
    /// directory opened by `open_root` has another identity (the path was swapped for another folder in
    /// between) the scan fails with [`ScanError::Root`] instead of scanning the wrong place. `None`
    /// skips the check.
    pub root_identity: Option<(u64, u64)>,
    /// Worker threads; `None` picks a default for I/O-bound walking.
    pub threads: Option<usize>,
    /// Scan time in unix seconds (reference for age buckets); `None` reads the system clock.
    pub now: Option<i64>,
    /// Minimum time between progress callbacks.
    pub progress_interval: Duration,
    /// Most entries (files and directories) to collect before the scan fails with
    /// [`ScanError::TooLarge`], so a hostile or endless listing cannot exhaust memory. `None` leaves only
    /// the hard limit of the arena (a little under 2^32 entries). Room is reserved while a directory is
    /// listed, so the cap holds however many directories are read at once (see [`crate::budget`]).
    pub max_entries: Option<u64>,
    /// Most bytes of file names to collect before the scan fails with [`ScanError::TooLarge`]. `None`
    /// leaves only the hard limit of the arena (4 GiB of names).
    pub max_name_bytes: Option<u64>,
    /// Most memory the scan may use, by the estimate of [`crate::budget`]: a fixed cost per entry plus
    /// the length of its name. `None` is no limit beyond the other two. An app picks a figure that fits
    /// the memory it finds (the platform crate suggests half of what is free).
    pub max_memory_bytes: Option<u64>,
    /// The `(dev, ino)` of folders the user wants left out. A directory with one of these identities is
    /// not entered and becomes a `mount` node of size 0 with [`SkipReason::Excluded`](crate::model::SkipReason::Excluded).
    /// The identity of the scan root is ignored, the root is always scanned.
    pub excluded: Vec<(u64, u64)>,
    /// Some folders the user wants left out could not be resolved: the finished scan carries
    /// [`ScanWarning::ExcludedUnresolved`] so the user learns they were scanned after all.
    pub excluded_unresolved: bool,
}

/// Default for [`ScanConfig::max_entries`]: 30 million entries, several gigabytes at the top end. An app
/// should bound a scan by memory instead ([`ScanConfig::max_memory_bytes`]) and leave this as the hard
/// cap it is.
pub const DEFAULT_MAX_ENTRIES: u64 = 30_000_000;

/// Default for [`ScanConfig::max_memory_bytes`]: 4 GiB.
pub const DEFAULT_MAX_MEMORY_BYTES: u64 = 4 << 30;

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            one_file_system: false,
            root_identity: None,
            threads: None,
            now: None,
            progress_interval: Duration::from_millis(100),
            max_entries: Some(DEFAULT_MAX_ENTRIES),
            max_name_bytes: None,
            max_memory_bytes: Some(DEFAULT_MAX_MEMORY_BYTES),
            excluded: Vec::new(),
            excluded_unresolved: false,
        }
    }
}

/// Scans `root` through `source` and returns the finished tree.
///
/// `on_progress` is called at most once per `config.progress_interval`, from a worker thread. Raising
/// `cancel` stops the walk at the next directory and makes this return [`ScanError::Cancelled`].
pub fn scan<S: DirSource>(
    source: &S,
    root: &Path,
    config: &ScanConfig,
    cancel: &CancelToken,
    on_progress: ProgressFn<'_>,
) -> Result<Tree, ScanError> {
    let started = Instant::now();
    let scanned_at = config.now.unwrap_or_else(unix_now);

    let options = SourceOptions {
        one_file_system: config.one_file_system,
    };
    let root_info = source
        .open_root(root, options)
        .map_err(|source| ScanError::Root {
            path: root.to_path_buf(),
            source,
        })?;
    if root_info.meta.kind != EntryKind::Dir {
        return Err(ScanError::NotADirectory(root.to_path_buf()));
    }
    if let Some(expected) = config.root_identity
        && expected != (root_info.meta.dev, root_info.meta.ino)
    {
        return Err(ScanError::Root {
            path: root.to_path_buf(),
            source: io::Error::other("the folder was replaced while the scan was starting"),
        });
    }

    let threads = config.threads.unwrap_or_else(default_threads).max(1);
    let pool = build_pool(threads)?;
    let root_text = root.to_string_lossy().into_owned();
    let reporter = Reporter::new(on_progress, root_text.clone(), config.progress_interval);

    // A warning is a problem the source ran into while opening the root; each counts as an error too.
    let mut warnings = root_info.warnings;
    if config.excluded_unresolved {
        warnings.push(ScanWarning::ExcludedUnresolved);
    }
    reporter.add(Delta {
        errors: warnings.len() as u64,
        ..Delta::default()
    });

    let max_entries = config.max_entries.map_or(MAX_ENTRIES, |max| max.min(MAX_ENTRIES));
    let name_limit = MAX_NAME_BYTES.saturating_sub(root_name_len(root));
    let max_name_bytes = config
        .max_name_bytes
        .map_or(name_limit, |max| max.min(name_limit));
    let max_memory_bytes = config.max_memory_bytes.unwrap_or(u64::MAX);
    let budget = EntryBudget::new(
        BudgetLimits {
            max_entries,
            max_name_bytes,
            max_memory_bytes,
        },
        threads,
    );
    let root_identity = (root_info.meta.dev, root_info.meta.ino);
    let excluded: Vec<(u64, u64)> = config
        .excluded
        .iter()
        .copied()
        .filter(|&identity| identity != root_identity)
        .collect();
    let walker = Walker {
        source,
        cancel,
        reporter: &reporter,
        budget: &budget,
        excluded: &excluded,
    };

    let root_chunk = pool.install(|| {
        let chain = PathChain {
            parent: None,
            name: root_text.as_bytes(),
            identity: root_identity,
        };
        walker.walk(root_info.handle, 0, &chain)
    });
    if cancel.is_cancelled() {
        return Err(ScanError::Cancelled);
    }
    if let Some(hit) = budget.limit_hit() {
        let limit = match hit {
            LimitHit::Entries => ScanLimit::Entries(max_entries),
            LimitHit::NameBytes => ScanLimit::Names(max_name_bytes),
            LimitHit::Memory => ScanLimit::Memory(max_memory_bytes),
        };
        return Err(ScanError::TooLarge { limit });
    }

    let (_, name_bytes) = budget.used();
    assemble(
        root_chunk,
        Assembly {
            root_path: root,
            root_meta: root_info.meta,
            root_mount_id: root_info.mount_id,
            errors: reporter.errors(),
            scanned_at,
            elapsed: started.elapsed(),
            expected_entries: reporter.entries(),
            expected_dirs: reporter.dirs(),
            expected_name_bytes: name_bytes,
            entry_limit: max_entries,
            name_limit,
            warnings,
        },
    )
}

fn build_pool(threads: usize) -> Result<rayon::ThreadPool, ScanError> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .stack_size(WORKER_STACK_BYTES)
        .thread_name(|index| format!("platter-scan-{index}"))
        .build()
        .map_err(|err| ScanError::ThreadPool(err.to_string()))
}

fn default_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    (cores * 2).clamp(4, 32)
}

fn unix_now() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs() as i64,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests;
