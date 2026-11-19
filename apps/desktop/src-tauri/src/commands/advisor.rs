//! The advisor model over IPC: its status, its download, loading and unloading, and the options (A1);
//! labelling folders, asking about one folder and finding folders (A2). The web view sends no path and no
//! address: the files, the folder and the download addresses are fixed in `platter-ai`, and folders are
//! named by node ids of a scan. There is one pinned model; the web view cannot name another.
//!
//! Every free question is also appended to a local log (see [`log`]), unless the user switched that off.
//!
//! The model runs in a helper process of its own (the app's binary started again, see `platter_ai::process`),
//! so that a graphics driver that crashes or hangs cannot take the app down. What goes wrong on the card is
//! written to a second local log (see [`gpu_log`]) and reaches the web view as `gpuProblem` in the status,
//! which asks the user what to do (`answer_gpu_problem`); `clear_gpu_log` deletes the log.
//!
//! The download is confirmed by the backend itself, in a native dialog, before every start: the consent
//! is not left to the web view (see [`download_model`]).
//!
//! [`AdvisorService`] is what the commands talk to. It owns the worker thread of `platter-ai`, turns
//! its status into the contract's DTO, and pushes every change to the one watcher (a new `watch_advisor`
//! replaces the previous one). Where the runtime is not built in, it answers `unavailable`.
//!
//! A labelling job runs on a thread of its own: `label_folders` answers when the job has been taken, and
//! the job's labels are stored with the scan when it ends. While it runs, the status names the scan.

use std::collections::HashSet;
#[cfg(any(feature = "advisor", test))]
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use platter_ai::{
    AdvisorHandle, Device, FolderFacts, LabelRun, MODEL, ModelSpec, ModelState, Options, RuleContext,
    ScanQuestion, StateTicket, Status, Stopped, UnloadAfter, WorkerError, facts,
};
#[cfg(any(feature = "advisor", test))]
use platter_ai::{AdvisorWorker, Engine, Fetch, WorkerConfig};
#[cfg(feature = "advisor")]
use platter_ai::{ProcessEngine, RealSpawner};
use platter_cleanup::{CandidateLimits, Protected, fenced, label_candidates, with_real_keeps};
use platter_core::dto::{
    ApiErrorCode, OsDto, ScanId, SuggestionDto, SuggestionSource, SuggestionsDto, Verdict,
};
use platter_core::{
    EntryProbe, Environment, ModelLabel, ModelVerdict, NodeId, NodeKind, Tree, Units, UserRule, UserVerdict,
};
use platter_core::{find_entries_until as search_entries_until, find_folders_until as search_folders_until};
use platter_platform::Platform;
use platter_platform::environment::current_os;
use platter_platform::paths::node_path;
use platter_rules::{Explainer, RuleSet, scan_is_kept};
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

use self::gpu_log::{Choice, GpuLog, Problem, What};
use self::question::SizeUnitsDto;
use crate::commands::cleanup::Question;
use crate::commands::rules::UserVerdictDto;
use crate::commands::suggestions;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, LabelSet};
use crate::user_rules;

/// The largest integer a JavaScript number holds exactly; sizes are clamped to it on the wire.
const MAX_SAFE: u64 = 9_007_199_254_740_991;
/// Matches `find_folders` and `find_entries` may return (the contract: 1 to 20).
const MAX_FOUND: u32 = 20;
/// How long `label_folders` waits to hear that the worker took the job (a refusal is immediate).
const TAKEN_WITHIN: Duration = Duration::from_millis(250);
/// How long after a declined download dialog a new download request is refused (no rapid re-asking, as
/// with the cleanup's confirmation).
const DECLINE_PAUSE: Duration = Duration::from_secs(2);

// ---- Wire types ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelStateDto {
    Unavailable,
    NotDownloaded,
    Downloading,
    OnDisk,
    Loading,
    Loaded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnloadAfterDto {
    OneMinute,
    FiveMinutes,
    WindowClose,
}

/// What the user chose in Settings. The only advisor type the web view sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdvisorOptionsDto {
    pub enabled: bool,
    pub unload_after: UnloadAfterDto,
    /// Whether each free question is written to the local question log (see [`log`]); on unless the
    /// message leaves it out or says `false`. Never sent anywhere.
    #[serde(default = "log_questions_by_default")]
    pub log_questions: bool,
    /// "Use the graphics card": on unless the message leaves it out or says `false`.
    #[serde(default = "use_gpu_by_default")]
    pub use_gpu: bool,
}

fn log_questions_by_default() -> bool {
    true
}

fn use_gpu_by_default() -> bool {
    true
}

/// Where the loaded model runs. Sent only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKindDto {
    Gpu,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDto {
    pub kind: DeviceKindDto,
    /// The driver's description of the graphics card; only for `gpu`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl From<&Device> for DeviceDto {
    fn from(device: &Device) -> Self {
        match device {
            Device::Cpu => Self {
                kind: DeviceKindDto::Cpu,
                name: None,
            },
            Device::Gpu { name } => Self {
                kind: DeviceKindDto::Gpu,
                name: Some(name.clone()),
            },
        }
    }
}

/// Whether this build can run the model on a graphics card: every Linux build with the advisor (the
/// desktop crate's target dependency turns Vulkan on there), every macOS build with it (Metal, the same
/// way), and any build that asks for `gpu-vulkan` or `gpu-metal`.
const GPU_SUPPORTED: bool = cfg!(any(
    feature = "gpu-vulkan",
    feature = "gpu-metal",
    all(feature = "advisor", any(target_os = "linux", target_os = "macos"))
));

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfoDto {
    pub name: String,
    pub format: String,
    pub bytes: u64,
    /// Where the download comes from, for showing ("huggingface.co").
    pub host: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDto {
    pub received: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorStatusDto {
    pub state: ModelStateDto,
    /// The pinned model.
    pub model: ModelInfoDto,
    pub options: AdvisorOptionsDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<DownloadDto>,
    /// Plain text: a failed download, a failed check, a refused load.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_bytes: Option<u64>,
    pub ram_total: u64,
    /// This build can run the model on a graphics card.
    pub gpu_supported: bool,
    /// The operating system this app runs on (the web view's wording and path handling follow it).
    pub os: OsDto,
    /// Where the model runs, while it is loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceDto>,
    /// Video memory the model takes, while it is loaded on a graphics card.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f64>,
    /// Set while folders are being labelled: the scan they belong to and how far the job has come.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labelling: Option<LabellingDto>,
    /// A problem on the graphics card that waits for the user's answer (`answer_gpu_problem`): the model ran
    /// on the card and it crashed, hung or reported an error, and the job finished on the processor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_problem: Option<GpuProblemDto>,
    /// The last problem on the graphics card in the log, answered or not (until the log is cleared).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_gpu_problem: Option<GpuProblemDto>,
}

/// What went wrong on the graphics card. Sent only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuProblemWhatDto {
    Crash,
    Hang,
    Error,
}

/// A problem on the graphics card. Sent only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuProblemDto {
    /// Names the problem when it is answered (Unix milliseconds, unique).
    pub id: u64,
    /// When it happened (Unix milliseconds).
    pub at: u64,
    /// The driver's description of the card, when one was chosen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    pub what: GpuProblemWhatDto,
}

impl From<&Problem> for GpuProblemDto {
    fn from(problem: &Problem) -> Self {
        Self {
            id: problem.id,
            at: problem.at,
            card: problem.card.clone(),
            what: match problem.what {
                What::Crash => GpuProblemWhatDto::Crash,
                What::Hang => GpuProblemWhatDto::Hang,
                What::Error => GpuProblemWhatDto::Error,
            },
        }
    }
}

/// What the user chose to do about a graphics card problem. The only other advisor type the web view sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuChoiceDto {
    /// Use the processor from now on (the web view also saves the switch and configures the advisor).
    UseCpu,
    /// Try the graphics card again at the next load.
    TryAgain,
}

impl From<GpuChoiceDto> for Choice {
    fn from(choice: GpuChoiceDto) -> Self {
        match choice {
            GpuChoiceDto::UseCpu => Self::UseCpu,
            GpuChoiceDto::TryAgain => Self::TryAgain,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabellingDto {
    pub scan: ScanId,
    pub done: u64,
    pub total: u64,
}

impl From<ModelState> for ModelStateDto {
    fn from(state: ModelState) -> Self {
        match state {
            ModelState::Unavailable => Self::Unavailable,
            ModelState::NotDownloaded => Self::NotDownloaded,
            ModelState::Downloading => Self::Downloading,
            ModelState::OnDisk => Self::OnDisk,
            ModelState::Loading => Self::Loading,
            ModelState::Loaded => Self::Loaded,
        }
    }
}

impl From<UnloadAfterDto> for UnloadAfter {
    fn from(unload_after: UnloadAfterDto) -> Self {
        match unload_after {
            UnloadAfterDto::OneMinute => Self::OneMinute,
            UnloadAfterDto::FiveMinutes => Self::FiveMinutes,
            UnloadAfterDto::WindowClose => Self::WindowClose,
        }
    }
}

impl From<UnloadAfter> for UnloadAfterDto {
    fn from(unload_after: UnloadAfter) -> Self {
        match unload_after {
            UnloadAfter::OneMinute => Self::OneMinute,
            UnloadAfter::FiveMinutes => Self::FiveMinutes,
            UnloadAfter::WindowClose => Self::WindowClose,
        }
    }
}

impl From<AdvisorOptionsDto> for Options {
    fn from(options: AdvisorOptionsDto) -> Self {
        Self {
            enabled: options.enabled,
            unload_after: options.unload_after.into(),
            use_gpu: options.use_gpu,
        }
    }
}

impl AdvisorOptionsDto {
    /// The options of the model, and the question log's switch that the model does not know of.
    fn of(options: Options, log_questions: bool) -> Self {
        Self {
            enabled: options.enabled,
            unload_after: options.unload_after.into(),
            log_questions,
            use_gpu: options.use_gpu,
        }
    }
}

fn model_info(spec: &ModelSpec) -> ModelInfoDto {
    let host = spec
        .url
        .strip_prefix("https://")
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .unwrap_or_default();

    ModelInfoDto {
        name: spec.name.to_owned(),
        format: spec.format.to_owned(),
        bytes: spec.bytes,
        host: host.to_owned(),
    }
}

impl From<WorkerError> for CommandError {
    fn from(error: WorkerError) -> Self {
        CommandError::invalid(error.to_string())
    }
}

// ---- The service ----

/// Receives every change of the status.
pub type Sink = Arc<dyn Fn(AdvisorStatusDto) + Send + Sync>;

/// The labelling job that is running or has just ended and is storing its labels.
#[derive(Debug, Clone, Copy)]
struct Claim {
    scan: ScanId,
    /// Folders the job was given.
    total: usize,
    /// Folders dealt with at the last progress report. The worker clears its own progress when the job
    /// ends, before the labels are stored; the status goes on showing this until they are.
    done: usize,
}

/// What a status is made into a DTO from: the model's description and the labelling job. Shared with the
/// worker's callback.
struct View {
    model: ModelSpec,
    /// What went wrong on the graphics card, and the user's answer to it.
    gpu_log: Arc<GpuLog>,
    claim: Mutex<Option<Claim>>,
    /// The question log's switch (`AdvisorOptionsDto::log_questions`). Kept here, with the status, so
    /// every status the web view gets shows it.
    log_questions: AtomicBool,
}

impl View {
    fn new(model: ModelSpec, gpu_log: Arc<GpuLog>) -> Arc<Self> {
        Arc::new(Self {
            model,
            gpu_log,
            claim: Mutex::new(None),
            log_questions: AtomicBool::new(true),
        })
    }

    /// The status as the web view gets it. `labelling` stays set from the moment a job is claimed until
    /// its labels are stored, so "the job ended" is never seen before the labels can be fetched.
    fn dto(&self, status: &Status) -> AdvisorStatusDto {
        let labelling = lock(&self.claim).as_mut().map(|claim| {
            if let Some(progress) = status.labelling {
                claim.done = progress.done;
            }

            LabellingDto {
                scan: claim.scan,
                done: claim.done as u64,
                total: claim.total as u64,
            }
        });

        AdvisorStatusDto {
            state: status.state.into(),
            model: model_info(&self.model),
            options: AdvisorOptionsDto::of(status.options, self.log_questions.load(Ordering::SeqCst)),
            download: status.download.map(|progress| DownloadDto {
                received: progress.received,
                total: progress.total,
            }),
            problem: status.problem.clone(),
            ram_bytes: status.ram_bytes,
            ram_total: status.ram_total,
            gpu_supported: GPU_SUPPORTED,
            os: current_os().into(),
            device: status.device.as_ref().map(DeviceDto::from),
            video_bytes: status.video_bytes,
            tokens_per_second: status.tokens_per_second,
            labelling,
            gpu_problem: self.gpu_log.pending().as_ref().map(GpuProblemDto::from),
            last_gpu_problem: self.gpu_log.last().as_ref().map(GpuProblemDto::from),
        }
    }
}

/// One kind of dialog (the download, or clearing a log): shown at most once at a time, and not again right
/// after it was declined.
struct DialogGate {
    open: AtomicBool,
    /// When the user last declined (or the dialog failed).
    declined: Mutex<Option<Instant>>,
    pause: Duration,
    /// The refusal while the dialog is open.
    open_text: &'static str,
    /// The refusal right after a decline.
    declined_text: &'static str,
}

impl DialogGate {
    /// The gate of the download dialog.
    fn new(pause: Duration) -> Arc<Self> {
        Self::named(
            pause,
            "the download is already being asked about",
            "the download was just declined, so wait a moment",
        )
    }

    /// The gate of the dialogs that clear a log (the question log and the graphics card problem log).
    fn clearing(pause: Duration) -> Arc<Self> {
        Self::named(
            pause,
            "clearing a log is already being asked about",
            "clearing a log was just declined, so wait a moment",
        )
    }

    fn named(pause: Duration, open_text: &'static str, declined_text: &'static str) -> Arc<Self> {
        Arc::new(Self {
            open: AtomicBool::new(false),
            declined: Mutex::new(None),
            pause,
            open_text,
            declined_text,
        })
    }
}

/// The right to show the download dialog. Taken before the dialog, given back when this is dropped,
/// however the dialog ends.
struct DialogSlot {
    gate: Arc<DialogGate>,
}

impl DialogSlot {
    /// Refused (`invalid`) while a dialog is open or was just declined.
    fn take(gate: &Arc<DialogGate>) -> CommandResult<Self> {
        Self::take_between(gate, || {})
    }

    /// [`DialogSlot::take`], running `between` after the slot was won and before the pause is checked.
    ///
    /// The slot is won first. A dialog that is declined has noted it before it gives its slot back, so
    /// a request that wins the slot afterwards always sees the decline; checked the other way round, a
    /// request could pass the pause check just before a decline and win the slot just after it.
    fn take_between(gate: &Arc<DialogGate>, between: impl FnOnce()) -> CommandResult<Self> {
        if gate
            .open
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(CommandError::invalid(gate.open_text));
        }

        // From here on the slot gives itself back when it is dropped, on a refusal too.
        let slot = Self {
            gate: Arc::clone(gate),
        };
        between();

        if lock(&gate.declined).is_some_and(|at| at.elapsed() < gate.pause) {
            return Err(CommandError::invalid(gate.declined_text));
        }

        Ok(slot)
    }

    /// The user said no. Noted before the slot is given back, so no request slips in between.
    fn declined(&self) {
        *lock(&self.gate.declined) = Some(Instant::now());
    }
}

impl Drop for DialogSlot {
    fn drop(&mut self) {
        self.gate.open.store(false, Ordering::SeqCst);
    }
}

/// The advisor as the commands see it.
pub struct AdvisorService {
    view: Arc<View>,
    /// `None` where the runtime is not built in: everything but the options is refused.
    worker: Option<AdvisorHandle>,
    watcher: Arc<Mutex<Option<Sink>>>,
    /// The options kept while there is no worker to keep them.
    options: Mutex<Options>,
    dialog: Arc<DialogGate>,
    /// The dialogs that clear a log: one at a time, none for a moment after a no.
    clearing: Arc<DialogGate>,
    /// A free question is being answered (see [`AdvisorService::begin_ask`]).
    asking: AtomicBool,
}

/// How the model's reading of a question ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRead {
    Read(ScanQuestion),
    TimedOut,
    /// The model wrote something that is no query.
    Unreadable,
}

impl ModelRead {
    /// The query to go on with: what the model wrote, or the general question when it wrote nothing usable.
    pub fn query(&self) -> ScanQuestion {
        match self {
            Self::Read(query) => query.clone(),
            Self::TimedOut | Self::Unreadable => ScanQuestion::general(),
        }
    }
}

/// The right to answer a free question, held from the model's reading of it to the written answer. The
/// worker is idle once the model has read the question, so without this a second question, a labelling run
/// or a question about a folder could take the model while the first answer is still being made.
pub struct AskGuard {
    service: Arc<AdvisorService>,
}

impl Drop for AskGuard {
    fn drop(&mut self) {
        self.service.asking.store(false, Ordering::SeqCst);
    }
}

impl AdvisorService {
    /// The real advisor: the model in the app's data folder, the llama.cpp runtime, and the network
    /// client for the one download. `unavailable` when the build has no runtime, the OS cannot run it
    /// safely, or there is no home folder to keep the model in.
    ///
    /// The model runs in a helper process: this very binary started again with `--advisor-runtime` (see
    /// [`crate::advisor_runtime`]). The graphics card problem log is read here: a problem the user has not
    /// answered blocks the card until they do.
    pub fn production() -> Self {
        #[cfg(feature = "advisor")]
        {
            if platter_platform::model_runtime_supported()
                && let Some(data) = platter_platform::data_dir()
                && let Ok(spawner) = RealSpawner::current_exe()
            {
                let config = WorkerConfig::new(MODEL, data.join("models"));
                let engine = Box::new(ProcessEngine::new(spawner));
                let fetch = Arc::new(platter_ai::UreqFetch::new());

                if let Ok(service) = Self::spawn_logged(config, engine, fetch, Arc::new(GpuLog::production()))
                {
                    return service;
                }
            }
        }

        Self::unavailable()
    }

    /// What most tests of the app use: the real worker on the real models folder, with no model runtime (no
    /// helper process is ever started) and no graphics card problem log (the user's own is never read).
    #[cfg(test)]
    pub fn for_test() -> Self {
        let config = match platter_platform::data_dir() {
            Some(data) => WorkerConfig::new(MODEL, data.join("models")),
            None => return Self::unavailable(),
        };
        let fetch = Arc::new(platter_ai::UreqFetch::new());

        Self::spawn(config, Box::new(NoRuntime), fetch).unwrap_or_else(|_| Self::unavailable())
    }

    /// An advisor that is not there.
    pub fn unavailable() -> Self {
        Self {
            view: View::new(MODEL, Arc::new(GpuLog::new(None))),
            worker: None,
            watcher: Arc::default(),
            options: Mutex::new(Options::default()),
            dialog: DialogGate::new(DECLINE_PAUSE),
            clearing: DialogGate::clearing(DECLINE_PAUSE),
            asking: AtomicBool::new(false),
        }
    }

    /// Tests shorten the pause after a declined download dialog.
    #[cfg(test)]
    fn with_decline_pause(mut self, pause: Duration) -> Self {
        self.dialog = DialogGate::new(pause);
        self.clearing = DialogGate::clearing(pause);
        self
    }

    /// An advisor on the given parts (tests give it fakes), with no graphics card problem log.
    #[cfg(test)]
    pub fn spawn(config: WorkerConfig, engine: Box<dyn Engine>, fetch: Arc<dyn Fetch>) -> io::Result<Self> {
        Self::spawn_logged(config, engine, fetch, Arc::new(GpuLog::new(None)))
    }

    /// [`spawn`](Self::spawn) with the log the graphics card problems are written to. What the log says is
    /// waiting for an answer starts with the card blocked, and every incident the worker tells is written to it.
    #[cfg(any(feature = "advisor", test))]
    pub fn spawn_logged(
        mut config: WorkerConfig,
        engine: Box<dyn Engine>,
        fetch: Arc<dyn Fetch>,
        gpu_log: Arc<GpuLog>,
    ) -> io::Result<Self> {
        let view = View::new(config.spec, Arc::clone(&gpu_log));
        let watcher: Arc<Mutex<Option<Sink>>> = Arc::default();

        config.gpu_blocked = gpu_log.pending().is_some();
        let recorder = Arc::clone(&gpu_log);
        config.on_gpu_incident = Some(Arc::new(move |incident| recorder.record(incident)));

        let pushed = (Arc::clone(&watcher), Arc::clone(&view));
        let on_status = Arc::new(move |status: &Status| {
            let sink = lock(&pushed.0).clone();
            if let Some(sink) = sink {
                sink(pushed.1.dto(status));
            }
        });
        let worker = AdvisorWorker::spawn(config, engine, fetch, on_status)?;

        Ok(Self {
            view,
            worker: Some(worker),
            watcher,
            options: Mutex::new(Options::default()),
            dialog: DialogGate::new(DECLINE_PAUSE),
            clearing: DialogGate::clearing(DECLINE_PAUSE),
            asking: AtomicBool::new(false),
        })
    }

    pub fn status(&self) -> AdvisorStatusDto {
        match &self.worker {
            Some(worker) => self.view.dto(&worker.status()),
            None => self.unavailable_status(),
        }
    }

    /// Whether "Use the local model" is on.
    pub fn enabled(&self) -> bool {
        match &self.worker {
            Some(worker) => worker.status().options.enabled,
            None => lock(&self.options).enabled,
        }
    }

    /// Whether a free question is written to the question log now (the user's switch in Settings).
    pub fn log_questions(&self) -> bool {
        self.view.log_questions.load(Ordering::SeqCst)
    }

    /// Sends the current status to `sink` at once, then every later change, instead of to the previous
    /// watcher. The first push closes the gap between `status` and `watch`: a change in between would
    /// otherwise be lost.
    pub fn watch(&self, sink: Sink) {
        let Some(worker) = &self.worker else {
            *lock(&self.watcher) = Some(Arc::clone(&sink));
            sink(self.unavailable_status());
            return;
        };

        // Installed and told the current status while the worker's state is locked, so no change can be
        // told in between (out of order). Same lock order as the worker's callback: state, then watcher.
        worker.observe(|status| {
            *lock(&self.watcher) = Some(Arc::clone(&sink));
            sink(self.view.dto(status));
        });
    }

    pub fn configure(&self, options: AdvisorOptionsDto) -> CommandResult<AdvisorStatusDto> {
        self.view
            .log_questions
            .store(options.log_questions, Ordering::SeqCst);
        let options = Options::from(options);
        let Some(worker) = &self.worker else {
            *lock(&self.options) = options;
            return Ok(self.unavailable_status());
        };

        // The settings already say to use the processor: a problem on the card that waits for an answer
        // has its answer, and the user is not asked (at start the web view sends the saved options before it
        // shows anything).
        if !options.use_gpu {
            self.view.gpu_log.answer_for_cpu_only();
        }

        Ok(self.view.dto(&worker.configure(options)?))
    }

    /// Writes the user's answer to the graphics card problem `id` (`invalid` when that is not the problem that
    /// waits, so an answer is never applied twice or to a newer problem). `tryAgain` lets the next load try the
    /// card; `useCpu` changes nothing here (the web view saves the switch and configures the advisor).
    pub fn answer_gpu_problem(&self, id: u64, choice: GpuChoiceDto) -> CommandResult<AdvisorStatusDto> {
        let worker = self.worker()?;

        self.view.gpu_log.answer(id, choice.into())?;
        if choice == GpuChoiceDto::TryAgain {
            worker.retry_gpu();
        }
        self.announce();

        Ok(self.view.dto(&worker.status()))
    }

    /// Deletes the graphics card problem log and forgets the last problem. A problem that waits for an
    /// answer still waits.
    pub fn clear_gpu_log(&self) -> CommandResult<()> {
        self.view.gpu_log.clear()?;
        self.announce();

        Ok(())
    }

    /// What the user is asked before the download starts, and the ticket of the state it was asked in
    /// (see [`start_download`](Self::start_download)). `invalid`, with the reason, when no download can
    /// start now (the model is on disk, busy, or not available), so there is nothing to ask. A partial
    /// file from an earlier try is said in the text.
    pub fn download_question(&self) -> CommandResult<(Question, StateTicket)> {
        let (status, ticket) = self.worker()?.status_with_ticket();
        if let Some(reason) = status.state.download_refusal() {
            return Err(CommandError::invalid(reason));
        }

        let model = model_info(&self.view.model);
        let size = gigabytes(self.view.model.bytes);
        let what = match status.download {
            Some(progress) if progress.received > 0 => format!(
                "continue the download of {} ({} of {size} received)",
                model.name,
                gigabytes(progress.received).trim_end_matches(" GB")
            ),
            _ => format!("download {} ({size})", model.name),
        };

        let question = Question {
            title: "Download the advisor model?".to_owned(),
            text: format!(
                "Platter will {what} from {}. This is Platter's only network call. The file is checked before it is used.",
                model.host
            ),
            accept: "Download",
        };

        Ok((question, ticket))
    }

    /// Queues the download, if the model is still in the state `ticket` (from
    /// [`download_question`](Self::download_question)) was read in: a download the user approved is never
    /// started in a state they were not asked about. Only [`download_model`] calls this, after the user
    /// has approved it.
    fn start_download(&self, ticket: StateTicket) -> CommandResult<()> {
        Ok(self.worker()?.download_if(ticket)?)
    }

    pub fn cancel_download(&self) {
        if let Some(worker) = &self.worker {
            worker.cancel_download();
        }
    }

    /// Waits until the model is loaded (or the load failed, which is in the status).
    pub fn load(&self) -> CommandResult<AdvisorStatusDto> {
        Ok(self.view.dto(&self.worker()?.load()?))
    }

    pub fn unload(&self) -> CommandResult<AdvisorStatusDto> {
        Ok(self.view.dto(&self.worker()?.unload()?))
    }

    /// For quitting: stops a download (its partial file stays) and unloads the model.
    pub fn shutdown(&self) {
        if let Some(worker) = &self.worker {
            worker.shutdown();
        }
    }

    /// Whether folders are being labelled now (or the labels of a job that just ended are being stored).
    pub fn is_labelling(&self) -> bool {
        lock(&self.view.claim).is_some()
    }

    /// Stops a running labelling job; the labels made so far are kept. Nothing happens when none runs.
    pub fn cancel_labelling(&self) {
        if let Some(worker) = &self.worker {
            worker.cancel_labelling();
        }
    }

    /// Whether a free question (`ask_scan`) is being answered: it holds the model from the reading of the
    /// question to the written answer, also while the worker is idle after the reading.
    pub fn is_asking(&self) -> bool {
        self.asking.load(Ordering::SeqCst)
    }

    /// Whether a labelling job could start now: the model is switched on and on disk (or loaded), and no
    /// other job, no free question, uses it.
    pub fn check_can_label(&self) -> CommandResult<()> {
        let status = self.worker()?.status();

        if self.is_asking() {
            return Err(busy());
        }

        if !status.options.enabled {
            return Err(CommandError::invalid(
                "the local model is switched off in the settings",
            ));
        }

        match status.state {
            ModelState::OnDisk | ModelState::Loaded => Ok(()),
            ModelState::NotDownloaded => Err(CommandError::invalid("the model is not on disk yet")),
            ModelState::Unavailable => Err(CommandError::invalid(
                "the local model is not available in this build",
            )),
            ModelState::Downloading | ModelState::Loading => Err(busy()),
        }
    }

    /// Starts a labelling job for `scan` on a thread of its own and returns when the worker has taken it
    /// (or refused it: `invalid`). When the job ends, `store` gets what it made, and the status stops
    /// naming the scan only after that.
    pub fn start_labelling(
        self: &Arc<Self>,
        scan: ScanId,
        items: Vec<(NodeId, FolderFacts)>,
        store: impl FnOnce(LabelRun) + Send + 'static,
    ) -> CommandResult<()> {
        self.worker()?;

        {
            let mut claim = lock(&self.view.claim);
            if claim.is_some() {
                return Err(CommandError::invalid("folders are already being labelled"));
            }
            *claim = Some(Claim {
                scan,
                total: items.len(),
                done: 0,
            });
        }

        // A free question that began after the caller looked: it keeps the model until it is answered.
        // (`begin_ask` looks at the claim after it has set its flag, so one of the two always sees the other.)
        if self.is_asking() {
            *lock(&self.view.claim) = None;
            self.announce();
            return Err(busy());
        }

        let (taken, heard) = mpsc::channel();
        let service = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("platter-labelling".to_owned())
            .spawn(move || service.run_labelling(items, store, &taken));
        if let Err(error) = spawned {
            *lock(&self.view.claim) = None;
            self.announce();
            return Err(error.into());
        }

        // A refusal comes at once; a job that is taken stays silent until it ends.
        match heard.recv_timeout(TAKEN_WITHIN) {
            Ok(Err(error)) => Err(error.into()),
            _ => Ok(()),
        }
    }

    /// The labelling thread: runs the job, stores its labels, then lets go of the claim.
    fn run_labelling(
        &self,
        items: Vec<(NodeId, FolderFacts)>,
        store: impl FnOnce(LabelRun),
        taken: &mpsc::Sender<Result<(), WorkerError>>,
    ) {
        let outcome = match &self.worker {
            Some(worker) => worker.label(items),
            None => Err(WorkerError::Stopped),
        };

        match outcome {
            Ok(run) => {
                store(run);
                *lock(&self.view.claim) = None;
                // The worker's last report was made while the job was still claimed.
                self.announce();
                let _ = taken.send(Ok(()));
            }
            Err(error) => {
                *lock(&self.view.claim) = None;
                // A status pushed while the job was claimed named the scan: take it back.
                self.announce();
                let _ = taken.send(Err(error));
            }
        }
    }

    /// Tells the watcher the current status, in order with the worker's own changes.
    fn announce(&self) {
        let Some(worker) = &self.worker else {
            return;
        };

        worker.observe(|status| {
            let sink = lock(&self.watcher).clone();
            if let Some(sink) = sink {
                sink(self.view.dto(status));
            }
        });
    }

    /// Whether a free question could start now, in the order the contract names the refusals: folders are
    /// being labelled, another model job (or another free question) runs, the model is switched off, is not
    /// on disk, or is not available in this build (all `invalid`).
    pub fn check_can_ask(&self) -> CommandResult<()> {
        if self.is_labelling() {
            return Err(CommandError::invalid(
                "folders are being labelled, so ask again when that is done",
            ));
        }

        let worker = self.worker()?;
        let status = worker.status();
        let downloading = matches!(status.state, ModelState::Downloading | ModelState::Loading);
        if downloading || worker.job_running() || self.is_asking() {
            return Err(busy());
        }

        self.check_can_label()
    }

    /// The right to answer one free question: the question is refused as [`check_can_ask`] says, and while
    /// the guard is held (through the reading and the writing of the answer) every other free question is
    /// refused as busy.
    ///
    /// [`check_can_ask`]: Self::check_can_ask
    pub fn begin_ask(self: &Arc<Self>) -> CommandResult<AskGuard> {
        self.check_can_ask()?;

        // Two questions can pass the check together; only the one that sets the flag goes on.
        if self.asking.swap(true, Ordering::SeqCst) {
            return Err(busy());
        }

        // Labelling may have been claimed since `check_can_ask` looked (see `start_labelling`).
        if self.is_labelling() {
            self.asking.store(false, Ordering::SeqCst);
            return Err(CommandError::invalid(
                "folders are being labelled, so ask again when that is done",
            ));
        }

        Ok(AskGuard {
            service: Arc::clone(self),
        })
    }

    /// The model reads a free question (cleaned) into a query. A model that runs
    /// out of time or writes something unreadable gives the general question instead. Everything else is
    /// an error (`invalid`): a refusal (a job began in between, the model was switched off), a model that
    /// was cancelled meanwhile (switched off, the app closing) and one that could not be loaded or failed.
    #[cfg(test)]
    pub fn interpret(&self, question: &str) -> CommandResult<ScanQuestion> {
        Ok(self.interpret_traced(question)?.query())
    }

    /// [`interpret`](Self::interpret) that says how the model's reading ended (for the question log).
    pub fn interpret_traced(&self, question: &str) -> CommandResult<ModelRead> {
        match self.worker()?.interpret(question) {
            Ok(query) => Ok(ModelRead::Read(query)),
            Err(WorkerError::TimedOut) => Ok(ModelRead::TimedOut),
            Err(WorkerError::Unreadable) => Ok(ModelRead::Unreadable),
            Err(WorkerError::Cancelled) => Err(CommandError::invalid(
                "the model was stopped before it read the question",
            )),
            Err(error) => Err(error.into()),
        }
    }

    fn worker(&self) -> CommandResult<&AdvisorHandle> {
        self.worker
            .as_ref()
            .ok_or_else(|| CommandError::invalid("the local model is not available in this build"))
    }

    fn unavailable_status(&self) -> AdvisorStatusDto {
        let options = *lock(&self.options);
        AdvisorStatusDto {
            state: ModelStateDto::Unavailable,
            model: model_info(&self.view.model),
            options: AdvisorOptionsDto::of(options, self.log_questions()),
            download: None,
            problem: None,
            ram_bytes: None,
            ram_total: platter_platform::total_memory_bytes().unwrap_or(0),
            gpu_supported: false,
            os: current_os().into(),
            device: None,
            video_bytes: None,
            tokens_per_second: None,
            labelling: None,
            gpu_problem: None,
            last_gpu_problem: None,
        }
    }
}

/// Bytes as the dialog says them: decimal gigabytes with one digit (`1.3 GB`).
fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// The refusal while the model is used by something else.
fn busy() -> CommandError {
    CommandError::invalid("the model is busy, so wait for it to finish")
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---- Commands ----

/// The state of the model now.
#[tauri::command]
pub fn advisor_status(state: State<'_, AppState>) -> CommandResult<AdvisorStatusDto> {
    Ok(state.advisor.status())
}

/// Sends the current status to `on_event` at once, then every change of the advisor's status. The one
/// channel is replaced by a new call.
#[tauri::command]
pub fn watch_advisor(state: State<'_, AppState>, on_event: Channel<AdvisorStatusDto>) -> CommandResult<()> {
    state.advisor.watch(Arc::new(move |status| {
        // The webview may have gone away; there is nobody to tell.
        let _ = on_event.send(status);
    }));

    Ok(())
}

/// Sets the options from Settings (at start and when they change). Switching the model off unloads it,
/// which may take a moment, so this runs off the main thread.
#[tauri::command]
pub async fn configure_advisor(
    state: State<'_, AppState>,
    options: AdvisorOptionsDto,
) -> CommandResult<AdvisorStatusDto> {
    let advisor = Arc::clone(&state.advisor);

    tauri::async_runtime::spawn_blocking(move || advisor.configure(options)).await?
}

/// Starts the download of the model after the user approved it in the backend's own dialog; progress
/// arrives through `watch_advisor`. A declined (or failed) dialog is the error `cancelled` and nothing
/// changes. The dialog is shown from the main thread, so this runs off it.
#[tauri::command]
pub async fn start_model_download(state: State<'_, AppState>) -> CommandResult<()> {
    download_model(&state).await
}

/// Stops the download; the part that arrived is kept for the next try.
#[tauri::command]
pub fn cancel_model_download(state: State<'_, AppState>) -> CommandResult<()> {
    state.advisor.cancel_download();

    Ok(())
}

/// Checks the model file and loads it into memory.
#[tauri::command]
pub async fn load_model(state: State<'_, AppState>) -> CommandResult<AdvisorStatusDto> {
    let advisor = Arc::clone(&state.advisor);

    tauri::async_runtime::spawn_blocking(move || advisor.load()).await?
}

/// Frees the model's memory.
#[tauri::command]
pub async fn unload_model(state: State<'_, AppState>) -> CommandResult<AdvisorStatusDto> {
    let advisor = Arc::clone(&state.advisor);

    tauri::async_runtime::spawn_blocking(move || advisor.unload()).await?
}

// ---- Labelling, asking and finding folders (A2) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelVerdictDto {
    Review,
    Keep,
}

impl From<ModelVerdict> for ModelVerdictDto {
    fn from(verdict: ModelVerdict) -> Self {
        match verdict {
            ModelVerdict::Review => Self::Review,
            ModelVerdict::Keep => Self::Keep,
        }
    }
}

/// The verdict as the suggestions name it (the model's is `review` or `keep`, never `safe`).
fn verdict_of(verdict: ModelVerdict) -> Verdict {
    match verdict {
        ModelVerdict::Review => Verdict::Review,
        ModelVerdict::Keep => Verdict::Keep,
    }
}

/// One folder the model labelled. `decided` says the user has a rule on exactly this folder (`keep` or
/// `safe`), so the label needs no more attention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelDto {
    pub node: NodeId,
    pub path: String,
    pub allocated: u64,
    pub verdict: ModelVerdictDto,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided: Option<UserVerdictDto>,
}

/// One step of "How a verdict is made".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepDto {
    pub folders: u64,
    pub allocated: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<StepExampleDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepExampleDto {
    pub path: String,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StoppedDto {
    Cancelled,
    TimeBudget,
}

impl From<Stopped> for StoppedDto {
    fn from(stopped: Stopped) -> Self {
        match stopped {
            Stopped::Cancelled => Self::Cancelled,
            Stopped::TimeBudget => Self::TimeBudget,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorLabelsDto {
    pub scan: ScanId,
    /// Folders the model labelled in this scan.
    pub labelled: u64,
    /// Folders a labelling run would take now.
    pub candidates: u64,
    /// Largest first.
    pub labels: Vec<LabelDto>,
    pub model_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<StoppedDto>,
    pub rules: StepDto,
    pub model: StepDto,
    pub guard: StepDto,
}

/// Who decided an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AnswerSourceDto {
    Guard,
    Rule,
    /// One of the user's own rules.
    User,
    /// The verdict is the label the local model made earlier in this scan for a folder no rule covers. The
    /// answer's text never carries what the model said.
    Model,
    /// No rule covers the folder and the model has not labelled it: the verdict is `review`, "your call".
    Unruled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorAnswerDto {
    pub node: NodeId,
    pub path: String,
    pub verdict: Verdict,
    /// What Platter says: the rule's own description, why the guard leaves the folder alone, or that no rule
    /// covers it (and how the model marked it, when it did). Written by Platter, never by the model.
    pub reason: String,
    pub source: AnswerSourceDto,
    /// A short plain line: the rule's title, the folder's size and the share older than a year.
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderMatchDto {
    pub node: NodeId,
    pub path: String,
    pub allocated: u64,
}

/// A folder or a file of a scan whose name (or path) fits what the user typed. `kind` is `dir` or `file`
/// (the wire names of [`NodeKind`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryMatchDto {
    pub node: NodeId,
    pub path: String,
    pub allocated: u64,
    pub kind: NodeKind,
}

fn clamp(value: u64) -> u64 {
    value.min(MAX_SAFE)
}

/// What the rules, the guard and the explainer say about one finished scan, worked out once so a command
/// can ask several questions of it. Walks the whole tree: use it off the async runtime.
pub struct Review<'a> {
    tree: &'a Tree,
    env: Environment,
    /// The built-in rules and the user's own, with each keep that is a link also kept where it leads,
    /// as the suggestions and the plans see them.
    rules: RuleSet,
    /// What the user's own rules say, as they said it.
    user: Vec<UserRule>,
    explainer: Explainer<'a>,
}

impl<'a> Review<'a> {
    /// `user` is the user's own rules (none while their file is broken).
    pub fn new(tree: &'a Tree, platform: &Platform, user: Vec<UserRule>) -> Self {
        Self::with_probe(tree, platform, &platform.entry_probe, user)
    }

    /// [`Review::new`] with the disk looked at through `probe`.
    pub fn with_probe(
        tree: &'a Tree,
        platform: &Platform,
        probe: &impl EntryProbe,
        user: Vec<UserRule>,
    ) -> Self {
        Self::for_environment(tree, platform.environment_for(tree.root_path()), probe, user)
    }

    /// [`Review::with_probe`] for an environment that is already known.
    pub fn for_environment(
        tree: &'a Tree,
        env: Environment,
        probe: &impl EntryProbe,
        user: Vec<UserRule>,
    ) -> Self {
        // A folder kept through a link is kept where it really is too, as the suggestions see it, so the
        // labels listed, the folders sent to the model and the answers agree with the plan.
        let rules = fenced(
            with_real_keeps(
                RuleSet::builtin().with_user(user.clone()),
                env.home.as_deref(),
                probe,
            ),
            &env,
        );
        let explainer = Explainer::new(tree, &env, &rules);

        Self {
            tree,
            env,
            rules,
            user,
            explainer,
        }
    }

    pub fn env(&self) -> &Environment {
        &self.env
    }

    pub fn rules(&self) -> &RuleSet {
        &self.rules
    }

    fn home(&self) -> Option<&Path> {
        self.env.home.as_deref()
    }

    /// What the user's own rules say about exactly this folder; `keep` when they say both.
    fn decided(&self, node: NodeId) -> Option<UserVerdictDto> {
        let folder = node_path(self.tree, node);
        let said = |verdict: UserVerdict| {
            self.user
                .iter()
                .any(|rule| rule.verdict == verdict && rule.folder == folder)
        };

        if said(UserVerdict::Keep) {
            Some(UserVerdictDto::Keep)
        } else if said(UserVerdict::Safe) {
            Some(UserVerdictDto::Safe)
        } else {
            None
        }
    }

    /// The folder's path for people to read: the home directory is `~`.
    fn display(&self, node: NodeId) -> String {
        let path = self.tree.path_of(node);

        match self.home().and_then(|home| path.strip_prefix(home).ok()) {
            Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
            Some(rest) => format!("~/{}", rest.to_string_lossy()),
            None => path.to_string_lossy().into_owned(),
        }
    }

    /// What the model is told about a folder, with the rules that matched nearby.
    fn facts(&self, node: NodeId) -> FolderFacts {
        let context = RuleContext {
            inside: self
                .explainer
                .inside(node)
                .into_iter()
                .map(|(rule, bytes)| (rule.to_owned(), bytes))
                .collect(),
            beside: self
                .explainer
                .beside(node)
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };

        facts(self.tree, self.home(), node, context)
    }

    /// The folders a labelling run asks the model about, each with its facts.
    fn label_items(&self) -> Vec<(NodeId, FolderFacts)> {
        label_candidates(self.tree, &self.env, &self.rules, CandidateLimits::DEFAULT)
            .into_iter()
            .map(|node| (node, self.facts(node)))
            .collect()
    }

    /// Why the guard leaves a folder alone, when it does: the scan root, a scan inside a kept folder, a
    /// folder Platter could not read, a protected place (or one inside it, or one around it).
    fn guard(&self, node: NodeId) -> Option<String> {
        if node == self.tree.root() {
            return Some(
                "This is the folder that was scanned, so Platter never suggests cleaning it.".to_owned(),
            );
        }
        if scan_is_kept(self.tree, &self.env, &self.rules) {
            return Some(
                "The scan starts inside a folder you keep, so nothing in it is suggested.".to_owned(),
            );
        }
        if self.tree.is_unreadable(node) {
            return Some("Platter could not read this folder, so it cannot tell what is in it.".to_owned());
        }

        let Some(protected) = Protected::for_environment(&self.env) else {
            return Some(
                "Platter does not know your home folder, so it cannot tell which places are protected."
                    .to_owned(),
            );
        };
        let place = protected.refuses(&self.tree.path_bytes(node))?;

        Some(if relation(&self.tree.path_of(node), &place) == Relation::Holds {
            format!(
                "This folder holds a protected place ({place}), so it cannot be cleaned as a whole. Look inside it."
            )
        } else {
            format!("This is a protected place, or inside one ({place}). Platter does not clean it.")
        })
    }

    /// The labels the `advisor` suggestion lists, and so the ones `plan_labelled` may be given: `review`
    /// labels of readable, non-empty folders below the scan root that no rule covers or holds a match of,
    /// and that do not lie inside another listed label. None for a scan that starts inside a kept folder.
    /// This follows `platter_rules::evaluate_with`, so a plan never holds more than the list shows.
    pub fn listed_labels(&self, labels: &[ModelLabel]) -> Vec<ModelLabel> {
        let tree = self.tree;
        if scan_is_kept(tree, &self.env, &self.rules) {
            return Vec::new();
        }

        let listable: Vec<&ModelLabel> = labels
            .iter()
            .filter(|label| label.verdict == ModelVerdict::Review)
            .filter(|label| {
                let node = label.node;

                tree.contains(node)
                    && tree.kind(node) == NodeKind::Dir
                    && tree.parent(node).is_some()
                    && tree.allocated(node) > 0
                    && !tree.is_unreadable(node)
                    && !self.explainer.touches(node)
            })
            .collect();
        let nodes: HashSet<NodeId> = listable.iter().map(|label| label.node).collect();

        listable
            .into_iter()
            .filter(|label| {
                let ancestry = tree.ancestry(label.node);
                !ancestry[..ancestry.len() - 1]
                    .iter()
                    .any(|above| nodes.contains(above))
            })
            .cloned()
            .collect()
    }

    /// The folders the guard turned away while the candidates were searched, as the "guard" step counts
    /// them: the outermost folder of at least the candidate size inside a protected place (not entered),
    /// and every unreadable folder met on the way. Largest first.
    fn refused_while_searching(&self) -> Vec<NodeId> {
        let tree = self.tree;
        let limits = CandidateLimits::DEFAULT;
        let mut refused = Vec::new();

        if scan_is_kept(tree, &self.env, &self.rules) {
            return refused;
        }
        let Some(protected) = Protected::for_environment(&self.env) else {
            return refused;
        };

        let is_dir = |node: NodeId| tree.kind(node) == NodeKind::Dir;
        let mut pending = vec![tree.root()];

        while let Some(node) = pending.pop() {
            if !is_dir(node)
                || tree.allocated(node) < limits.min_bytes
                || tree.is_unreadable(node)
                || self.explainer.covering(node).is_some()
            {
                continue;
            }

            let root = node == tree.root();
            let place = if root {
                None
            } else {
                protected.refuses(&tree.path_bytes(node))
            };

            // A candidate is not entered; neither is a folder inside a protected place.
            let candidate =
                !root && place.is_none() && !self.explainer.holds(node) && !tree.has_unreadable(node);
            if candidate {
                continue;
            }
            if place.is_some_and(|place| relation(&tree.path_of(node), &place) == Relation::Inside) {
                refused.push(node);
                continue;
            }

            for child in tree.children(node).filter(|&child| is_dir(child)) {
                if tree.is_unreadable(child) {
                    refused.push(child);
                }
                pending.push(child);
            }
        }

        refused.sort_unstable_by_key(|&node| (std::cmp::Reverse(tree.allocated(node)), node));
        refused
    }
}

/// How a folder stands to the protected place the guard named for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relation {
    /// The folder is the place.
    Exact,
    /// The place lies inside the folder.
    Holds,
    /// The folder lies inside the place.
    Inside,
}

fn relation(path: &Path, place: &str) -> Relation {
    let place = Path::new(place);

    if place == path {
        Relation::Exact
    } else if place.starts_with(path) {
        Relation::Holds
    } else {
        Relation::Inside
    }
}

/// Labels the folders of a finished scan that no rule covers; progress arrives through `watch_advisor`.
/// A few seconds of work per folder, so the job runs on a thread of its own.
#[tauri::command]
pub async fn label_folders(state: State<'_, AppState>, scan: ScanId) -> CommandResult<()> {
    label_scan(&state, scan).await
}

/// Stops the labelling job; what it labelled so far is kept. Nothing happens when none runs.
#[tauri::command]
pub fn cancel_labelling(state: State<'_, AppState>) -> CommandResult<()> {
    state.advisor.cancel_labelling();

    Ok(())
}

/// The labels of a scan and the counts for "How a verdict is made".
#[tauri::command]
pub async fn get_advisor_labels(state: State<'_, AppState>, scan: ScanId) -> CommandResult<AdvisorLabelsDto> {
    advisor_labels(&state, scan).await
}

/// What Platter says about one folder, from its rules alone (no model call): the guard, a rule, or that no
/// rule covers it. `units` is how the size is written; decimal when the web view leaves it out.
#[tauri::command]
pub async fn ask_advisor(
    state: State<'_, AppState>,
    scan: ScanId,
    node: NodeId,
    units: Option<SizeUnitsDto>,
) -> CommandResult<AdvisorAnswerDto> {
    ask_about_in(&state, scan, node, units.unwrap_or(SizeUnitsDto::Gb)).await
}

/// Folders of a scan whose name (or exact path) matches what the user typed, for the folder pickers.
#[tauri::command]
pub async fn find_folders(
    state: State<'_, AppState>,
    scan: ScanId,
    query: String,
    limit: u32,
) -> CommandResult<Vec<FolderMatchDto>> {
    find(&state, scan, query, limit).await
}

/// Folders and files of a scan whose name (or exact path) matches what the user typed, for the Ask card's
/// autocomplete.
#[tauri::command]
pub async fn find_entries(
    state: State<'_, AppState>,
    scan: ScanId,
    query: String,
    limit: u32,
) -> CommandResult<Vec<EntryMatchDto>> {
    find_entries_in(&state, scan, query, limit).await
}

/// Asks the user, in the backend's dialog, then starts the download. The web view's own consent screen
/// is not enough: a compromised page could call the command without it, so every start is confirmed
/// here. Where no download can start there is nothing to ask and the refusal is the answer (`invalid`);
/// nothing is ever started without the dialog being approved, and only when the model is still in the
/// state the dialog was about. One dialog is open at a time, and none for a moment after one was
/// declined.
pub async fn download_model(state: &AppState) -> CommandResult<()> {
    let slot = DialogSlot::take(&state.advisor.dialog)?;
    let (question, ticket) = state.advisor.download_question()?;

    // The slot goes with the dialog, so it is given back when the dialog has ended and not before.
    let confirm = state.confirm();
    let asked = tauri::async_runtime::spawn_blocking(move || {
        let approved = catch_unwind(AssertUnwindSafe(|| confirm.confirm(&question))).unwrap_or(false);
        if !approved {
            slot.declined();
        }

        (approved, slot)
    })
    .await;

    let Ok((true, _slot)) = asked else {
        return Err(CommandError::new(
            ApiErrorCode::Cancelled,
            "the download was not confirmed",
        ));
    };

    state.advisor.start_download(ticket)
}

pub async fn label_scan(state: &AppState, scan: ScanId) -> CommandResult<()> {
    if state.scans.is_running(scan) {
        return Err(CommandError::invalid("the scan is still running"));
    }
    let tree = state.scans.tree(scan)?;
    if state.scans.is_outdated(scan) {
        return Err(CommandError::invalid(
            "the disk changed since this scan, so rescan first",
        ));
    }
    // With rules that cannot be read nothing says which folders the user keeps, so no folder is sent to
    // the model.
    let user = user_rules::current(&state.user_rules).await?;
    user.usable()?;
    state.advisor.check_can_label()?;
    if state.advisor.is_labelling() {
        return Err(CommandError::invalid("folders are already being labelled"));
    }

    let platform = Arc::clone(&state.platform);
    let items =
        tauri::async_runtime::spawn_blocking(move || Review::new(&tree, &platform, user.rules).label_items())
            .await?;

    let scans = Arc::clone(&state.scans);
    if items.is_empty() {
        // Nothing to ask about: the run is empty, and replaces the earlier one.
        scans.set_labels(scan, LabelSet::default());
        return Ok(());
    }

    let advisor = Arc::clone(&state.advisor);
    tauri::async_runtime::spawn_blocking(move || {
        advisor.start_labelling(scan, items, move |run| {
            scans.set_labels(
                scan,
                LabelSet {
                    labels: run.labels,
                    model_ms: run.ms,
                    stopped: run.stopped,
                },
            );
        })
    })
    .await?
}

pub async fn advisor_labels(state: &AppState, scan: ScanId) -> CommandResult<AdvisorLabelsDto> {
    let tree = state.scans.tree(scan)?;
    let set = state.scans.labels(scan)?;
    let enabled = state.advisor.enabled();
    let scans = Arc::clone(&state.scans);
    let platform = Arc::clone(&state.platform);
    let user = user_rules::current(&state.user_rules).await?;

    tauri::async_runtime::spawn_blocking(move || {
        let suggestions = suggestions::suggestions(&scans, &platform, scan, enabled, &user, Units::Decimal)?;
        let review = Review::new(&tree, &platform, user.rules);

        Ok(labels_dto(scan, &review, &suggestions, set.as_deref()))
    })
    .await?
}

fn labels_dto(
    scan: ScanId,
    review: &Review,
    suggestions: &SuggestionsDto,
    set: Option<&LabelSet>,
) -> AdvisorLabelsDto {
    let tree = review.tree;

    let mut labelled: Vec<&ModelLabel> = set
        .into_iter()
        .flat_map(|set| &set.labels)
        .filter(|label| tree.contains(label.node))
        .collect();
    labelled.sort_by_key(|label| (std::cmp::Reverse(tree.allocated(label.node)), label.node));

    let labels: Vec<LabelDto> = labelled
        .iter()
        .map(|label| LabelDto {
            node: label.node,
            path: review.display(label.node),
            allocated: clamp(tree.allocated(label.node)),
            verdict: label.verdict.into(),
            reason: label.reason.clone(),
            decided: review.decided(label.node),
        })
        .collect();

    let model = StepDto {
        folders: labels.len() as u64,
        allocated: clamp(labels.iter().map(|label| label.allocated).sum()),
        example: labelled
            .first()
            .zip(labels.first())
            .map(|(label, shown)| StepExampleDto {
                path: shown.path.clone(),
                verdict: verdict_of(label.verdict),
            }),
    };

    let refused = review.refused_while_searching();
    let guard = StepDto {
        folders: refused.len() as u64,
        allocated: clamp(refused.iter().map(|&node| tree.allocated(node)).sum()),
        example: refused.first().map(|&node| StepExampleDto {
            path: review.display(node),
            verdict: Verdict::Keep,
        }),
    };

    AdvisorLabelsDto {
        scan,
        labelled: set.map_or(0, |set| set.labelled() as u64),
        candidates: label_candidates(tree, review.env(), review.rules(), CandidateLimits::DEFAULT).len()
            as u64,
        labels,
        model_ms: set.map_or(0, |set| set.model_ms),
        stopped: set.and_then(|set| set.stopped).map(Into::into),
        rules: rules_step(suggestions),
        model,
        guard,
    }
}

/// Everything the rules matched (the built-in table and the user's own), as one step: how many folders
/// and files, how many bytes, and the largest rule as the example.
fn rules_step(suggestions: &SuggestionsDto) -> StepDto {
    let by_rules = || {
        suggestions
            .suggestions
            .iter()
            .filter(|suggestion| suggestion.source != SuggestionSource::Model)
    };

    // The largest rule; the earlier one when two are equal.
    let largest = by_rules().fold(None, |best: Option<&SuggestionDto>, suggestion| match best {
        Some(best) if best.allocated >= suggestion.allocated => Some(best),
        _ => Some(suggestion),
    });

    StepDto {
        folders: by_rules().map(|suggestion| suggestion.items).sum(),
        allocated: clamp(by_rules().map(|suggestion| suggestion.allocated).sum()),
        example: largest.map(|suggestion| StepExampleDto {
            path: suggestion.path.clone(),
            verdict: suggestion.verdict,
        }),
    }
}

const GUARD_DETAIL: &str = "Left alone by the safety guard, not decided by the model.";

/// Answers about one folder from the rules alone, so it needs neither the model nor a free model: the model
/// being off, not downloaded or busy does not matter. The refusals are an unknown or running scan
/// (`notFound`), a node that does not exist (`notFound`), a node that is not a folder (`invalid`) and a rules
/// file that cannot be read (`invalid`: with it nothing says which folders the user keeps). Labelling and free
/// questions do not hold it back. Sizes are written in decimal units; see [`ask_about_in`].
#[cfg(test)]
pub async fn ask_about(state: &AppState, scan: ScanId, node: NodeId) -> CommandResult<AdvisorAnswerDto> {
    ask_about_in(state, scan, node, SizeUnitsDto::Gb).await
}

/// [`ask_about`] with the size written in `units`.
pub async fn ask_about_in(
    state: &AppState,
    scan: ScanId,
    node: NodeId,
    units: SizeUnitsDto,
) -> CommandResult<AdvisorAnswerDto> {
    let tree = state.scans.tree(scan)?;
    if !tree.contains(node) {
        return Err(CommandError::not_found(format!("node {node:?} does not exist")));
    }
    if tree.kind(node) != NodeKind::Dir {
        return Err(CommandError::invalid("only a folder can be asked about"));
    }

    let set = state.scans.labels(scan)?;
    let platform = Arc::clone(&state.platform);
    let user = user_rules::current(&state.user_rules).await?;
    user.usable()?;
    // The rules walk the whole tree: one of the few walks that may run at once.
    let walk = state.walks.take()?;
    // The log is written when the user's switch is on as the answer is made.
    let log = Arc::clone(&state.question_log);
    let advisor = Arc::clone(&state.advisor);

    // The rules walk the tree: off the async runtime.
    let answered = tauri::async_runtime::spawn_blocking(move || {
        let _walk = walk;
        let started = Instant::now();
        let review = Review::new(&tree, &platform, user.rules);
        let answered = folder::answer(&review, set.as_deref(), node, units.into());

        log.append_if(&folder::entry(scan, &answered, started.elapsed()), || {
            advisor.log_questions()
        });

        answered.dto
    })
    .await?;

    Ok(answered)
}

/// [`find_folders`] for the state itself. Like [`find_entries_in`], the walk stops as soon as a newer call
/// has started, and at most a few walks run at once (`invalid` beyond them).
pub async fn find(
    state: &AppState,
    scan: ScanId,
    query: String,
    limit: u32,
) -> CommandResult<Vec<FolderMatchDto>> {
    let number = state.folder_searches.fetch_add(1, Ordering::SeqCst) + 1;

    find_numbered(state, scan, query, limit, number).await
}

/// [`find`] as the search that took `number`.
async fn find_numbered(
    state: &AppState,
    scan: ScanId,
    query: String,
    limit: u32,
    number: u64,
) -> CommandResult<Vec<FolderMatchDto>> {
    let tree = state.scans.tree(scan)?;
    let platform = Arc::clone(&state.platform);
    let searches = Arc::clone(&state.folder_searches);
    let limit = limit.clamp(1, MAX_FOUND) as usize;
    let walk = state.walks.take()?;

    // A walk over every folder of the tree.
    tauri::async_runtime::spawn_blocking(move || {
        let _walk = walk;
        let env = platform.environment_for(tree.root_path());
        let superseded = || searches.load(Ordering::SeqCst) != number;
        let found = search_folders_until(&tree, env.home.as_deref(), &query, limit, superseded)?;

        Ok(found
            .into_iter()
            .map(|found| FolderMatchDto {
                node: found.node,
                path: found.path,
                allocated: clamp(found.allocated),
            })
            .collect())
    })
    .await?
}

/// [`find_entries`] for the state itself. The call takes the next number of the state's searches, and its
/// walk stops as soon as a newer call has taken a higher one: the answer of a call that was typed over is
/// dropped by the web view anyway, so it is an empty list.
pub async fn find_entries_in(
    state: &AppState,
    scan: ScanId,
    query: String,
    limit: u32,
) -> CommandResult<Vec<EntryMatchDto>> {
    let number = state.entry_searches.fetch_add(1, Ordering::SeqCst) + 1;

    find_entries_numbered(state, scan, query, limit, number).await
}

/// [`find_entries_in`] as the search that took `number`.
async fn find_entries_numbered(
    state: &AppState,
    scan: ScanId,
    query: String,
    limit: u32,
    number: u64,
) -> CommandResult<Vec<EntryMatchDto>> {
    let tree = state.scans.tree(scan)?;
    let platform = Arc::clone(&state.platform);
    let searches = Arc::clone(&state.entry_searches);
    let limit = limit.clamp(1, MAX_FOUND) as usize;
    let walk = state.walks.take()?;

    // A walk over every entry of the tree.
    tauri::async_runtime::spawn_blocking(move || {
        let _walk = walk;
        let env = platform.environment_for(tree.root_path());
        let superseded = || searches.load(Ordering::SeqCst) != number;
        let found = search_entries_until(&tree, env.home.as_deref(), &query, limit, superseded)?;

        Ok(found
            .into_iter()
            .map(|found| EntryMatchDto {
                node: found.node,
                path: found.path,
                allocated: clamp(found.allocated),
                kind: found.kind,
            })
            .collect())
    })
    .await?
}

/// Deletes the question log (`advisor-questions.jsonl`), once the user said yes in the native dialog (a no
/// is `cancelled`). Succeeds when there is none, and when the settings place is not known. A link or a
/// folder at its name is left alone (`io`).
#[tauri::command]
pub async fn clear_question_log(state: State<'_, AppState>) -> CommandResult<()> {
    clear_question_log_for(&state).await
}

/// [`clear_question_log`] for the state itself.
pub async fn clear_question_log_for(state: &AppState) -> CommandResult<()> {
    confirm_clear(state, CLEAR_QUESTION_LOG).await?;
    let log = Arc::clone(&state.question_log);

    tauri::async_runtime::spawn_blocking(move || log.clear()).await?
}

/// What the user is asked before the question log is deleted.
const CLEAR_QUESTION_LOG: (&str, &str) = (
    "Clear the question log?",
    "Platter deletes its local log of the questions you asked and the answers it gave. This cannot be undone.",
);

/// What the user is asked before the graphics card problem log is deleted.
const CLEAR_GPU_LOG: (&str, &str) = (
    "Clear the graphics card problem log?",
    "Platter deletes its record of problems with the graphics card. This cannot be undone.",
);

/// Asks `(title, text)` in the native dialog, with "Clear log" to approve. A no (or no dialog) is `cancelled`.
/// One such dialog is open at a time, and none for a moment after a no (`invalid`), like the download's: a
/// page cannot stack dialogs, or bury another one among them.
async fn confirm_clear(state: &AppState, (title, text): (&str, &str)) -> CommandResult<()> {
    let slot = DialogSlot::take(&state.advisor.clearing)?;
    let question = Question {
        title: title.to_owned(),
        text: text.to_owned(),
        accept: "Clear log",
    };
    let confirm = state.confirm();
    // The slot goes with the dialog, so it is given back when the dialog has ended and not before.
    let approved = tauri::async_runtime::spawn_blocking(move || {
        let approved = catch_unwind(AssertUnwindSafe(|| confirm.confirm(&question))).unwrap_or(false);
        if !approved {
            slot.declined();
        }
        drop(slot);

        approved
    })
    .await
    .unwrap_or(false);

    if approved {
        Ok(())
    } else {
        Err(CommandError::new(
            ApiErrorCode::Cancelled,
            "clearing the log was not confirmed",
        ))
    }
}

/// What the user chose to do about the graphics card problem `id` of the status (`gpuProblem`): `useCpu` or
/// `tryAgain`. `invalid` when that problem is not the one that waits for an answer. Returns the new status.
#[tauri::command]
pub async fn answer_gpu_problem(
    state: State<'_, AppState>,
    id: u64,
    choice: GpuChoiceDto,
) -> CommandResult<AdvisorStatusDto> {
    let advisor = Arc::clone(&state.advisor);

    tauri::async_runtime::spawn_blocking(move || advisor.answer_gpu_problem(id, choice)).await?
}

/// Deletes the graphics card problem log (`advisor-gpu.log`) and with it `lastGpuProblem`, once the user
/// said yes in the native dialog (a no is `cancelled`). Succeeds when there is none. A link or a folder at its
/// name is left alone (`io`). A problem that waits for an answer still does.
#[tauri::command]
pub async fn clear_gpu_log(state: State<'_, AppState>) -> CommandResult<()> {
    confirm_clear(&state, CLEAR_GPU_LOG).await?;
    let advisor = Arc::clone(&state.advisor);

    tauri::async_runtime::spawn_blocking(move || advisor.clear_gpu_log()).await?
}

/// The advisor of the tests of the app that need a worker but no model: it can load nothing.
#[cfg(test)]
struct NoRuntime;

#[cfg(test)]
impl Engine for NoRuntime {
    fn load(
        &mut self,
        _model: &platter_ai::VerifiedModel,
        _threads: u32,
        _placement: platter_ai::Placement,
    ) -> Result<platter_ai::LoadInfo, platter_ai::AiError> {
        Err(platter_ai::AiError::Load(
            "There is no model runtime in this test.".to_owned(),
        ))
    }

    fn unload(&mut self) {}

    fn judge(
        &mut self,
        _prompt: &str,
        _grammar: &str,
        _max_tokens: u32,
        _cancel: &AtomicBool,
    ) -> Result<platter_ai::Generation, platter_ai::AiError> {
        Err(platter_ai::AiError::NotLoaded)
    }
}

mod folder;
pub mod gpu_log;
mod jsonl;
pub mod log;
pub mod question;

#[cfg(test)]
mod gpu_tests;
#[cfg(test)]
mod tests;
