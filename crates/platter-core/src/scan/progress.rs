use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::model::Progress;

/// Callback receiving throttled progress snapshots; called from worker threads.
pub type ProgressFn<'a> = &'a (dyn Fn(&Progress) + Sync);

/// Counters gathered while reading one directory, flushed in one go.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Delta {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub errors: u64,
}

/// Linked list of names from the root to the directory being read. Lives on the stack of the walk and is
/// only turned into a path when a progress snapshot is actually emitted.
pub(crate) struct PathChain<'a> {
    pub parent: Option<&'a PathChain<'a>>,
    pub name: &'a [u8],
    /// (dev, ino) of this directory; (0, 0) when the source does not know it.
    pub identity: (u64, u64),
}

impl PathChain<'_> {
    /// Whether `identity` is that of this directory or of one of its ancestors (a directory loop).
    pub fn is_inside(&self, identity: (u64, u64)) -> bool {
        if identity == (0, 0) {
            return false;
        }

        let mut link = Some(self);
        while let Some(current) = link {
            if current.identity == identity {
                return true;
            }
            link = current.parent;
        }
        false
    }

    fn render(&self, root: &str) -> String {
        let mut names = Vec::new();
        let mut link = Some(self);

        while let Some(current) = link {
            if current.parent.is_some() {
                names.push(current.name);
            }
            link = current.parent;
        }

        let mut path = PathBuf::from(root);
        for name in names.into_iter().rev() {
            path.push(&*String::from_utf8_lossy(name));
        }
        path.to_string_lossy().into_owned()
    }
}

/// Whole milliseconds of `duration`, saturating at `u64::MAX` (a bare `as u64` would wrap a huge
/// duration around to a small one).
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Whether a snapshot is due: at least `interval_ms` since the last one, with no overflow whatever the
/// interval.
fn is_due(now_ms: u64, last_ms: u64, interval_ms: u64) -> bool {
    now_ms >= last_ms.saturating_add(interval_ms)
}

/// Lock-free counters plus a throttle: at most one snapshot per interval, claimed with a CAS so only one
/// worker pays for building the path string.
pub(crate) struct Reporter<'a> {
    callback: ProgressFn<'a>,
    root: String,
    started: Instant,
    interval_ms: u64,
    last_emit_ms: AtomicU64,
    files: AtomicU64,
    dirs: AtomicU64,
    bytes: AtomicU64,
    errors: AtomicU64,
}

impl<'a> Reporter<'a> {
    pub fn new(callback: ProgressFn<'a>, root: String, interval: Duration) -> Self {
        Self {
            callback,
            root,
            started: Instant::now(),
            interval_ms: millis(interval).max(1),
            last_emit_ms: AtomicU64::new(0),
            files: AtomicU64::new(0),
            dirs: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }
    }

    pub fn add(&self, delta: Delta) {
        self.files.fetch_add(delta.files, Ordering::Relaxed);
        self.dirs.fetch_add(delta.dirs, Ordering::Relaxed);
        let _ = self
            .bytes
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |bytes| {
                Some(bytes.saturating_add(delta.bytes))
            });
        self.errors.fetch_add(delta.errors, Ordering::Relaxed);
    }

    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    /// Directories counted so far (a capacity hint for the arena).
    pub fn dirs(&self) -> usize {
        usize::try_from(self.dirs.load(Ordering::Relaxed)).unwrap_or(usize::MAX)
    }

    /// Entries counted so far (a capacity hint for the arena).
    pub fn entries(&self) -> usize {
        let total = self
            .files
            .load(Ordering::Relaxed)
            .saturating_add(self.dirs.load(Ordering::Relaxed));
        usize::try_from(total).unwrap_or(usize::MAX)
    }

    /// Emits a snapshot if the interval has elapsed and no other worker claimed it first.
    pub fn tick(&self, path: &PathChain<'_>) {
        let now_ms = millis(self.started.elapsed());
        let last = self.last_emit_ms.load(Ordering::Relaxed);

        if !is_due(now_ms, last, self.interval_ms) {
            return;
        }
        if self
            .last_emit_ms
            .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        (self.callback)(&Progress {
            files: self.files.load(Ordering::Relaxed),
            dirs: self.dirs.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            current_path: path.render(&self.root),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_huge_interval_saturates_instead_of_wrapping_to_a_small_one() {
        // 2^64 + 384 milliseconds: `as u64` would turn this into 384.
        let huge = Duration::from_secs(18_446_744_073_709_552);

        assert_eq!(millis(huge), u64::MAX);
        assert_eq!(millis(Duration::MAX), u64::MAX);
        assert_eq!(millis(Duration::from_millis(250)), 250);
    }

    #[test]
    fn the_throttle_does_not_overflow_when_the_last_emit_and_the_interval_are_huge() {
        assert!(is_due(100, 0, 100));
        assert!(!is_due(99, 0, 100));
        assert!(
            !is_due(u64::MAX - 1, u64::MAX - 5, u64::MAX),
            "saturates, never due, no panic"
        );
    }
}
