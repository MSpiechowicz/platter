//! The model runtime as the rest of the crate sees it. The real one is `llama` (feature `llama`); tests
//! use `testing::FakeEngine`.

use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use crate::verify::VerifiedModel;

/// What loading a model took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadInfo {
    /// Wall time of the load.
    pub millis: u64,
    /// Threads the model will compute with.
    pub threads: u32,
}

/// One answer of the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Generation {
    /// What the model wrote (no more than the token cap, so it may be cut short).
    pub text: String,
    /// Tokens written.
    pub tokens: u32,
    /// Writing speed.
    pub tokens_per_s: f64,
}

/// Where a model is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    /// On the processor: no layer, no cache and no operation is given to a graphics card.
    #[default]
    Cpu,
    /// Every layer and the cache on one graphics card, by its index in [`Engine::gpus`].
    Gpu { index: usize },
}

/// A graphics card the runtime can run the model on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuDevice {
    /// The runtime's index of the device.
    pub index: usize,
    /// The runtime's name of the device (`Vulkan0`).
    pub name: String,
    /// The driver's description (`AMD Radeon RX 7900 XTX (RADV NAVI31)`).
    pub description: String,
    /// Video memory in all, as the driver reports it.
    pub total_bytes: u64,
    /// Free video memory, as the driver reports it.
    pub free_bytes: u64,
    /// A graphics part of the processor, which shares the system's memory.
    pub integrated: bool,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AiError {
    /// The model could not be loaded. The text is for the user.
    #[error("{0}")]
    Load(String),
    #[error("The model is not loaded.")]
    NotLoaded,
    /// The cancel flag was raised.
    #[error("Cancelled.")]
    Cancelled,
    /// The model failed while answering. The text is for the user.
    #[error("{0}")]
    Run(String),
    /// The text could not be given to the model (too long for its context, a grammar it cannot use):
    /// the same on any device, so never a reason to move the model. The text is for the user.
    #[error("{0}")]
    Input(String),
}

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// It left with this exit code.
    Code(i32),
    /// A signal ended it (`11` is a segmentation fault).
    Signal(i32),
}

/// How a helper process of the runtime went wrong (see [`RuntimeIncident`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncidentKind {
    /// The process died (a driver crash ends it with a signal).
    Crash,
    /// It stopped answering and was killed.
    Hang,
    /// The runtime reported an error.
    Error,
}

/// What went wrong in the process the model runs in, kept until it is taken with
/// [`Engine::take_incident`]. A crash or a hang also fails the call it happened in, as
/// [`AiError::Run`] (or [`AiError::Load`] while loading), so a caller that only looks at errors treats it
/// like any other failure of the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIncident {
    pub what: IncidentKind,
    /// How the process ended, when it ended on its own or was killed.
    pub exit: Option<Exit>,
    /// The last lines the process wrote to its standard error, cleaned of control characters.
    pub stderr_tail: String,
    /// A short sentence on what was seen.
    pub detail: String,
}

/// Tells that a call is making progress: called while a model is loading and while it reads and writes. A
/// runtime in another process uses it to show the other side that it has not hung.
pub type ProgressHook = Box<dyn Fn() + Send>;

/// A language model that can be loaded, asked, and unloaded. It is used from one thread at a time.
pub trait Engine: Send {
    /// The graphics cards the model could be loaded on, with their free memory now. None by default (a
    /// runtime without a graphics backend).
    fn gpus(&mut self) -> Vec<GpuDevice> {
        Vec::new()
    }

    /// Loads the verified file at `placement`. A model that is already loaded is replaced.
    fn load(
        &mut self,
        model: &VerifiedModel,
        threads: u32,
        placement: Placement,
    ) -> Result<LoadInfo, AiError>;

    /// Frees the model. Does nothing when none is loaded.
    fn unload(&mut self);

    /// Writes an answer to `prompt` that fits `grammar` (GBNF, rule `root`), at most `max_tokens` long,
    /// greedily. Returns `Cancelled` when `cancel` is raised, checked between tokens. Each call starts
    /// from nothing: nothing of an earlier prompt is remembered.
    fn judge(
        &mut self,
        prompt: &str,
        grammar: &str,
        max_tokens: u32,
        cancel: &AtomicBool,
    ) -> Result<Generation, AiError>;

    /// Where to report progress from now on. Nothing by default (a runtime that cannot hang on its own).
    fn on_progress(&mut self, hook: ProgressHook) {
        drop(hook);
    }

    /// What went wrong in the runtime's process since the last call, once. Nothing by default (a runtime
    /// in this process has no incident but an error it returns).
    fn take_incident(&mut self) -> Option<RuntimeIncident> {
        None
    }

    /// The number of the process the loaded model runs in. `None` when it runs in this one, or is not loaded.
    fn process_id(&self) -> Option<u32> {
        None
    }
}
