//! The labelling loop: asks the model about folder after folder and keeps what can be read.
//!
//! A folder whose answer cannot be read, or that takes longer than its time, is skipped: nothing is
//! assumed in its place. Two clocks run: 15 seconds for one folder and 5 minutes for the job. The engine
//! only looks at a cancel flag between tokens, so a watcher thread raises the engine's flag when the
//! user cancels or a time runs out.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use platter_core::{ModelLabel, NodeId};

use crate::engine::{AiError, Engine, Generation};
use crate::facts::FolderFacts;
use crate::grammar::VERDICT_GRAMMAR;
use crate::parse::{Judgement, parse};
use crate::prompt::prompt;

/// How often the watcher looks at the user's cancel flag.
const WATCH_TICK: Duration = Duration::from_millis(20);

/// The time and the tokens the model may use to read a free question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageLimit {
    pub time: Duration,
    /// Tokens the model may write.
    pub max_tokens: u32,
}

/// The limits of the jobs: labelling folders, and reading a free question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Time for one folder.
    pub per_folder: Duration,
    /// Time for the whole job.
    pub total: Duration,
    /// Tokens the model may write for one folder.
    pub max_tokens: u32,
    /// Reading a question into a query.
    pub interpret: StageLimit,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            per_folder: Duration::from_secs(15),
            total: Duration::from_secs(5 * 60),
            max_tokens: 96,
            // The worker uses the model's own time (`ModelSpec::interpret_limit`) unless a budget is set;
            // this is the usual one for the runtime tests.
            interpret: StageLimit {
                time: Duration::from_secs(15),
                max_tokens: 64,
            },
        }
    }
}

/// Why a job ended before every folder was asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    Cancelled,
    TimeBudget,
}

/// What a job made. The labels are those of the folders that were answered and read, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelRun {
    pub labels: Vec<ModelLabel>,
    /// Wall time of the job.
    pub ms: u64,
    /// Why the job ended early; `None` when every folder was asked about.
    pub stopped: Option<Stopped>,
    /// Speed of the last answer, when there was one.
    pub tokens_per_s: Option<f64>,
}

/// How one question went.
#[derive(Debug)]
pub(crate) enum Outcome {
    Judged(Judgement, Generation),
    /// The user's cancel flag was raised.
    Cancelled,
    /// The time ran out.
    TimedOut,
    /// The model wrote something that cannot be used.
    Unreadable,
    /// The runtime failed.
    Failed(AiError),
}

/// Asks about every folder of `items`, in order. `progress(done, total)` is told after each folder that
/// was dealt with (answered or skipped).
pub fn label(
    engine: &mut dyn Engine,
    items: &[(NodeId, FolderFacts)],
    budget: &Budget,
    cancel: &AtomicBool,
    progress: &dyn Fn(usize, usize),
) -> LabelRun {
    let started = Instant::now();
    let total = items.len();
    let mut labels = Vec::new();
    let mut tokens_per_s = None;
    let mut stopped = None;

    for (done, (node, facts)) in items.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            stopped = Some(Stopped::Cancelled);
            break;
        }

        let left = budget.total.saturating_sub(started.elapsed());
        if left.is_zero() {
            stopped = Some(Stopped::TimeBudget);
            break;
        }
        // A folder gets its own time, but never more than what is left of the job.
        let limited_by_job = left <= budget.per_folder;
        let limit = left.min(budget.per_folder);

        match ask(engine, facts, limit, budget.max_tokens, cancel) {
            Outcome::Judged(judgement, generation) => {
                tokens_per_s = Some(generation.tokens_per_s);
                labels.push(ModelLabel {
                    node: *node,
                    verdict: judgement.verdict,
                    reason: judgement.reason,
                });
            }
            Outcome::Cancelled => {
                stopped = Some(Stopped::Cancelled);
                break;
            }
            Outcome::TimedOut if limited_by_job => {
                stopped = Some(Stopped::TimeBudget);
                break;
            }
            // This folder took too long, or the model wrote nothing usable: skip it.
            Outcome::TimedOut | Outcome::Unreadable | Outcome::Failed(_) => {}
        }

        progress(done + 1, total);
    }

    LabelRun {
        labels,
        ms: started.elapsed().as_millis() as u64,
        stopped,
        tokens_per_s,
    }
}

/// Asks the model about one folder, within `limit`, and reads the answer.
pub(crate) fn ask(
    engine: &mut dyn Engine,
    facts: &FolderFacts,
    limit: Duration,
    max_tokens: u32,
    cancel: &AtomicBool,
) -> Outcome {
    match generate(engine, &prompt(facts), VERDICT_GRAMMAR, limit, max_tokens, cancel) {
        Generated::Done(generation) => match parse(&generation.text) {
            Ok(judgement) => Outcome::Judged(judgement, generation),
            Err(_) => Outcome::Unreadable,
        },
        Generated::Cancelled => Outcome::Cancelled,
        Generated::TimedOut => Outcome::TimedOut,
        Generated::Failed(error) => Outcome::Failed(error),
    }
}

/// How one run of the model ended, before its text is read.
#[derive(Debug)]
pub(crate) enum Generated {
    Done(Generation),
    /// The user's cancel flag was raised.
    Cancelled,
    /// The time ran out.
    TimedOut,
    /// The runtime failed.
    Failed(AiError),
}

/// Runs the model on `prompt` with `grammar`, within `limit`. A watcher thread raises the engine's
/// flag when `cancel` is raised or the time runs out, because the engine only looks at its flag
/// between tokens. Labelling and questions use this one.
pub(crate) fn generate(
    engine: &mut dyn Engine,
    prompt: &str,
    grammar: &str,
    limit: Duration,
    max_tokens: u32,
    cancel: &AtomicBool,
) -> Generated {
    let stop = AtomicBool::new(false);
    let timed_out = AtomicBool::new(false);
    let (finished, watch) = mpsc::channel::<()>();

    let generated = thread::scope(|scope| {
        let (stop, timed_out) = (&stop, &timed_out);
        scope.spawn(move || {
            let deadline = Instant::now() + limit;

            loop {
                let wait = deadline.saturating_duration_since(Instant::now()).min(WATCH_TICK);
                if watch.recv_timeout(wait) != Err(RecvTimeoutError::Timeout) {
                    return;
                }

                if cancel.load(Ordering::SeqCst) {
                    stop.store(true, Ordering::SeqCst);
                    return;
                }
                if Instant::now() >= deadline {
                    timed_out.store(true, Ordering::SeqCst);
                    stop.store(true, Ordering::SeqCst);
                    return;
                }
            }
        });

        let generated = engine.judge(prompt, grammar, max_tokens, stop);
        let _ = finished.send(());
        generated
    });

    match generated {
        Ok(generation) => Generated::Done(generation),
        Err(AiError::Cancelled) if cancel.load(Ordering::SeqCst) => Generated::Cancelled,
        Err(AiError::Cancelled) if timed_out.load(Ordering::SeqCst) => Generated::TimedOut,
        Err(error) => Generated::Failed(error),
    }
}
