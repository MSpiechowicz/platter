//! A call that cannot be interrupted, run under a time limit (W2). The shell's move to the Recycle Bin
//! can wait for an answer to a question nobody sees (the bin will not take the item, say), and a system
//! call that waits cannot be stopped, only stopped being waited for. Uses nothing but `std`, so the
//! tests here run on every system (`tests/windows_bin_logic.rs` includes this file).
//!
//! The call runs in its own thread. When the limit passes the caller gets [`Ended::TimedOut`], never a
//! success, and the thread is left to end on its own: it is counted as *abandoned* until it does, and a
//! new call is refused meanwhile, so a shell that is stuck is asked once and not again and again. No
//! lock is held while the call runs, so nothing here can wait on the stuck call.

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

/// How a call that was started ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The call came back in time: `Ok`, or the error it reported.
    Returned(Result<(), String>),
    /// The limit passed first. The call may still be running, and may still do its work later.
    TimedOut,
}

/// Where a call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Abandoned,
    Finished,
}

/// The calls under a limit that were given up on and have not come back.
#[derive(Debug, Clone, Default)]
pub struct Limiter {
    abandoned: Arc<AtomicUsize>,
}

impl Limiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many calls were given up on and are still running.
    pub fn abandoned(&self) -> usize {
        self.abandoned.load(Ordering::SeqCst)
    }

    /// Runs `call` in a thread and waits `limit` for it. An error before it starts (an earlier call that
    /// is still stuck, a thread that cannot be made) means `call` was not run.
    pub fn run(
        &self,
        limit: Duration,
        call: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) -> io::Result<Ended> {
        if self.abandoned() > 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "an earlier request to the Recycle Bin has not come back (it may be waiting on a Windows prompt), so none is made until it does",
            ));
        }

        let phase = Arc::new(Mutex::new(Phase::Running));
        let (send, receive) = mpsc::sync_channel(1);
        let worker_phase = Arc::clone(&phase);
        let abandoned = Arc::clone(&self.abandoned);

        std::thread::Builder::new()
            .name("platter-shell".to_owned())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(call))
                    .unwrap_or_else(|_| Err("the call stopped unexpectedly".to_owned()));

                // The result is sent while the phase is held, so a caller that sees `Finished` finds it.
                let mut phase = lock(&worker_phase);
                if *phase == Phase::Abandoned {
                    abandoned.fetch_sub(1, Ordering::SeqCst);
                }
                *phase = Phase::Finished;
                let _ = send.send(result);
            })?;

        match receive.recv_timeout(limit) {
            Ok(result) => Ok(Ended::Returned(result)),
            Err(_) => {
                let mut phase = lock(&phase);
                if *phase == Phase::Finished {
                    return match receive.try_recv() {
                        Ok(result) => Ok(Ended::Returned(result)),
                        Err(_) => Ok(Ended::TimedOut),
                    };
                }

                *phase = Phase::Abandoned;
                self.abandoned.fetch_add(1, Ordering::SeqCst);

                Ok(Ended::TimedOut)
            }
        }
    }
}

fn lock(phase: &Mutex<Phase>) -> MutexGuard<'_, Phase> {
    phase.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const SHORT: Duration = Duration::from_millis(100);
    const LONG: Duration = Duration::from_secs(30);

    #[test]
    fn a_call_that_returns_in_time_gives_its_result() {
        let limiter = Limiter::new();

        assert_eq!(limiter.run(LONG, || Ok(())).unwrap(), Ended::Returned(Ok(())));
        assert_eq!(
            limiter.run(LONG, || Err("no".to_owned())).unwrap(),
            Ended::Returned(Err("no".to_owned()))
        );
        assert_eq!(limiter.abandoned(), 0);
    }

    #[test]
    fn a_call_that_panics_is_an_error_and_not_a_hang() {
        let limiter = Limiter::new();

        let ended = limiter.run(LONG, || panic!("boom")).unwrap();

        assert!(matches!(ended, Ended::Returned(Err(_))), "{ended:?}");
    }

    #[test]
    fn a_call_that_waits_is_timed_out_and_blocks_new_calls_until_it_comes_back() {
        let limiter = Limiter::new();
        let (release, held) = mpsc::channel::<()>();

        let started = Instant::now();
        let ended = limiter
            .run(SHORT, move || {
                let _ = held.recv();
                Ok(())
            })
            .unwrap();

        assert_eq!(ended, Ended::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(limiter.abandoned(), 1);
        let refused = limiter.run(LONG, || Ok(())).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::WouldBlock);

        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while limiter.abandoned() > 0 {
            assert!(
                Instant::now() < deadline,
                "the abandoned call never counted as ended"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(limiter.run(LONG, || Ok(())).unwrap(), Ended::Returned(Ok(())));
    }
}
