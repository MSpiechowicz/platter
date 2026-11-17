//! Fakes for tests (feature `testing`): a web server and a model that need neither the network nor a
//! real model file, and (on Unix) a way to start the model's helper process as a thread.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::engine::{AiError, Engine, Generation, GpuDevice, LoadInfo, Placement, ProgressHook};
use crate::fetch::{Fetch, FetchResponse};
use crate::spec::ModelSpec;
use crate::verify::{VerifiedModel, hex};

#[cfg(any(unix, windows))]
mod spawner;
#[cfg(any(unix, windows))]
pub use spawner::{Fault, Phase, SpawnerControl, ThreadSpawner};

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A spec for `content`: a tiny stand-in for the model with the right size and checksum. Never make a
/// test file large.
pub fn tiny_spec(content: &[u8]) -> ModelSpec {
    ModelSpec {
        name: "Tiny",
        format: "test",
        file: "tiny.gguf",
        bytes: content.len() as u64,
        sha256: Box::leak(hex(&Sha256::digest(content)).into_boxed_str()),
        url: "https://models.example/tiny.gguf",
        licence: "test",
        min_free_bytes: content.len() as u64 + 512 * 1024 * 1024,
        gpu_min_free_bytes: content.len() as u64 + 256 * 1024 * 1024,
        // 4 GiB of free video memory.
        gpu_video_bytes: 4 << 30,
        context_tokens: 4096,
        folder_limit: Duration::from_secs(15),
        job_limit: Duration::from_secs(5 * 60),
        interpret_limit: Duration::from_secs(15),
    }
}

/// A door that stays shut until it is opened, and tells when somebody is waiting at it.
#[derive(Debug, Default)]
pub struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct GateState {
    open: bool,
    waiting: usize,
}

impl Gate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Lets everybody through, now and later.
    pub fn open(&self) {
        locked(&self.state).open = true;
        self.changed.notify_all();
    }

    /// Blocks until the gate is open.
    pub fn pass(&self) {
        let mut state = locked(&self.state);
        state.waiting += 1;
        self.changed.notify_all();
        while !state.open {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Waits until somebody is blocked at the gate; `false` when nobody came within `timeout`.
    pub fn wait_for_visitor(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = locked(&self.state);
        while state.waiting == 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        true
    }
}

/// One scripted answer of a [`FakeFetch`].
#[derive(Clone)]
pub struct FakeReply {
    status: u16,
    location: Option<String>,
    content_range: Option<String>,
    content_length: Option<u64>,
    body: Vec<u8>,
    chunk: usize,
    fail_after: Option<usize>,
    pause_after: Option<(usize, Arc<Gate>)>,
}

impl FakeReply {
    fn new(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            location: None,
            content_range: None,
            content_length: Some(body.len() as u64),
            body: body.to_vec(),
            chunk: 16,
            fail_after: None,
            pause_after: None,
        }
    }

    /// `200` with `body` and its length.
    pub fn ok(body: &[u8]) -> Self {
        Self::new(200, body)
    }

    /// `206` for the part of a `total`-byte file that starts at `from`; `body` is that part.
    pub fn partial(body: &[u8], from: u64, total: u64) -> Self {
        let mut reply = Self::new(206, body);
        let end = from + body.len() as u64 - 1;
        reply.content_range = Some(format!("bytes {from}-{end}/{total}"));
        reply
    }

    /// A redirect (`302`) to `location`.
    pub fn redirect(location: &str) -> Self {
        let mut reply = Self::new(302, b"");
        reply.location = Some(location.to_owned());
        reply
    }

    /// Any status with no body.
    pub fn status(status: u16) -> Self {
        Self::new(status, b"")
    }

    pub fn content_range(mut self, value: &str) -> Self {
        self.content_range = Some(value.to_owned());
        self
    }

    pub fn content_length(mut self, value: Option<u64>) -> Self {
        self.content_length = value;
        self
    }

    /// The body arrives at most `bytes` at a time.
    pub fn chunk(mut self, bytes: usize) -> Self {
        self.chunk = bytes.max(1);
        self
    }

    /// The connection breaks after `bytes` of the body.
    pub fn fail_after(mut self, bytes: usize) -> Self {
        self.fail_after = Some(bytes);
        self
    }

    /// The body stops after `bytes` until `gate` is opened.
    pub fn pause_after(mut self, bytes: usize, gate: &Arc<Gate>) -> Self {
        self.pause_after = Some((bytes, Arc::clone(gate)));
        self
    }
}

/// What was asked of a [`FakeFetch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeRequest {
    pub url: String,
    pub from: Option<u64>,
}

/// A [`Fetch`] that answers from a script, in order, and remembers every request.
#[derive(Default)]
pub struct FakeFetch {
    script: Mutex<VecDeque<FakeReply>>,
    /// What answers once the script is used up: a server for this file that honours `Range`.
    serve: Option<Vec<u8>>,
    requests: Mutex<Vec<FakeRequest>>,
    /// Every request waits here before it is answered (a server that does not answer).
    stall_at: Option<Arc<Gate>>,
}

impl FakeFetch {
    pub fn new(replies: impl IntoIterator<Item = FakeReply>) -> Self {
        Self {
            script: Mutex::new(replies.into_iter().collect()),
            serve: None,
            requests: Mutex::default(),
            stall_at: None,
        }
    }

    /// Every request is recorded and then waits at `gate` for its answer, as if the server did not
    /// answer, until the gate is opened.
    pub fn stall_at(mut self, gate: &Arc<Gate>) -> Self {
        self.stall_at = Some(Arc::clone(gate));
        self
    }

    /// After the script, answers every request like a real server holding `file`: the whole of it, or
    /// the part asked for with `Range`.
    pub fn then_serve(mut self, file: &[u8]) -> Self {
        self.serve = Some(file.to_vec());
        self
    }

    /// Every request so far, oldest first.
    pub fn requests(&self) -> Vec<FakeRequest> {
        locked(&self.requests).clone()
    }
}

impl Fetch for FakeFetch {
    fn get(&self, url: &str, from: Option<u64>) -> io::Result<FetchResponse> {
        locked(&self.requests).push(FakeRequest {
            url: url.to_owned(),
            from,
        });
        if let Some(gate) = &self.stall_at {
            gate.pass();
        }

        let scripted = locked(&self.script).pop_front();
        let reply = match (scripted, &self.serve) {
            (Some(reply), _) => reply,
            (None, Some(file)) => match from {
                Some(from) if (from as usize) < file.len() => {
                    FakeReply::partial(&file[from as usize..], from, file.len() as u64)
                }
                _ => FakeReply::ok(file),
            },
            (None, None) => return Err(io::Error::other("the fake server has no more answers")),
        };

        Ok(FetchResponse {
            status: reply.status,
            location: reply.location,
            content_range: reply.content_range,
            content_length: reply.content_length,
            body: Box::new(FakeBody {
                data: reply.body,
                at: 0,
                chunk: reply.chunk,
                fail_after: reply.fail_after,
                pause_after: reply.pause_after,
            }),
        })
    }
}

struct FakeBody {
    data: Vec<u8>,
    at: usize,
    chunk: usize,
    fail_after: Option<usize>,
    pause_after: Option<(usize, Arc<Gate>)>,
}

impl Read for FakeBody {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.fail_after.is_some_and(|limit| self.at >= limit) {
            return Err(io::Error::from(io::ErrorKind::ConnectionReset));
        }
        let paused = self
            .pause_after
            .as_ref()
            .is_some_and(|(limit, _)| self.at >= *limit);
        if paused && let Some((_, gate)) = self.pause_after.take() {
            gate.pass();
        }

        let mut room = self.chunk.min(buffer.len()).min(self.data.len() - self.at);
        if let Some(limit) = self.fail_after {
            room = room.min(limit - self.at);
        }
        if let Some((limit, _)) = &self.pause_after {
            room = room.min(limit - self.at);
        }

        buffer[..room].copy_from_slice(&self.data[self.at..self.at + room]);
        self.at += room;
        Ok(room)
    }
}

/// A graphics card for a [`FakeEngine`]: `free_mib` of free video memory, out of twice that.
pub fn card(index: usize, description: &str, free_mib: u64, integrated: bool) -> GpuDevice {
    GpuDevice {
        index,
        name: format!("Vulkan{index}"),
        description: description.to_owned(),
        total_bytes: (free_mib * 2) << 20,
        free_bytes: free_mib << 20,
        integrated,
    }
}

/// What a [`FakeEngine`] was asked to do, and what it should do next.
#[derive(Default)]
struct EngineLog {
    calls: Vec<String>,
    loaded: bool,
    fail_load: Option<String>,
    hold_load: Option<Arc<Gate>>,
    loads_started: usize,
    reply: String,
    script: VecDeque<Step>,
    prompts: Vec<String>,
    /// The graphics cards the engine reports.
    gpus: Vec<GpuDevice>,
    /// Loads on a graphics card fail with this.
    fail_gpu_load: Option<String>,
    /// Generations on a graphics card fail with this.
    fail_gpu_run: Option<String>,
    /// The next generation, anywhere, cannot take its text.
    fail_input: Option<String>,
    /// Where the model is loaded.
    on: Placement,
    /// Free video memory the loaded model takes from its card.
    video_use: u64,
}

/// One scripted answer of a [`FakeEngine`].
enum Step {
    /// Answers `text` after `takes`.
    Reply { text: String, takes: Duration },
    /// Writes nothing until the cancel flag is raised.
    Stall,
}

/// A [`Engine`] that loads nothing. Its [`FakeControl`] steers it and reports what happened.
pub struct FakeEngine {
    log: Arc<Mutex<EngineLog>>,
    progress: Option<ProgressHook>,
}

/// The remote control of a [`FakeEngine`] that was given to a worker.
#[derive(Clone)]
pub struct FakeControl {
    log: Arc<Mutex<EngineLog>>,
}

impl FakeEngine {
    pub fn new() -> (Self, FakeControl) {
        let log = Arc::new(Mutex::new(EngineLog::default()));
        (
            Self {
                log: Arc::clone(&log),
                progress: None,
            },
            FakeControl { log },
        )
    }

    /// An engine on the log `control` reads, as a new process would have it: nothing is loaded in it. Used
    /// by a [`ThreadSpawner`], whose helpers share one control.
    pub fn attached_to(control: &FakeControl) -> Self {
        {
            let mut log = locked(&control.log);
            log.loaded = false;
            log.on = Placement::Cpu;
        }

        Self {
            log: Arc::clone(&control.log),
            progress: None,
        }
    }

    fn report_progress(&self) {
        if let Some(progress) = &self.progress {
            progress();
        }
    }
}

impl FakeControl {
    /// What the engine was asked, in order: `load <threads>` (on the processor), `load <threads> gpu
    /// <index>`, `unload`, `judge`.
    pub fn calls(&self) -> Vec<String> {
        locked(&self.log).calls.clone()
    }

    pub fn is_loaded(&self) -> bool {
        locked(&self.log).loaded
    }

    /// How many loads began (a load held at its gate has begun).
    pub fn loads_started(&self) -> usize {
        locked(&self.log).loads_started
    }

    /// The graphics cards the engine reports.
    pub fn set_gpus(&self, gpus: Vec<GpuDevice>) {
        locked(&self.log).gpus = gpus;
    }

    /// Every load on a graphics card fails with `message`.
    pub fn fail_gpu_loads(&self, message: &str) {
        locked(&self.log).fail_gpu_load = Some(message.to_owned());
    }

    /// Every generation on a graphics card fails with `message`.
    pub fn fail_gpu_runs(&self, message: &str) {
        locked(&self.log).fail_gpu_run = Some(message.to_owned());
    }

    /// The next generation fails as text the model cannot take ([`AiError::Input`]).
    pub fn fail_next_input(&self, message: &str) {
        locked(&self.log).fail_input = Some(message.to_owned());
    }

    /// While loaded on a card, it takes this much of its free video memory.
    pub fn use_video_memory(&self, bytes: u64) {
        locked(&self.log).video_use = bytes;
    }

    /// Where the model is loaded.
    pub fn placement(&self) -> Placement {
        locked(&self.log).on
    }

    /// The next load fails with `message`.
    pub fn fail_next_load(&self, message: &str) {
        locked(&self.log).fail_load = Some(message.to_owned());
    }

    /// Loads wait at `gate` before they finish.
    pub fn hold_loads_at(&self, gate: &Arc<Gate>) {
        locked(&self.log).hold_load = Some(Arc::clone(gate));
    }

    /// What `judge` answers when its script is used up.
    pub fn set_reply(&self, text: &str) {
        locked(&self.log).reply = text.to_owned();
    }

    /// The next unscripted `judge` answers `text` at once.
    pub fn push_reply(&self, text: &str) {
        self.push_slow_reply(text, Duration::ZERO);
    }

    /// The next unscripted `judge` answers `text` after `takes` (and gives up when cancelled first).
    pub fn push_slow_reply(&self, text: &str, takes: Duration) {
        locked(&self.log).script.push_back(Step::Reply {
            text: text.to_owned(),
            takes,
        });
    }

    /// The next unscripted `judge` writes nothing until it is cancelled.
    pub fn push_stall(&self) {
        locked(&self.log).script.push_back(Step::Stall);
    }

    /// Every prompt `judge` was given, in order.
    pub fn prompts(&self) -> Vec<String> {
        locked(&self.log).prompts.clone()
    }

    /// How many times `judge` was called.
    pub fn judges(&self) -> usize {
        locked(&self.log).prompts.len()
    }
}

impl Engine for FakeEngine {
    fn gpus(&mut self) -> Vec<GpuDevice> {
        let log = locked(&self.log);
        let mut gpus = log.gpus.clone();
        if log.loaded
            && let Placement::Gpu { index } = log.on
            && let Some(card) = gpus.iter_mut().find(|card| card.index == index)
        {
            card.free_bytes = card.free_bytes.saturating_sub(log.video_use);
        }
        gpus
    }

    fn load(
        &mut self,
        _model: &VerifiedModel,
        threads: u32,
        placement: Placement,
    ) -> Result<LoadInfo, AiError> {
        let gate = {
            let mut log = locked(&self.log);
            log.loads_started += 1;
            log.loaded = false;
            log.on = Placement::Cpu;
            log.calls.push(match placement {
                Placement::Cpu => format!("load {threads}"),
                Placement::Gpu { index } => format!("load {threads} gpu {index}"),
            });
            log.hold_load.clone()
        };
        if let Some(gate) = gate {
            gate.pass();
        }

        let mut log = locked(&self.log);
        if let Some(message) = log.fail_load.take() {
            return Err(AiError::Load(message));
        }
        if matches!(placement, Placement::Gpu { .. })
            && let Some(message) = log.fail_gpu_load.clone()
        {
            return Err(AiError::Load(message));
        }
        log.loaded = true;
        log.on = placement;
        Ok(LoadInfo { millis: 1, threads })
    }

    fn unload(&mut self) {
        let mut log = locked(&self.log);
        log.calls.push("unload".to_owned());
        log.loaded = false;
        log.on = Placement::Cpu;
    }

    fn on_progress(&mut self, hook: ProgressHook) {
        self.progress = Some(hook);
    }

    fn judge(
        &mut self,
        prompt: &str,
        _grammar: &str,
        _max_tokens: u32,
        cancel: &AtomicBool,
    ) -> Result<Generation, AiError> {
        self.report_progress();
        let step = {
            let mut log = locked(&self.log);
            log.calls.push("judge".to_owned());

            if !log.loaded {
                return Err(AiError::NotLoaded);
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            if let Some(message) = log.fail_input.take() {
                return Err(AiError::Input(message));
            }
            if matches!(log.on, Placement::Gpu { .. })
                && let Some(message) = log.fail_gpu_run.clone()
            {
                return Err(AiError::Run(message));
            }

            log.prompts.push(prompt.to_owned());
            log.script.pop_front().unwrap_or_else(|| Step::Reply {
                text: log.reply.clone(),
                takes: Duration::ZERO,
            })
        };

        // Waits without holding the log, so the test can look at the engine meanwhile. A reply that takes a
        // while shows progress as it goes, as the real model does; a stall shows none.
        let started = Instant::now();
        let (takes, working) = match &step {
            Step::Reply { takes, .. } => (*takes, true),
            Step::Stall => (Duration::MAX, false),
        };
        while started.elapsed() < takes {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            if working {
                self.report_progress();
            }
            std::thread::sleep(Duration::from_millis(1));
        }

        match step {
            Step::Reply { text, .. } => Ok(Generation {
                text,
                tokens: 10,
                tokens_per_s: 20.0,
            }),
            Step::Stall => Err(AiError::Cancelled),
        }
    }
}
