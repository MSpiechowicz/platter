//! The advisor's worker: one thread that owns the model runtime and the download, and a handle that
//! talks to it.
//!
//! The worker is an actor. Everything that changes the model goes through its queue and runs one job at
//! a time (download, load, labelling, one question about a folder, the reading of a free question). The handle
//! refuses a request that needs the model while another job runs, instead of queueing it, so the user is never kept waiting behind a long job
//! without knowing why. Three things bypass the queue because the thread is busy exactly when they
//! matter: cancelling a download or a labelling job (flags) and quitting (the flags, then a message the
//! worker reads when the job has stopped).
//!
//! The worker serves one pinned model ([`ModelSpec`]). When it starts it also removes the files of the
//! retired model from the models folder (see [`crate::retire`]).
//!
//! Labelling and questions need the model: they load it first when it is on disk and switched on, and
//! refuse when it is switched off or not downloaded. After every job the idle clock starts again.
//!
//! Every change of the state is passed to the status callback, in order. The callback runs while the
//! state is locked: it must be quick and must not call back into the handle.

use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use platter_core::NodeId;

use crate::download::{DownloadError, Helpers, download};
use crate::engine::Engine;
use crate::facts::FolderFacts;
use crate::fetch::Fetch;
use crate::grammar::QUESTION_GRAMMAR;
use crate::interpret::{interpret_prompt, parse_question};
use crate::label::{self, Budget, Generated, LabelRun, Outcome, StageLimit};
use crate::lock::ModelLock;
use crate::parse::Judgement;
use crate::question::ScanQuestion;
use crate::retire::remove_retired;
use crate::spec::ModelSpec;
use crate::verify::{VerifyError, verify};

/// Most threads the model computes with; more are slower (see the architecture notes).
const MAX_THREADS: usize = 8;

/// Where the model is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelState {
    /// No runtime in this build.
    Unavailable,
    NotDownloaded,
    Downloading,
    OnDisk,
    Loading,
    Loaded,
}

impl ModelState {
    /// Why a download cannot start in this state, in words for the user; `None` when it can.
    pub fn download_refusal(self) -> Option<&'static str> {
        match self {
            Self::NotDownloaded => None,
            Self::OnDisk | Self::Loaded => Some("The model is already downloaded."),
            Self::Downloading | Self::Loading => Some(BUSY),
            Self::Unavailable => Some(UNAVAILABLE),
        }
    }
}

/// A claim on the state the model was in when it was read: a download approved by the user starts only
/// if nothing changed since (see [`AdvisorHandle::download_if`]). Every change of the state makes older
/// tickets stale, even a change back to the same state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateTicket(u64);

/// How long an unused model stays in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnloadAfter {
    OneMinute,
    FiveMinutes,
    /// Until the app closes.
    WindowClose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// "Use the local model": off keeps the model out of memory and refuses to load it. Off until the app
    /// sends the user's choice, so nothing loads before that.
    pub enabled: bool,
    pub unload_after: UnloadAfter,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            enabled: false,
            unload_after: UnloadAfter::FiveMinutes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub received: u64,
    pub total: u64,
}

/// How far a labelling job has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Labelling {
    /// Folders dealt with so far (answered or skipped).
    pub done: usize,
    pub total: usize,
}

/// Everything the user can be told about the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: ModelState,
    pub options: Options,
    pub download: Option<Progress>,
    /// Plain text: a failed download, a failed check, a refused load.
    pub problem: Option<String>,
    /// Memory the model added to this process, while it is loaded.
    pub ram_bytes: Option<u64>,
    /// Memory of the machine (0 when unknown).
    pub ram_total: u64,
    /// Speed of the last answer.
    pub tokens_per_second: Option<f64>,
    /// Set while a labelling job runs.
    pub labelling: Option<Labelling>,
}

pub type StatusFn = Arc<dyn Fn(&Status) + Send + Sync>;

/// How long "one minute" and "five minutes" last. Tests shorten them.
#[derive(Debug, Clone, Copy)]
pub struct IdleTimes {
    pub one_minute: Duration,
    pub five_minutes: Duration,
}

impl Default for IdleTimes {
    fn default() -> Self {
        Self {
            one_minute: Duration::from_secs(60),
            five_minutes: Duration::from_secs(300),
        }
    }
}

type Probe = Arc<dyn Fn() -> Option<u64> + Send + Sync>;

/// What the worker asks the machine. Tests replace it.
#[derive(Clone)]
pub struct Host {
    pub available_memory: Probe,
    pub total_memory: Probe,
    pub resident: Probe,
}

impl Host {
    pub fn real() -> Self {
        Self {
            available_memory: Arc::new(platter_platform::available_memory_bytes),
            total_memory: Arc::new(platter_platform::total_memory_bytes),
            resident: Arc::new(platter_platform::resident_bytes),
        }
    }
}

pub struct WorkerConfig {
    /// The pinned model.
    pub spec: ModelSpec,
    /// The folder the model files and their partial files live in.
    pub models_dir: PathBuf,
    pub threads: u32,
    pub idle: IdleTimes,
    pub host: Host,
    /// The least time between two download progress reports.
    pub progress_every: Duration,
    /// The limits of labelling jobs and questions. `None`: the model's own times for a folder, for the
    /// job and for reading a question (see [`ModelSpec::folder_limit`], [`ModelSpec::interpret_limit`]),
    /// with the usual number of tokens.
    pub budget: Option<Budget>,
}

impl WorkerConfig {
    /// A worker for the model `spec`, whose files live in `models_dir`.
    pub fn new(spec: ModelSpec, models_dir: PathBuf) -> Self {
        Self {
            spec,
            models_dir,
            threads: default_threads(),
            idle: IdleTimes::default(),
            host: Host::real(),
            progress_every: Duration::from_millis(100),
            budget: None,
        }
    }
}

/// Threads for the model on this machine: the physical cores, at most 8; when they are not known, half
/// of the hardware threads.
pub fn default_threads() -> u32 {
    let parallelism = thread::available_parallelism().map_or(2, NonZeroUsize::get);

    threads_for(platter_platform::physical_cores(), parallelism)
}

/// See [`default_threads`].
pub fn threads_for(physical_cores: Option<usize>, hardware_threads: usize) -> u32 {
    let wanted = physical_cores.unwrap_or(hardware_threads / 2);

    wanted.clamp(1, MAX_THREADS) as u32
}

/// Why a request was not taken.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkerError {
    /// The request does not fit the state of the model or the job that runs. The text is for the user.
    #[error("{0}")]
    Refused(String),
    /// The worker has stopped (the app is closing).
    #[error("The advisor has stopped.")]
    Stopped,
    /// The job began but the model did not deliver: it could not be loaded (not enough memory, a file that
    /// does not check out) or the runtime failed. The text is for the user.
    #[error("{0}")]
    Failed(String),
    /// The job was cancelled: the model was switched off or unloaded, or the app is closing.
    #[error("Cancelled.")]
    Cancelled,
    /// The model did not answer in its time.
    #[error("The model took too long to answer.")]
    TimedOut,
    /// The model wrote something that cannot be read as an answer.
    #[error("The model's answer could not be read.")]
    Unreadable,
}

enum Command {
    Download,
    Load(Sender<Status>),
    Unload(Sender<Status>),
    /// The options were already stored; look at them (unload when switched off, restart the idle clock).
    Configure(Option<Sender<Status>>),
    Label {
        items: Vec<(NodeId, FolderFacts)>,
        reply: Sender<Result<LabelRun, WorkerError>>,
    },
    Ask {
        facts: Box<FolderFacts>,
        reply: Sender<Result<Judgement, WorkerError>>,
    },
    /// A free question: reads the (cleaned) question into a query.
    Interpret {
        question: String,
        reply: Sender<Result<ScanQuestion, WorkerError>>,
    },
    Shutdown,
}

/// The job that uses the model, if one is queued or running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Busy {
    No,
    Labelling,
    /// A question about a folder, or the reading of a free question.
    Asking,
}

struct Inner {
    status: Status,
    /// Counts changes of `status.state`; see [`StateTicket`].
    generation: u64,
    busy: Busy,
    /// Memory in use before the model was loaded; the model's share is measured from it.
    baseline: Option<u64>,
    last_active: Instant,
}

impl Inner {
    /// The only way the state changes, so that every change is counted.
    fn set_state(&mut self, state: ModelState) {
        self.status.state = state;
        self.generation += 1;
    }
}

struct Shared {
    inner: Mutex<Inner>,
    spec: ModelSpec,
    models_dir: PathBuf,
    on_status: StatusFn,
    host: Host,
    download_cancel: AtomicBool,
    /// Raised to stop a labelling job or a question.
    job_cancel: AtomicBool,
    closing: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn snapshot(&self, inner: &Inner) -> Status {
        let mut status = inner.status.clone();

        status.ram_total = (self.host.total_memory)().unwrap_or(0);
        status.ram_bytes = match (status.state, inner.baseline) {
            (ModelState::Loaded, Some(before)) => {
                (self.host.resident)().map(|now| now.saturating_sub(before))
            }
            _ => None,
        };
        status
    }

    fn status(&self) -> Status {
        self.snapshot(&self.lock())
    }

    /// Changes the state and tells the callback, in one step so events keep their order.
    fn change(&self, change: impl FnOnce(&mut Inner)) -> Status {
        let mut inner = self.lock();
        change(&mut inner);

        let status = self.snapshot(&inner);
        (self.on_status)(&status);
        status
    }
}

/// Starts the worker.
pub struct AdvisorWorker;

impl AdvisorWorker {
    /// Starts the worker thread. The state starts as what is on disk: a model file of exactly the
    /// model's size counts as `OnDisk` (it is checked in full before every load).
    pub fn spawn(
        config: WorkerConfig,
        engine: Box<dyn Engine>,
        fetch: Arc<dyn Fetch>,
        on_status: StatusFn,
    ) -> std::io::Result<AdvisorHandle> {
        // The retired model's files go first, so the folder is as the status below describes it.
        remove_retired(&config.models_dir);

        let spec = config.spec;
        let found = found_on_disk(&spec, &config.models_dir);

        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                status: Status {
                    state: if found.on_disk {
                        ModelState::OnDisk
                    } else {
                        ModelState::NotDownloaded
                    },
                    options: Options::default(),
                    download: found.received.map(|received| Progress {
                        received,
                        total: spec.bytes,
                    }),
                    problem: None,
                    ram_bytes: None,
                    ram_total: 0,
                    tokens_per_second: None,
                    labelling: None,
                },
                generation: 0,
                busy: Busy::No,
                baseline: None,
                last_active: Instant::now(),
            }),
            spec,
            models_dir: config.models_dir.clone(),
            on_status,
            host: config.host.clone(),
            download_cancel: AtomicBool::new(false),
            job_cancel: AtomicBool::new(false),
            closing: AtomicBool::new(false),
        });

        let (commands, queue) = mpsc::channel();
        let worker = Worker {
            shared: Arc::clone(&shared),
            config,
            engine,
            fetch,
            helpers: Helpers::new(),
        };
        let thread = thread::Builder::new()
            .name("platter-advisor".to_owned())
            .spawn(move || worker.run(&queue))?;

        Ok(AdvisorHandle {
            shared,
            commands,
            thread: Mutex::new(Some(thread)),
        })
    }
}

/// What the models folder holds of the model: the whole file, or the part of an earlier download.
struct Found {
    on_disk: bool,
    /// Bytes of a partial download that are kept.
    received: Option<u64>,
}

fn found_on_disk(spec: &ModelSpec, dir: &std::path::Path) -> Found {
    let on_disk = std::fs::symlink_metadata(dir.join(spec.file))
        .is_ok_and(|meta| meta.is_file() && meta.len() == spec.bytes);
    let kept = if on_disk {
        None
    } else {
        kept_part(&dir.join(spec.partial_file()), spec.bytes)
    };

    Found {
        on_disk,
        received: kept.map(|progress| progress.received),
    }
}

/// The part of an earlier download kept on disk, if any.
fn kept_part(partial: &std::path::Path, total: u64) -> Option<Progress> {
    let received = std::fs::symlink_metadata(partial)
        .ok()
        .filter(|meta| meta.is_file())
        .map_or(0, |meta| meta.len().min(total));

    (received > 0).then_some(Progress { received, total })
}

/// The way to the worker. Cheap calls answer at once; `load` and `unload` wait for the worker.
pub struct AdvisorHandle {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl AdvisorHandle {
    /// The state now.
    pub fn status(&self) -> Status {
        self.shared.status()
    }

    /// The state now and a ticket for it, read together.
    pub fn status_with_ticket(&self) -> (Status, StateTicket) {
        let inner = self.shared.lock();

        (self.shared.snapshot(&inner), StateTicket(inner.generation))
    }

    /// Starts the download (the model must not be on disk). Progress and the end arrive through the
    /// status callback.
    pub fn download(&self) -> Result<(), WorkerError> {
        let inner = self.shared.lock();

        self.start_download(inner)
    }

    /// Like [`download`](Self::download), but only when the state is the one `ticket` was read from: a
    /// download the user approved for one state is not started in another (the state is compared under
    /// the same lock the download is queued under).
    pub fn download_if(&self, ticket: StateTicket) -> Result<(), WorkerError> {
        let inner = self.shared.lock();
        if inner.generation != ticket.0 {
            return Err(refused("The model's state changed; try again."));
        }

        self.start_download(inner)
    }

    fn start_download(&self, mut inner: MutexGuard<'_, Inner>) -> Result<(), WorkerError> {
        self.check_open()?;

        if let Some(reason) = inner.status.state.download_refusal() {
            return Err(refused(reason));
        }

        let spec = self.shared.spec;
        let received = std::fs::symlink_metadata(self.shared.models_dir.join(spec.partial_file()))
            .ok()
            .filter(|meta| meta.is_file())
            .map_or(0, |meta| meta.len().min(spec.bytes));
        // Queued while the state is locked: `shutdown` takes the same lock before it closes the queue,
        // so a download that was accepted is always queued before the shutdown message, and one that
        // comes after is refused above. Queued before the status is told, so a callback that fails
        // cannot leave the state `Downloading` with nothing queued; the worker cannot act on it before
        // the lock is released.
        self.shared.download_cancel.store(false, Ordering::SeqCst);
        self.commands
            .send(Command::Download)
            .map_err(|_| WorkerError::Stopped)?;

        inner.set_state(ModelState::Downloading);
        inner.status.problem = None;
        inner.status.download = Some(Progress {
            received,
            total: spec.bytes,
        });
        let status = self.shared.snapshot(&inner);
        (self.shared.on_status)(&status);

        Ok(())
    }

    /// Whether a labelling job or a question (about a folder, or the reading of a free question) runs now.
    pub fn job_running(&self) -> bool {
        self.shared.lock().busy != Busy::No
    }

    /// Runs `see` with the state now, while the state is locked, so no change is told in between: a new
    /// watcher installed in `see` gets this status first and every later change after it.
    pub fn observe(&self, see: impl FnOnce(&Status)) {
        let inner = self.shared.lock();
        see(&self.shared.snapshot(&inner));
    }

    /// Stops a running download; the partial file stays for a later resume. Nothing happens when no
    /// download runs.
    pub fn cancel_download(&self) {
        if self.shared.lock().status.state == ModelState::Downloading {
            self.shared.download_cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Checks the model file and loads it. Waits until the model is loaded or the load failed; a failure
    /// is in the returned status (`problem`), not an error.
    pub fn load(&self) -> Result<Status, WorkerError> {
        let mut inner = self.shared.lock();
        self.check_open()?;

        match inner.status.state {
            ModelState::OnDisk if inner.busy != Busy::No => return Err(refused(BUSY)),
            ModelState::OnDisk => {}
            ModelState::Loaded => return Ok(self.shared.snapshot(&inner)),
            ModelState::NotDownloaded => return Err(refused("The model has not been downloaded.")),
            ModelState::Downloading | ModelState::Loading => return Err(refused(BUSY)),
            ModelState::Unavailable => return Err(refused(UNAVAILABLE)),
        }
        if !inner.status.options.enabled {
            return Err(refused("The local model is switched off in the settings."));
        }

        // Queued first, as in `download`: a failing callback cannot leave `Loading` with nothing queued.
        let answer = self.queue(Command::Load)?;
        inner.set_state(ModelState::Loading);
        inner.status.problem = None;
        let status = self.shared.snapshot(&inner);
        (self.shared.on_status)(&status);
        drop(inner);

        wait_for(&answer)
    }

    /// Frees the model's memory. Waits until it is done.
    pub fn unload(&self) -> Result<Status, WorkerError> {
        let inner = self.shared.lock();
        self.check_open()?;

        match inner.status.state {
            ModelState::Loaded if inner.busy != Busy::No => return Err(refused(BUSY)),
            ModelState::Loaded => {}
            ModelState::OnDisk | ModelState::NotDownloaded => return Ok(self.shared.snapshot(&inner)),
            ModelState::Downloading | ModelState::Loading => return Err(refused(BUSY)),
            ModelState::Unavailable => return Err(refused(UNAVAILABLE)),
        }
        let answer = self.queue(Command::Unload)?;
        drop(inner);

        wait_for(&answer)
    }

    /// Stores the options. Switching the model off unloads it (and waits for that).
    pub fn configure(&self, options: Options) -> Result<Status, WorkerError> {
        let mut inner = self.shared.lock();
        self.check_open()?;

        let changed = inner.status.options != options;
        inner.status.options = options;
        let state = inner.status.state;
        let status = self.shared.snapshot(&inner);
        if changed {
            (self.shared.on_status)(&status);
        }

        let in_use = matches!(state, ModelState::Loaded | ModelState::Loading) || inner.busy != Busy::No;
        if !options.enabled && in_use {
            // A running job is stopped; the model is freed when it has ended.
            if inner.busy != Busy::No {
                self.shared.job_cancel.store(true, Ordering::SeqCst);
            }
            let answer = self.queue(|reply| Command::Configure(Some(reply)))?;
            drop(inner);

            return wait_for(&answer);
        }
        if changed {
            // Only the idle clock needs to know.
            self.commands
                .send(Command::Configure(None))
                .map_err(|_| WorkerError::Stopped)?;
        }
        Ok(status)
    }

    /// Asks the model about every folder of `items` and waits until the job has ended. The model is
    /// loaded first when it is on disk. Progress arrives through the status callback (`labelling`). A job
    /// that stops early (cancelled, out of time) still returns the labels made so far.
    ///
    /// Refused when the model is switched off or not downloaded, or when another job runs.
    pub fn label(&self, items: Vec<(NodeId, FolderFacts)>) -> Result<LabelRun, WorkerError> {
        let mut inner = self.shared.lock();
        self.check_open()?;
        self.check_model_job(&inner)?;

        if items.is_empty() {
            return Ok(LabelRun {
                labels: Vec::new(),
                ms: 0,
                stopped: None,
                tokens_per_s: None,
            });
        }

        let total = items.len();
        // Queued before the state changes, as in `download`.
        self.shared.job_cancel.store(false, Ordering::SeqCst);
        let answer = self.queue(|reply| Command::Label { items, reply })?;
        inner.busy = Busy::Labelling;
        inner.status.labelling = Some(Labelling { done: 0, total });
        let status = self.shared.snapshot(&inner);
        (self.shared.on_status)(&status);
        drop(inner);

        answer.recv().map_err(|_| WorkerError::Stopped)?
    }

    /// Stops a running labelling job; the labels made so far are kept. Nothing happens when none runs.
    pub fn cancel_labelling(&self) {
        if self.shared.lock().busy == Busy::Labelling {
            self.shared.job_cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Asks the model about one folder and waits for the answer. The model is loaded first when it is on
    /// disk. Refused while a labelling job (or another question) runs, when the model is switched off or
    /// not downloaded.
    pub fn ask(&self, facts: FolderFacts) -> Result<Judgement, WorkerError> {
        let mut inner = self.shared.lock();
        self.check_open()?;
        self.check_model_job(&inner)?;

        self.shared.job_cancel.store(false, Ordering::SeqCst);
        let answer = self.queue(|reply| Command::Ask {
            facts: Box::new(facts),
            reply,
        })?;
        inner.busy = Busy::Asking;
        drop(inner);

        answer.recv().map_err(|_| WorkerError::Stopped)?
    }

    /// A free question: the model reads `question` (already cleaned) into a query. The model is
    /// loaded first when it is on disk. Refused while labelling or another job runs, when the model is
    /// switched off or not downloaded. It ends in `TimedOut` or `Unreadable` when the model runs out of
    /// time or writes something the parser rejects (the caller may then ask the general question),
    /// `Cancelled` when the model is switched off meanwhile and `Failed` when it cannot be loaded.
    pub fn interpret(&self, question: &str) -> Result<ScanQuestion, WorkerError> {
        let mut inner = self.shared.lock();
        self.check_open()?;
        self.check_model_job(&inner)?;

        self.shared.job_cancel.store(false, Ordering::SeqCst);
        let answer = self.queue(|reply| Command::Interpret {
            question: question.to_owned(),
            reply,
        })?;
        inner.busy = Busy::Asking;
        drop(inner);

        answer.recv().map_err(|_| WorkerError::Stopped)?
    }

    /// Stops the worker: a download is cancelled (its partial file stays), a labelling job or question
    /// is cancelled, the model is unloaded, and the call returns when the thread has ended. A load in
    /// progress is finished first. Requests made after this are refused.
    pub fn shutdown(&self) {
        {
            // Under the state lock, so that a request that passed `check_open` has finished queueing
            // (and a download its flag) before the flags are set and the queue is told to stop.
            let _inner = self.shared.lock();
            self.shared.closing.store(true, Ordering::SeqCst);
            self.shared.download_cancel.store(true, Ordering::SeqCst);
            self.shared.job_cancel.store(true, Ordering::SeqCst);
            let _ = self.commands.send(Command::Shutdown);
        }

        let thread = self
            .thread
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }

    /// Whether a job that uses the model (labelling, a question) may start now. Called with the state
    /// locked.
    fn check_model_job(&self, inner: &Inner) -> Result<(), WorkerError> {
        if inner.busy != Busy::No {
            return Err(refused(BUSY));
        }

        match inner.status.state {
            ModelState::Loaded | ModelState::OnDisk => {}
            ModelState::NotDownloaded => return Err(refused("The model has not been downloaded.")),
            ModelState::Downloading | ModelState::Loading => return Err(refused(BUSY)),
            ModelState::Unavailable => return Err(refused(UNAVAILABLE)),
        }
        if !inner.status.options.enabled {
            return Err(refused("The local model is switched off in the settings."));
        }
        Ok(())
    }

    fn check_open(&self) -> Result<(), WorkerError> {
        if self.shared.closing.load(Ordering::SeqCst) {
            return Err(WorkerError::Stopped);
        }
        Ok(())
    }

    /// Queues a command with a reply channel and returns where the answer comes. Called with the state
    /// locked (see `shutdown`); the caller waits for the answer after it has let go of the lock.
    fn queue<T>(&self, command: impl FnOnce(Sender<T>) -> Command) -> Result<Receiver<T>, WorkerError> {
        let (reply, answer) = mpsc::channel();
        self.commands
            .send(command(reply))
            .map_err(|_| WorkerError::Stopped)?;

        Ok(answer)
    }
}

fn wait_for(answer: &Receiver<Status>) -> Result<Status, WorkerError> {
    answer.recv().map_err(|_| WorkerError::Stopped)
}

impl Drop for AdvisorHandle {
    fn drop(&mut self) {
        // The thread ends by itself when the queue closes; a running download should not outlive the app.
        self.shared.closing.store(true, Ordering::SeqCst);
        self.shared.download_cancel.store(true, Ordering::SeqCst);
        self.shared.job_cancel.store(true, Ordering::SeqCst);
    }
}

const BUSY: &str = "Another model job is running. Wait for it to finish.";
const UNAVAILABLE: &str = "The local model is not available in this build.";

fn refused(text: &str) -> WorkerError {
    WorkerError::Refused(text.to_owned())
}

struct Worker {
    shared: Arc<Shared>,
    config: WorkerConfig,
    engine: Box<dyn Engine>,
    fetch: Arc<dyn Fetch>,
    /// The network threads of every download so far, so abandoned ones cannot pile up.
    helpers: Helpers,
}

impl Worker {
    fn run(mut self, queue: &mpsc::Receiver<Command>) {
        loop {
            let command = match self.idle_deadline() {
                Some(deadline) => {
                    match queue.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(command) => command,
                        Err(RecvTimeoutError::Timeout) => {
                            self.guarded_unload();
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None => match queue.recv() {
                    Ok(command) => command,
                    Err(_) => break,
                },
            };

            match command {
                Command::Download => self.guarded_download(),
                Command::Load(reply) => {
                    let _ = reply.send(self.guarded_load());
                }
                Command::Unload(reply) => {
                    let _ = reply.send(self.guarded_unload());
                }
                Command::Configure(reply) => {
                    let status = self.guarded_configure();
                    if let Some(reply) = reply {
                        let _ = reply.send(status);
                    }
                }
                Command::Label { items, reply } => {
                    let _ = reply.send(self.guarded_label(&items));
                }
                Command::Ask { facts, reply } => {
                    let _ = reply.send(self.guarded_ask(&facts));
                }
                Command::Interpret { question, reply } => {
                    let _ = reply.send(self.guarded_interpret(&question));
                }
                Command::Shutdown => break,
            }
        }

        self.guarded_unload();
    }

    // A panic in a job must not leave the state stuck on `Downloading` or `Loading` (the handle would
    // refuse everything and a caller would wait for its answer forever). Each job runs behind
    // `catch_unwind`; after a panic the state is put back to the nearest sane one and the problem told.

    fn guarded_download(&mut self) {
        if catch_unwind(AssertUnwindSafe(|| self.download())).is_ok() {
            return;
        }

        let spec = self.spec();
        let kept = kept_part(&self.config.models_dir.join(spec.partial_file()), spec.bytes);
        self.recover(|inner| {
            inner.set_state(ModelState::NotDownloaded);
            inner.status.download = kept;
            inner.status.problem = Some("The download stopped unexpectedly.".to_owned());
        });
    }

    fn guarded_load(&mut self) -> Status {
        if let Ok(status) = catch_unwind(AssertUnwindSafe(|| self.load())) {
            return status;
        }

        // The runtime may hold a half-loaded model.
        let _ = catch_unwind(AssertUnwindSafe(|| self.engine.unload()));
        self.recover(|inner| {
            inner.set_state(ModelState::OnDisk);
            inner.status.problem = Some("Loading the model stopped unexpectedly.".to_owned());
            inner.baseline = None;
        })
    }

    fn guarded_unload(&mut self) -> Status {
        if let Ok(status) = catch_unwind(AssertUnwindSafe(|| self.unload())) {
            return status;
        }

        self.recover(|inner| {
            if inner.status.state == ModelState::Loaded {
                inner.set_state(ModelState::OnDisk);
            }
            inner.status.tokens_per_second = None;
            inner.baseline = None;
        })
    }

    fn guarded_label(&mut self, items: &[(NodeId, FolderFacts)]) -> Result<LabelRun, WorkerError> {
        let result = match catch_unwind(AssertUnwindSafe(|| self.label_job(items))) {
            Ok(result) => result,
            Err(_) => {
                self.recover_from_job_panic();
                Err(WorkerError::Failed("The model stopped unexpectedly.".to_owned()))
            }
        };

        let speed = result.as_ref().ok().and_then(|run| run.tokens_per_s);
        self.finish_job(speed);
        result
    }

    fn guarded_ask(&mut self, facts: &FolderFacts) -> Result<Judgement, WorkerError> {
        let (result, speed) = match catch_unwind(AssertUnwindSafe(|| self.ask_job(facts))) {
            Ok(done) => done,
            Err(_) => {
                self.recover_from_job_panic();
                (
                    Err(WorkerError::Failed("The model stopped unexpectedly.".to_owned())),
                    None,
                )
            }
        };

        self.finish_job(speed);
        result
    }

    /// Runs the reading of a free question behind `catch_unwind`, like the other jobs, and ends the job.
    fn guarded_interpret(&mut self, question: &str) -> Result<ScanQuestion, WorkerError> {
        let (result, speed) = match catch_unwind(AssertUnwindSafe(|| self.interpret_job(question))) {
            Ok(done) => done,
            Err(_) => {
                self.recover_from_job_panic();
                (
                    Err(WorkerError::Failed("The model stopped unexpectedly.".to_owned())),
                    None,
                )
            }
        };

        self.finish_job(speed);
        result
    }

    /// The runtime may be half way through an answer: free it, so the next job loads afresh.
    fn recover_from_job_panic(&mut self) {
        let _ = catch_unwind(AssertUnwindSafe(|| self.engine.unload()));
        self.recover(|inner| {
            inner.set_state(ModelState::OnDisk);
            inner.status.problem = Some("The model stopped unexpectedly.".to_owned());
            inner.status.tokens_per_second = None;
            inner.baseline = None;
        });
    }

    /// Ends a job: no job runs, the idle clock starts again, and the speed of the last answer is shown.
    fn finish_job(&self, speed: Option<f64>) {
        self.recover(|inner| {
            inner.busy = Busy::No;
            inner.status.labelling = None;
            inner.last_active = Instant::now();
            if speed.is_some() {
                inner.status.tokens_per_second = speed;
            }
        });
    }

    fn guarded_configure(&mut self) -> Status {
        match catch_unwind(AssertUnwindSafe(|| self.configure())) {
            Ok(status) => status,
            Err(_) => self.shared.status(),
        }
    }

    /// Sets the state after a panic. The callback or a probe may be what panicked, so a second panic
    /// here is swallowed; the state itself is changed before either runs.
    fn recover(&self, change: impl FnOnce(&mut Inner)) -> Status {
        catch_unwind(AssertUnwindSafe(|| self.shared.change(change)))
            .unwrap_or_else(|_| self.shared.lock().status.clone())
    }

    /// When the loaded model has been idle long enough to unload; `None` when it never is.
    fn idle_deadline(&self) -> Option<Instant> {
        let inner = self.shared.lock();
        if inner.status.state != ModelState::Loaded {
            return None;
        }

        let wait = match inner.status.options.unload_after {
            UnloadAfter::OneMinute => self.config.idle.one_minute,
            UnloadAfter::FiveMinutes => self.config.idle.five_minutes,
            UnloadAfter::WindowClose => return None,
        };
        Some(inner.last_active + wait)
    }

    /// Makes sure the model is in memory: loads it when it is on disk.
    fn ensure_loaded(&mut self) -> Result<(), WorkerError> {
        let state = self.shared.lock().status.state;

        match state {
            ModelState::Loaded => Ok(()),
            ModelState::OnDisk => {
                self.shared.change(|inner| {
                    inner.set_state(ModelState::Loading);
                    inner.status.problem = None;
                });

                let status = self.load();
                if status.state == ModelState::Loaded {
                    return Ok(());
                }
                let problem = status
                    .problem
                    .unwrap_or_else(|| "The model could not be loaded.".to_owned());
                Err(WorkerError::Failed(problem))
            }
            _ => Err(refused("The model is not available.")),
        }
    }

    fn label_job(&mut self, items: &[(NodeId, FolderFacts)]) -> Result<LabelRun, WorkerError> {
        self.ensure_loaded()?;

        let budget = self.budget();
        let shared = &self.shared;
        let total = items.len();
        shared.change(|inner| inner.status.labelling = Some(Labelling { done: 0, total }));

        let run = label::label(
            &mut *self.engine,
            items,
            &budget,
            &shared.job_cancel,
            &|done, total| {
                shared.change(|inner| inner.status.labelling = Some(Labelling { done, total }));
            },
        );
        Ok(run)
    }

    /// Answers one question; the second part is the speed of the answer, when there was one.
    fn ask_job(&mut self, facts: &FolderFacts) -> (Result<Judgement, WorkerError>, Option<f64>) {
        if let Err(error) = self.ensure_loaded() {
            return (Err(error), None);
        }

        let budget = self.budget();
        let outcome = label::ask(
            &mut *self.engine,
            facts,
            budget.per_folder,
            budget.max_tokens,
            &self.shared.job_cancel,
        );

        match outcome {
            Outcome::Judged(judgement, generation) => (Ok(judgement), Some(generation.tokens_per_s)),
            Outcome::Cancelled => (Err(WorkerError::Cancelled), None),
            Outcome::TimedOut => (Err(WorkerError::TimedOut), None),
            Outcome::Unreadable => (Err(WorkerError::Unreadable), None),
            Outcome::Failed(error) => (Err(WorkerError::Failed(error.to_string())), None),
        }
    }

    /// The model reads the question into a query: it is loaded first when needed, run on the question's
    /// prompt within the budget's time and tokens, and what it wrote is read. The second part is the speed
    /// of the answer, when there was one.
    fn interpret_job(&mut self, question: &str) -> (Result<ScanQuestion, WorkerError>, Option<f64>) {
        let limit = self.budget().interpret;

        if let Err(error) = self.ensure_loaded() {
            return (Err(error), None);
        }

        let generated = label::generate(
            &mut *self.engine,
            &interpret_prompt(question),
            QUESTION_GRAMMAR,
            limit.time,
            limit.max_tokens,
            &self.shared.job_cancel,
        );

        match generated {
            Generated::Done(generation) => match parse_question(&generation.text) {
                Ok(query) => (Ok(query), Some(generation.tokens_per_s)),
                Err(_) => (Err(WorkerError::Unreadable), None),
            },
            Generated::Cancelled => (Err(WorkerError::Cancelled), None),
            Generated::TimedOut => (Err(WorkerError::TimedOut), None),
            Generated::Failed(error) => (Err(WorkerError::Failed(error.to_string())), None),
        }
    }

    fn spec(&self) -> ModelSpec {
        self.config.spec
    }

    /// The limits of a job: the ones the config fixes, or else the model's own times.
    fn budget(&self) -> Budget {
        let spec = self.spec();

        let usual = Budget::default();

        self.config.budget.unwrap_or(Budget {
            per_folder: spec.folder_limit,
            total: spec.job_limit,
            interpret: StageLimit {
                time: spec.interpret_limit,
                ..usual.interpret
            },
            ..usual
        })
    }

    fn download(&mut self) {
        let spec = self.spec();
        let every = self.config.progress_every;
        let shared = &self.shared;
        let last_report = Mutex::new(None::<Instant>);

        let report = |received: u64, total: u64| {
            let mut last = last_report
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let due = last.is_none_or(|at| at.elapsed() >= every);
            if !due && received < total {
                return;
            }
            *last = Some(Instant::now());
            shared.change(|inner| inner.status.download = Some(Progress { received, total }));
        };

        let outcome = download(
            &spec,
            &self.config.models_dir,
            Arc::clone(&self.fetch),
            &self.shared.download_cancel,
            &report,
            &self.helpers,
        );

        // What arrived before a cancel or a failure stays on disk; the status shows it so the UI can
        // offer to continue.
        let kept = match outcome {
            Ok(()) => None,
            Err(_) => kept_part(&self.config.models_dir.join(spec.partial_file()), spec.bytes),
        };

        self.shared.change(|inner| {
            inner.status.download = kept;
            match outcome {
                Ok(()) => inner.set_state(ModelState::OnDisk),
                // The partial file stays; asking again continues it.
                Err(DownloadError::Cancelled) => inner.set_state(ModelState::NotDownloaded),
                Err(error) => {
                    inner.set_state(ModelState::NotDownloaded);
                    inner.status.problem = Some(error.to_string());
                }
            }
        });
    }

    fn load(&mut self) -> Status {
        let spec = self.spec();

        if let Some(problem) = self.not_enough_memory(&spec) {
            return self.fail_load(ModelState::OnDisk, problem);
        }

        // Checking (and deleting a wrong file) is one step for one process at a time: another window may
        // be downloading, or deleting, in this folder.
        let lock = match ModelLock::try_acquire(&self.config.models_dir) {
            Ok(Some(lock)) => Some(lock),
            Ok(None) => {
                return self.fail_load(
                    ModelState::OnDisk,
                    "Another Platter window is using the model files. Try again in a moment.".to_owned(),
                );
            }
            // No folder: the check below says the file is missing.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return self.fail_load(
                    ModelState::OnDisk,
                    format!("The model folder cannot be locked: {error}"),
                );
            }
        };

        let path = self.config.models_dir.join(spec.file);
        let verified = match verify(&spec, &path) {
            Ok(verified) => verified,
            Err(error) => {
                if error.is_corrupt() {
                    // What is there is not the model: remove it, so a new download starts clean.
                    let _ = std::fs::remove_file(&path);
                }
                let state = match error {
                    VerifyError::Io(_) | VerifyError::Unsupported => ModelState::OnDisk,
                    _ => ModelState::NotDownloaded,
                };
                return self.fail_load(state, error.to_string());
            }
        };

        // The file is held by its descriptor from here on; nobody needs to wait for the load.
        drop(lock);

        let before = (self.shared.host.resident)();
        let threads = self.config.threads;
        let loaded = catch_unwind(AssertUnwindSafe(|| self.engine.load(&verified, threads)));
        drop(verified);

        match loaded {
            Ok(Ok(_)) if !self.shared.lock().status.options.enabled => {
                // Switched off while it loaded.
                self.engine.unload();
                self.fail_load(
                    ModelState::OnDisk,
                    "The local model is switched off in the settings.".to_owned(),
                )
            }
            Ok(Ok(_)) => self.shared.change(|inner| {
                inner.set_state(ModelState::Loaded);
                inner.status.problem = None;
                inner.status.tokens_per_second = None;
                inner.baseline = before;
                inner.last_active = Instant::now();
            }),
            Ok(Err(error)) => self.fail_load(ModelState::OnDisk, error.to_string()),
            Err(_) => {
                self.engine.unload();
                self.fail_load(
                    ModelState::OnDisk,
                    "The model runtime stopped unexpectedly.".to_owned(),
                )
            }
        }
    }

    fn fail_load(&self, state: ModelState, problem: String) -> Status {
        self.shared.change(|inner| {
            inner.set_state(state);
            inner.status.problem = Some(problem);
        })
    }

    /// The reason a load must not start, when the machine has too little free memory.
    fn not_enough_memory(&self, spec: &ModelSpec) -> Option<String> {
        let available = (self.shared.host.available_memory)()?;
        let needed = spec.min_free_bytes;

        (available < needed).then(|| {
            format!(
                "There is not enough free memory to load the model: about {} is needed and {} is free.",
                gigabytes(needed),
                gigabytes(available)
            )
        })
    }

    fn unload(&mut self) -> Status {
        let was_loaded = self.shared.lock().status.state == ModelState::Loaded;
        if !was_loaded {
            return self.shared.status();
        }

        self.engine.unload();
        self.shared.change(|inner| {
            inner.set_state(ModelState::OnDisk);
            inner.status.tokens_per_second = None;
            inner.baseline = None;
        })
    }

    fn configure(&mut self) -> Status {
        let enabled = self.shared.lock().status.options.enabled;
        if !enabled {
            return self.guarded_unload();
        }

        // A new idle time counts from now.
        let mut inner = self.shared.lock();
        inner.last_active = Instant::now();
        self.shared.snapshot(&inner)
    }
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_are_the_physical_cores_but_at_most_eight() {
        assert_eq!(threads_for(Some(8), 16), 8);
        assert_eq!(threads_for(Some(4), 8), 4);
        assert_eq!(threads_for(Some(32), 64), 8);
        assert_eq!(threads_for(Some(0), 4), 1);
    }

    #[test]
    fn without_core_ids_half_the_hardware_threads_are_used() {
        assert_eq!(threads_for(None, 16), 8);
        assert_eq!(threads_for(None, 8), 4);
        assert_eq!(threads_for(None, 2), 1);
        assert_eq!(threads_for(None, 1), 1);
        assert_eq!(threads_for(None, 128), 8);
    }

    #[test]
    fn sizes_are_told_in_gigabytes() {
        assert_eq!(gigabytes(1_280_835_840), "1.3 GB");
    }
}
