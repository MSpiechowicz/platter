//! The model runtime as the rest of the crate sees it. The real one is `llama` (feature `llama`); tests
//! use `testing::FakeEngine`.

use std::sync::atomic::AtomicBool;

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

#[derive(Debug, thiserror::Error)]
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
}

/// A language model that can be loaded, asked, and unloaded. It is used from one thread at a time.
pub trait Engine: Send {
    /// Loads the verified file. A model that is already loaded is replaced.
    fn load(&mut self, model: &VerifiedModel, threads: u32) -> Result<LoadInfo, AiError>;

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
}
