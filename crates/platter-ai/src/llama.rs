//! The real model runtime: llama.cpp through `llama-cpp-2`, on the CPU or (feature `gpu-vulkan`) on one
//! graphics card.
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
use llama_cpp_2::model::params::{LlamaModelParams, LlamaSplitMode};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::{LlamaBackendDeviceType, list_llama_ggml_backend_devices};

use crate::engine::{AiError, Engine, Generation, GpuDevice, LoadInfo, Placement, ProgressHook};
use crate::grammar::GRAMMAR_ROOT;
use crate::verify::VerifiedModel;

/// Most tokens a context can hold: the prompt and the answer. A call asks for no more than it needs
/// (rounded up to [`CONTEXT_ROUND`]), so a short labelling prompt does not pay for a long question.
pub const MAX_CONTEXT_TOKENS: u32 = 4096;
/// A context size is rounded up to a multiple of this.
const CONTEXT_ROUND: u32 = 256;
/// Tokens decoded at a time.
const BATCH_TOKENS: u32 = 512;

/// The graphics cards llama.cpp sees. Empty in a CPU-only build and when no Vulkan driver is there.
pub fn gpus() -> Result<Vec<GpuDevice>, AiError> {
    backend()?;

    Ok(list_llama_ggml_backend_devices()
        .into_iter()
        .filter_map(|device| {
            let integrated = match device.device_type {
                LlamaBackendDeviceType::Gpu => false,
                LlamaBackendDeviceType::IntegratedGpu => true,
                _ => return None,
            };
            Some(GpuDevice {
                index: device.index,
                name: device.name,
                description: device.description,
                total_bytes: device.memory_total as u64,
                free_bytes: device.memory_free as u64,
                integrated,
            })
        })
        .collect())
}

/// The model on llama.cpp. Create it once; load and unload as needed.
#[derive(Default)]
pub struct LlamaEngine {
    model: Option<LlamaModel>,
    threads: u32,
    /// Where the loaded model is.
    loaded_on: Placement,
    /// Told that the engine is working: at every batch of the prompt and every token.
    progress: Option<ProgressHook>,
}

impl LlamaEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads a prompt and writes one token, to have the graphics driver compile what it needs.
    fn warm_up(&mut self) -> Result<(), AiError> {
        let never = AtomicBool::new(false);

        self.judge(&warm_up_prompt(), "root ::= \"a\"", 1, &never)
            .map(|_| ())
            .map_err(|error| AiError::Load(format!("The graphics card failed its first run: {error}")))
    }

    fn report_progress(&self) {
        if let Some(progress) = &self.progress {
            progress();
        }
    }

    /// How many tokens `text` is for the loaded model; `None` when none is loaded.
    pub fn count_tokens(&self, text: &str) -> Option<usize> {
        let model = self.model.as_ref()?;

        Some(model.vocab().tokenize(text.as_bytes(), false, true).len())
    }
}

/// What the graphics card is first given to read: a prompt of more than one batch (the driver compiles a
/// different kernel for a full batch and for a short one) and an answer of one token.
fn warm_up_prompt() -> String {
    "The quick brown fox jumps over the lazy dog. ".repeat(70)
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
    fn gpus(&mut self) -> Vec<GpuDevice> {
        gpus().unwrap_or_default()
    }

    fn load(
        &mut self,
        model: &VerifiedModel,
        threads: u32,
        placement: Placement,
    ) -> Result<LoadInfo, AiError> {
        let backend = backend()?;
        self.model = None;
        self.loaded_on = Placement::Cpu;

        let params = match placement {
            // An empty device list (not none, which lets llama.cpp take every card): no graphics backend
            // is created for the model or its contexts, so nothing reaches a card's driver.
            Placement::Cpu => LlamaModelParams::default()
                .with_devices(&[])
                .map_err(|error| AiError::Load(format!("The model could not be loaded: {error}")))?
                .with_n_gpu_layers(0),
            Placement::Gpu { index } => LlamaModelParams::default()
                .with_devices(&[index])
                .map_err(|error| AiError::Load(format!("The graphics card is not there: {error}")))?
                .with_split_mode(LlamaSplitMode::None)
                .with_main_gpu(0)
                .with_n_gpu_layers(u32::MAX),
        };

        let started = Instant::now();
        let loaded = LlamaModel::load_from_file(backend, model.path(), &params)
            .map_err(|error| AiError::Load(format!("The model could not be loaded: {error}")))?;

        self.model = Some(loaded);
        self.loaded_on = placement;
        self.threads = threads.max(1);

        // A graphics driver compiles its pipelines on first use, which can take seconds and shows no
        // progress: do it here, where the load's own (long) limit covers it, so the first answer is not
        // taken for a hang.
        if matches!(placement, Placement::Gpu { .. }) {
            self.warm_up().inspect_err(|_| self.unload())?;
        }

        Ok(LoadInfo {
            millis: started.elapsed().as_millis() as u64,
            threads: self.threads,
        })
    }

    fn unload(&mut self) {
        self.model = None;
        self.loaded_on = Placement::Cpu;
    }

    fn on_progress(&mut self, hook: ProgressHook) {
        self.progress = Some(hook);
    }

    fn judge(
        &mut self,
        prompt: &str,
        grammar: &str,
        max_tokens: u32,
        cancel: &AtomicBool,
    ) -> Result<Generation, AiError> {
        self.report_progress();
        let model = self.model.as_ref().ok_or(AiError::NotLoaded)?;
        let backend = backend()?;
        let run = |error: &dyn std::fmt::Display| AiError::Run(format!("The model failed: {error}"));

        let vocab = model.vocab();
        let tokens = vocab.tokenize(prompt.as_bytes(), false, true);
        let needed = (tokens.len() as u32).saturating_add(max_tokens);
        if tokens.is_empty() || needed > MAX_CONTEXT_TOKENS {
            return Err(AiError::Input("The text is too long for the model.".to_owned()));
        }

        let threads = self.threads as i32;
        let on_gpu = matches!(self.loaded_on, Placement::Gpu { .. });
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(needed.div_ceil(CONTEXT_ROUND) * CONTEXT_ROUND))
            .with_n_batch(BATCH_TOKENS)
            .with_n_threads(threads)
            .with_n_threads_batch(threads)
            // On the processor nothing is handed to a graphics card, not even large batches.
            .with_offload_kqv(on_gpu)
            .with_op_offload(on_gpu);
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
            self.report_progress();
        }

        // A fresh sampler for every answer: the grammar remembers what it has accepted.
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::grammar(model, grammar, GRAMMAR_ROOT)
                .map_err(|error| AiError::Input(format!("The model could not use the grammar: {error}")))?,
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
            self.report_progress();
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
