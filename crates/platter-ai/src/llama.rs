//! The real model runtime: llama.cpp through `llama-cpp-2`, on the CPU.
//!
//! The backend is started once per process (a second start fails) and kept for good, with its logging
//! silenced. The model is loaded from the verified descriptor's `/proc/self/fd/N` path. Each answer
//! gets a fresh context (the context borrows the model, so it cannot be kept next to it without
//! `unsafe`); that costs a few milliseconds and means nothing of an earlier prompt can leak into the
//! next answer.

use std::num::NonZeroU32;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::sampling::LlamaSampler;

use crate::engine::{AiError, Engine, Generation, LoadInfo};
use crate::grammar::GRAMMAR_ROOT;
use crate::verify::VerifiedModel;

/// Most tokens a context can hold: the prompt and the answer. A call asks for no more than it needs
/// (rounded up to [`CONTEXT_ROUND`]), so a short labelling prompt does not pay for a long question.
pub const MAX_CONTEXT_TOKENS: u32 = 4096;
/// A context size is rounded up to a multiple of this.
const CONTEXT_ROUND: u32 = 256;
/// Tokens decoded at a time.
const BATCH_TOKENS: u32 = 512;

/// The model on llama.cpp. Create it once; load and unload as needed.
#[derive(Default)]
pub struct LlamaEngine {
    model: Option<LlamaModel>,
    threads: u32,
}

impl LlamaEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many tokens `text` is for the loaded model; `None` when none is loaded.
    pub fn count_tokens(&self, text: &str) -> Option<usize> {
        let model = self.model.as_ref()?;

        Some(model.vocab().tokenize(text.as_bytes(), false, true).len())
    }
}

/// The process's one backend. `Err` keeps the reason a start failed.
fn backend() -> Result<&'static LlamaBackend, AiError> {
    static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

    BACKEND
        .get_or_init(|| {
            let mut backend = LlamaBackend::init().map_err(|error| error.to_string())?;
            backend.void_logs();
            Ok(backend)
        })
        .as_ref()
        .map_err(|reason| AiError::Load(format!("The model runtime could not start: {reason}")))
}

impl Engine for LlamaEngine {
    fn load(&mut self, model: &VerifiedModel, threads: u32) -> Result<LoadInfo, AiError> {
        let backend = backend()?;
        self.model = None;

        let started = Instant::now();
        let loaded = LlamaModel::load_from_file(backend, model.path(), &LlamaModelParams::default())
            .map_err(|error| AiError::Load(format!("The model could not be loaded: {error}")))?;

        self.model = Some(loaded);
        self.threads = threads.max(1);
        Ok(LoadInfo {
            millis: started.elapsed().as_millis() as u64,
            threads: self.threads,
        })
    }

    fn unload(&mut self) {
        self.model = None;
    }

    fn judge(
        &mut self,
        prompt: &str,
        grammar: &str,
        max_tokens: u32,
        cancel: &AtomicBool,
    ) -> Result<Generation, AiError> {
        let model = self.model.as_ref().ok_or(AiError::NotLoaded)?;
        let backend = backend()?;
        let run = |error: &dyn std::fmt::Display| AiError::Run(format!("The model failed: {error}"));

        let vocab = model.vocab();
        let tokens = vocab.tokenize(prompt.as_bytes(), false, true);
        let needed = (tokens.len() as u32).saturating_add(max_tokens);
        if tokens.is_empty() || needed > MAX_CONTEXT_TOKENS {
            return Err(AiError::Run("The text is too long for the model.".to_owned()));
        }

        let threads = self.threads as i32;
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(needed.div_ceil(CONTEXT_ROUND) * CONTEXT_ROUND))
            .with_n_batch(BATCH_TOKENS)
            .with_n_threads(threads)
            .with_n_threads_batch(threads);
        let mut context = model.new_context(backend, params).map_err(|error| run(&error))?;

        // The prompt goes in in batches; only its last token needs logits.
        let mut batch = LlamaBatch::new(BATCH_TOKENS as usize, 1);
        let last = tokens.len() - 1;
        let mut position = 0usize;
        let mut index = 0;
        for chunk in tokens.chunks(BATCH_TOKENS as usize) {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            batch.clear();
            for token in chunk {
                batch
                    .add(*token, position as i32, &[0], position == last)
                    .map_err(|error| run(&error))?;
                position += 1;
            }
            context.decode(&mut batch).map_err(|error| run(&error))?;
            index = batch.n_tokens() - 1;
        }

        // A fresh sampler for every answer: the grammar remembers what it has accepted.
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::grammar(model, grammar, GRAMMAR_ROOT).map_err(|error| run(&error))?,
            LlamaSampler::greedy(),
        ]);

        let started = Instant::now();
        let mut text = Vec::new();
        let mut written = 0u32;
        while written < max_tokens {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }

            let token = sampler.sample(&context, index);
            if vocab.is_eog(token) {
                break;
            }
            text.extend(vocab.token_to_piece(token, false, None));
            written += 1;

            batch.clear();
            batch
                .add(token, position as i32, &[0], true)
                .map_err(|error| run(&error))?;
            position += 1;
            index = 0;
            context.decode(&mut batch).map_err(|error| run(&error))?;
        }

        let seconds = started.elapsed().as_secs_f64();
        Ok(Generation {
            text: String::from_utf8_lossy(&text).into_owned(),
            tokens: written,
            tokens_per_s: if seconds > 0.0 {
                f64::from(written) / seconds
            } else {
                0.0
            },
        })
    }
}
