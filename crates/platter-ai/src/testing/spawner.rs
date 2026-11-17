//! A [`Spawner`] whose helpers are threads: the helper's side ([`serve`]) runs on a thread over a real socket
//! pair, with a [`FakeEngine`] inside, so a [`ProcessEngine`](crate::process::ProcessEngine) can be tested
//! with no process and no graphics card. A helper can be told to crash or hang at a given step.

use std::fs::File;
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use platter_platform::RuntimeMode;

use super::{FakeControl, FakeEngine, locked};
use crate::engine::{AiError, Engine, Exit, Generation, GpuDevice, LoadInfo, Placement, ProgressHook};
use crate::process::{RuntimeProcess, Spawned, Spawner, Waited};
use crate::runtime::serve;
use crate::verify::VerifiedModel;

/// The step of a helper's work a fault happens in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Listing the graphics cards, before the model is loaded (also the listing a load makes).
    Probe,
    /// Listing the graphics cards once the model is loaded in this helper (the video memory in use).
    Cards,
    /// Loading the model.
    Load,
    /// Answering (the first call that comes).
    Judge,
    /// Freeing the model when the helper is told to stop.
    Unload,
}

/// What goes wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The helper dies: it writes `stderr` to its standard error, and ends as `exit`.
    Crash { exit: Exit, stderr: Vec<String> },
    /// The helper stops answering and shows no progress, until it is killed.
    Hang,
    /// The helper keeps showing progress and ignores a cancel, until it is killed (or, so that a test that
    /// finds no kill ends, for five seconds).
    Chatter,
}

struct Planned {
    mode: RuntimeMode,
    phase: Phase,
    fault: Fault,
}

#[derive(Default)]
struct Shared {
    spawned: Mutex<Vec<RuntimeMode>>,
    planned: Mutex<Vec<Planned>>,
    refuse: Mutex<Option<String>>,
    running: AtomicUsize,
    next_pid: AtomicU32,
}

/// Starts helpers as threads.
pub struct ThreadSpawner {
    control: FakeControl,
    shared: Arc<Shared>,
}

/// The remote control of a [`ThreadSpawner`].
#[derive(Clone)]
pub struct SpawnerControl {
    shared: Arc<Shared>,
}

impl ThreadSpawner {
    /// A spawner whose helpers each run a [`FakeEngine`] on the log of `control`.
    pub fn new(control: &FakeControl) -> (Self, SpawnerControl) {
        let shared = Arc::new(Shared::default());

        (
            Self {
                control: control.clone(),
                shared: Arc::clone(&shared),
            },
            SpawnerControl { shared },
        )
    }
}

impl SpawnerControl {
    /// The next helper started for `mode` goes wrong with `fault` the first time it reaches `phase`.
    pub fn fault(&self, mode: RuntimeMode, phase: Phase, fault: Fault) {
        locked(&self.shared.planned).push(Planned { mode, phase, fault });
    }

    /// Every start fails with `message` (the system refused to start a process).
    pub fn refuse_spawns(&self, message: &str) {
        *locked(&self.shared.refuse) = Some(message.to_owned());
    }

    /// The mode of every helper started so far, in order.
    pub fn spawned(&self) -> Vec<RuntimeMode> {
        locked(&self.shared.spawned).clone()
    }

    /// Helpers that are still running.
    pub fn running(&self) -> usize {
        self.shared.running.load(Ordering::SeqCst)
    }
}

impl Spawner for ThreadSpawner {
    fn spawn(&mut self, model: Option<File>, mode: RuntimeMode) -> io::Result<Spawned> {
        if let Some(message) = locked(&self.shared.refuse).clone() {
            return Err(io::Error::other(message));
        }
        locked(&self.shared.spawned).push(mode);

        let (app_end, helper_end) = UnixStream::pair()?;
        let (stderr_read, stderr_write) = UnixStream::pair()?;
        let bytes = model
            .as_ref()
            .map_or(Ok(0), |file| file.metadata().map(|meta| meta.len()))?;

        let life = Arc::new(Life::default());
        let engine = Faulty {
            inner: FakeEngine::attached_to(&self.control),
            mode,
            shared: Arc::clone(&self.shared),
            life: Arc::clone(&life),
            socket: helper_end.try_clone()?,
            stderr: stderr_write,
            loaded: false,
        };

        self.shared.running.fetch_add(1, Ordering::SeqCst);
        let (shared, ended) = (Arc::clone(&self.shared), Arc::clone(&life));
        thread::Builder::new()
            .name("platter-fake-helper".to_owned())
            .spawn(move || {
                let code = serve(engine, helper_end, model, bytes);
                shared.running.fetch_sub(1, Ordering::SeqCst);
                ended.end(Exit::Code(code));
            })?;

        Ok(Spawned {
            process: Box::new(ThreadProcess {
                pid: 40_000 + self.shared.next_pid.fetch_add(1, Ordering::SeqCst),
                life,
                socket: app_end.try_clone()?,
            }),
            socket: app_end,
            stderr: Box::new(stderr_read),
        })
    }
}

/// How a helper thread ended, and whether it was killed.
#[derive(Default)]
struct Life {
    exit: Mutex<Option<Exit>>,
    ended: Condvar,
    killed: AtomicBool,
}

impl Life {
    /// Records how it ended, unless it already has (a crash decides its own exit). A helper that was
    /// killed ended by the signal that does it.
    fn end(&self, exit: Exit) {
        let exit = if self.killed.load(Ordering::SeqCst) {
            Exit::Signal(9)
        } else {
            exit
        };

        locked(&self.exit).get_or_insert(exit);
        self.ended.notify_all();
    }
}

struct ThreadProcess {
    pid: u32,
    life: Arc<Life>,
    /// The app's end: shutting it down is what a kill does to a process's socket.
    socket: UnixStream,
}

impl RuntimeProcess for ThreadProcess {
    fn id(&self) -> u32 {
        self.pid
    }

    fn kill(&mut self) {
        self.life.killed.store(true, Ordering::SeqCst);
        let _ = self.socket.shutdown(Shutdown::Both);
    }

    fn wait_for(&mut self, timeout: Duration) -> Waited {
        let deadline = Instant::now() + timeout;
        let mut slot = locked(&self.life.exit);

        loop {
            if let Some(exit) = *slot {
                return Waited::Exited(Some(exit));
            }

            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Waited::Running;
            }
            slot = self
                .life
                .ended
                .wait_timeout(slot, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }
}

/// The helper's engine: a [`FakeEngine`] that goes wrong where it was told to.
struct Faulty {
    inner: FakeEngine,
    mode: RuntimeMode,
    shared: Arc<Shared>,
    life: Arc<Life>,
    /// The helper's end of the connection.
    socket: UnixStream,
    stderr: UnixStream,
    /// The model is loaded in this helper.
    loaded: bool,
}

impl Faulty {
    /// The fault planned for this step of a helper of this mode, once.
    fn planned(&self, phase: Phase) -> Option<Fault> {
        let mut planned = locked(&self.shared.planned);
        let at = planned
            .iter()
            .position(|entry| entry.mode == self.mode && entry.phase == phase)?;

        Some(planned.remove(at).fault)
    }

    /// Goes wrong as `fault` says: a crash ends the connection and the helper; a hang never returns on
    /// its own and ends only when killed. Returns when the helper is to give up what it was doing.
    fn go_wrong(&mut self, fault: Fault) {
        match fault {
            Fault::Crash { exit, stderr } => {
                for line in stderr {
                    let _ = writeln!(self.stderr, "{line}");
                }
                // Decided before the connection ends, so the app that sees the end finds how it ended.
                self.life.end(exit);
                let _ = self.socket.shutdown(Shutdown::Both);
            }
            Fault::Hang => {
                while !self.life.killed.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            Fault::Chatter => {
                let began = Instant::now();
                while !self.life.killed.load(Ordering::SeqCst) && began.elapsed() < Duration::from_secs(5) {
                    self.inner.report_progress();
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

impl Engine for Faulty {
    fn gpus(&mut self) -> Vec<GpuDevice> {
        let phase = if self.loaded { Phase::Cards } else { Phase::Probe };
        if let Some(fault) = self.planned(phase) {
            self.go_wrong(fault);
            return Vec::new();
        }

        self.inner.gpus()
    }

    fn load(
        &mut self,
        model: &VerifiedModel,
        threads: u32,
        placement: Placement,
    ) -> Result<LoadInfo, AiError> {
        if let Some(fault) = self.planned(Phase::Load) {
            self.go_wrong(fault);
            return Err(AiError::Load("the helper went wrong".to_owned()));
        }

        let loaded = self.inner.load(model, threads, placement);
        self.loaded = loaded.is_ok();

        loaded
    }

    fn unload(&mut self) {
        self.loaded = false;
        if let Some(fault) = self.planned(Phase::Unload) {
            self.go_wrong(fault);
        }
        self.inner.unload();
    }

    fn judge(
        &mut self,
        prompt: &str,
        grammar: &str,
        max_tokens: u32,
        cancel: &AtomicBool,
    ) -> Result<Generation, AiError> {
        if let Some(fault) = self.planned(Phase::Judge) {
            self.go_wrong(fault);
            return Err(AiError::Run("the helper went wrong".to_owned()));
        }

        self.inner.judge(prompt, grammar, max_tokens, cancel)
    }

    fn on_progress(&mut self, hook: ProgressHook) {
        self.inner.on_progress(hook);
    }
}
