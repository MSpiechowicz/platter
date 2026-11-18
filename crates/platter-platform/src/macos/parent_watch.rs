//! The helper ends when the app does (package M3). macOS has no parent death signal, so besides the socket's
//! end of file (which `platter_ai::runtime::serve` acts on) a thread polls the parent's process id about every
//! 500 ms, and the helper exits when it is no longer its parent. The rule is
//! [`crate::runtime::parent_is_gone`].

use std::io;
use std::thread;
use std::time::Duration;

use crate::runtime::parent_is_gone;

/// How often the parent's process id is looked at.
const POLL_EVERY: Duration = Duration::from_millis(500);

/// Starts the watch in the helper. `on_orphaned` runs once when the parent is gone (it is meant to end the
/// process). Fails, and starts nothing, when the parent is already gone at the start (it is `init`).
pub fn watch_parent(on_orphaned: impl FnOnce() + Send + 'static) -> io::Result<()> {
    watch(current_parent, POLL_EVERY, on_orphaned)
}

/// The id of this process's parent (0 when the system says none).
fn current_parent() -> u32 {
    rustix::process::Pid::as_raw(rustix::process::getppid()).unsigned_abs()
}

/// [`watch_parent`] with its source of the parent's id and its interval given.
fn watch(
    parent: impl Fn() -> u32 + Send + 'static,
    every: Duration,
    on_orphaned: impl FnOnce() + Send + 'static,
) -> io::Result<()> {
    let at_start = parent();
    if parent_is_gone(at_start, at_start) {
        return Err(io::Error::other(
            "the app that started the helper is already gone",
        ));
    }

    thread::Builder::new()
        .name("parent-watch".to_owned())
        .spawn(move || {
            while !parent_is_gone(at_start, parent()) {
                thread::sleep(every);
            }
            on_orphaned();
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, mpsc};

    #[test]
    fn the_watch_fires_once_when_the_parents_id_changes() {
        let parent = Arc::new(AtomicU32::new(4242));
        let (told, orphaned) = mpsc::channel();

        let source = Arc::clone(&parent);
        watch(
            move || source.load(Ordering::SeqCst),
            Duration::from_millis(5),
            move || told.send(()).unwrap(),
        )
        .unwrap();
        assert!(
            orphaned.recv_timeout(Duration::from_millis(100)).is_err(),
            "not while the parent is there"
        );

        parent.store(1, Ordering::SeqCst);

        assert!(orphaned.recv_timeout(Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn a_parent_that_is_init_at_the_start_is_refused() {
        let result = watch(|| 1, Duration::from_millis(5), || {});

        assert!(result.is_err());
    }
}
