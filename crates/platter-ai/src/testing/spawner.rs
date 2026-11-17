//! A [`Spawner`] whose helpers are threads: the helper's side ([`serve`]) runs on a thread over a real socket
//! pair (Unix) or a pair of pipes (the way the real helper on Windows is reached, and available everywhere
//! with [`ThreadSpawner::over_pipes`]), with a [`FakeEngine`] inside, so a
//! [`ProcessEngine`](crate::process::ProcessEngine) can be tested with no process and no graphics card. A helper
//! can be told to crash or hang at a given step.

use std::fs::File;
use std::io::{self, Write};
#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use platter_platform::RuntimeMode;

use super::{FakeControl, FakeEngine, locked};
use crate::engine::{AiError, Engine, Exit, Generation, GpuDevice, LoadInfo, Placement, ProgressHook};
#[cfg(unix)]
use crate::process::SocketWire;
use crate::process::{PipeWire, RuntimeProcess, Spawned, Spawner, Waited, Wire};
use crate::runtime::{Connection, serve};
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

/// What a helper thread talks over.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Transport {
    #[cfg(unix)]
    Socket,
    Pipes,
}

/// Starts helpers as threads.
pub struct ThreadSpawner {
    control: FakeControl,
    shared: Arc<Shared>,
    transport: Transport,
    /// The size the helper's model has when it is not handed over as a file (a `load` that names a path).
    model_bytes: u64,
}

/// The remote control of a [`ThreadSpawner`].
#[derive(Clone)]
pub struct SpawnerControl {
    shared: Arc<Shared>,
}

impl ThreadSpawner {
    /// A spawner whose helpers each run a [`FakeEngine`] on the log of `control`, over a socket pair (pipes on
    /// Windows, where there is none).
    pub fn new(control: &FakeControl) -> (Self, SpawnerControl) {
        #[cfg(unix)]
        let transport = Transport::Socket;
        #[cfg(not(unix))]
        let transport = Transport::Pipes;

        Self::with_transport(control, transport)
    }

    /// [`ThreadSpawner::new`] with pipes on any system: the transport the helper on Windows uses.
    pub fn over_pipes(control: &FakeControl) -> (Self, SpawnerControl) {
        Self::with_transport(control, Transport::Pipes)
    }

    /// The model of a helper whose `load` names the path of the file has `bytes` bytes.
    #[must_use]
    pub fn expecting(mut self, bytes: u64) -> Self {
        self.model_bytes = bytes;
        self
    }

    fn with_transport(control: &FakeControl, transport: Transport) -> (Self, SpawnerControl) {
        let shared = Arc::new(Shared::default());

        (
            Self {
                control: control.clone(),
                shared: Arc::clone(&shared),
                transport,
                model_bytes: 0,
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

        let (stderr_read, stderr_write) = io::pipe()?;
        let bytes = model.as_ref().map_or(Ok(self.model_bytes), |file| {
            file.metadata().map(|meta| meta.len())
        })?;

        let life = Arc::new(Life::default());
        let broken = Arc::new(AtomicBool::new(false));

        // The helper's end, the app's, and how a crash ends the connection from the helper's side.
        #[cfg(unix)]
        let mut app_socket = None;
        let (wire, helper, breaker): (Box<dyn Wire>, Box<dyn Run>, Breaker) = match self.transport {
            #[cfg(unix)]
            Transport::Socket => {
                let (app_end, helper_end) = UnixStream::pair()?;
                let breaker = Breaker::Socket(helper_end.try_clone()?);
                app_socket = Some(app_end.try_clone()?);

                (
                    Box::new(SocketWire::new(app_end)),
                    Box::new(Ends(helper_end)),
                    breaker,
                )
            }
            Transport::Pipes => {
                // Requests: the app writes, the helper reads. Replies: the other way.
                let (requests_read, requests_write) = io::pipe()?;
                let (replies_read, replies_write) = io::pipe()?;
                let helper_end = (
                    requests_read,
                    BreakableWriter {
                        inner: replies_write,
                        broken: Arc::clone(&broken),
                    },
                );

                (
                    Box::new(PipeWire::new(replies_read, requests_write, || {})),
                    Box::new(Ends(helper_end)),
                    Breaker::Flag(Arc::clone(&broken)),
                )
            }
        };

        let engine = Faulty {
            inner: FakeEngine::attached_to(&self.control),
            mode,
            shared: Arc::clone(&self.shared),
            life: Arc::clone(&life),
            breaker,
            stderr: stderr_write,
            loaded: false,
        };

        self.shared.running.fetch_add(1, Ordering::SeqCst);
        let (shared, ended) = (Arc::clone(&self.shared), Arc::clone(&life));
        thread::Builder::new()
            .name("platter-fake-helper".to_owned())
            .spawn(move || {
                let code = helper.serve(engine, model, bytes);
                shared.running.fetch_sub(1, Ordering::SeqCst);
                ended.end(Exit::Code(code));
            })?;

        Ok(Spawned {
            process: Box::new(ThreadProcess {
                pid: 40_000 + self.shared.next_pid.fetch_add(1, Ordering::SeqCst),
                life,
                #[cfg(unix)]
                socket: app_socket,
                broken,
            }),
            wire,
            stderr: Box::new(stderr_read),
        })
    }
}

/// The helper's end of a connection, ready to serve on it.
trait Run: Send {
    fn serve(self: Box<Self>, engine: Faulty, model: Option<File>, bytes: u64) -> i32;
}

struct Ends<T>(T);

impl<T: Connection> Run for Ends<T> {
    fn serve(self: Box<Self>, engine: Faulty, model: Option<File>, bytes: u64) -> i32 {
        serve(engine, self.0, model, bytes)
    }
}

/// How a crash ends the connection, from the helper's side.
enum Breaker {
    /// Shut the helper's socket down: the app's end reads the end at once.
    #[cfg(unix)]
    Socket(UnixStream),
    /// Make the helper's writes fail: the helper ends at its next reply and the app's end reads the end then.
    Flag(Arc<AtomicBool>),
}

impl Breaker {
    fn break_connection(&self) {
        match self {
            #[cfg(unix)]
            Self::Socket(socket) => {
                let _ = socket.shutdown(Shutdown::Both);
            }
            Self::Flag(broken) => broken.store(true, Ordering::SeqCst),
        }
    }
}

/// The writing end of a helper's pipe that stops working when its connection is broken.
struct BreakableWriter {
    inner: io::PipeWriter,
    broken: Arc<AtomicBool>,
}

impl Write for BreakableWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }

        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }

        self.inner.flush()
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
    /// The app's end of a socket: shutting it down is what a kill does to a process's socket.
    #[cfg(unix)]
    socket: Option<UnixStream>,
    /// What a kill does to a helper that talks over pipes: its writes fail, so it ends at its next reply.
    broken: Arc<AtomicBool>,
}

impl RuntimeProcess for ThreadProcess {
    fn id(&self) -> u32 {
        self.pid
    }

    fn kill(&mut self) {
        self.life.killed.store(true, Ordering::SeqCst);
        self.broken.store(true, Ordering::SeqCst);
        #[cfg(unix)]
        if let Some(socket) = &self.socket {
            let _ = socket.shutdown(Shutdown::Both);
        }
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
    /// How a crash ends the connection.
    breaker: Breaker,
    stderr: io::PipeWriter,
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
                self.breaker.break_connection();
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
