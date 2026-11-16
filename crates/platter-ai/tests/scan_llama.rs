//! The experiment for free questions about a scan, on the real model. Needs the model file and a
//! build with `--features llama`, so it is ignored by default:
//!
//! ```text
//! PLATTER_MODEL=/path/Qwen3.5-4B-Q4_K_M.gguf \
//!   PLATTER_ANSWERS=/dir/answers cargo test --release -p platter-ai --features llama,testing \
//!   --test scan_llama -- --ignored --nocapture
//! ```
//!
//! `PLATTER_MODEL` names the file. `PLATTER_SETS` limits the questions to the sets named
//! (`tune,heldout,hostile,polish`; default all): prompt tuning uses `tune` only. `PLATTER_ANSWERS` is a
//! prefix: the model's raw readings go to `<prefix>.jsonl`, the summary to `<prefix>.summary.json`.
//!
//! The model only reads each question into a query (the answer is Platter's own text, written from the
//! facts), so this measures how well it reads: the accuracy of the intent, the scope, the kind and the
//! limit per set, the confusion between intents and the time of each reading. The result is only printed
//! and written, never asserted: the decision is the owner's.
#![cfg(feature = "llama")]

#[path = "question.rs"]
mod question;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use platter_ai::grammar::QUESTION_GRAMMAR;
use platter_ai::llama::LlamaEngine;
use platter_ai::{
    AiError, Engine, Generation, MODEL, ScanQuestion, clean_question, ground, interpret_prompt,
    parse_question, verify,
};
use question::fixture::{Item, load};
use serde_json::json;

/// Reading a question: 64 tokens, 15 s (the experiment measures; the app's limit is chosen from these
/// figures).
const A_TOKENS: u32 = 64;
const A_LIMIT: Duration = Duration::from_secs(15);
const THREADS: u32 = 8;

enum Failure {
    TimedOut,
    Failed(String),
}

/// One model run within `limit`: the engine is told to stop when the time is up.
fn generate(
    engine: &mut LlamaEngine,
    prompt: &str,
    grammar: &str,
    max_tokens: u32,
    limit: Duration,
) -> Result<(Generation, f64), Failure> {
    let stop = AtomicBool::new(false);
    let timed_out = AtomicBool::new(false);
    let (finished, watch) = mpsc::channel::<()>();
    let started = Instant::now();

    let result = thread::scope(|scope| {
        let (stop, timed_out) = (&stop, &timed_out);
        scope.spawn(move || {
            if watch.recv_timeout(limit) == Err(RecvTimeoutError::Timeout) {
                timed_out.store(true, Ordering::SeqCst);
                stop.store(true, Ordering::SeqCst);
            }
        });

        let result = engine.judge(prompt, grammar, max_tokens, stop);
        let _ = finished.send(());
        result
    });
    let seconds = started.elapsed().as_secs_f64();

    match result {
        Ok(generation) => Ok((generation, seconds)),
        Err(AiError::Cancelled) if timed_out.load(Ordering::SeqCst) => Err(Failure::TimedOut),
        Err(error) => Err(Failure::Failed(error.to_string())),
    }
}

fn norm_scope(scope: &str) -> String {
    let scope = scope.trim().to_lowercase();
    let scope = scope.strip_prefix("~/").unwrap_or(&scope);

    scope.trim_end_matches('/').to_owned()
}

/// What the model made of one question.
struct Reading {
    asked: ScanQuestion,
    grounded: ScanQuestion,
    raw: String,
    seconds: f64,
    tokens: u32,
    tokens_per_s: f64,
    prompt_tokens: usize,
    /// "ok", "unreadable" or "timeout".
    state: &'static str,
}

fn read_question(engine: &mut LlamaEngine, question: &str) -> Reading {
    let prompt = interpret_prompt(question);
    let prompt_tokens = engine.count_tokens(&prompt).unwrap_or(0);
    let general = ScanQuestion::general();

    let (asked, raw, seconds, tokens, tokens_per_s, state) =
        match generate(engine, &prompt, QUESTION_GRAMMAR, A_TOKENS, A_LIMIT) {
            Ok((generation, seconds)) => match parse_question(&generation.text) {
                Ok(asked) => (
                    asked,
                    generation.text,
                    seconds,
                    generation.tokens,
                    generation.tokens_per_s,
                    "ok",
                ),
                Err(_) => (
                    general,
                    generation.text,
                    seconds,
                    generation.tokens,
                    generation.tokens_per_s,
                    "unreadable",
                ),
            },
            Err(Failure::TimedOut) => (general, String::new(), A_LIMIT.as_secs_f64(), 0, 0.0, "timeout"),
            Err(Failure::Failed(message)) => (general, message, 0.0, 0, 0.0, "unreadable"),
        };
    let grounded = ground(question, asked.clone());

    Reading {
        asked,
        grounded,
        raw,
        seconds,
        tokens,
        tokens_per_s,
        prompt_tokens,
        state,
    }
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = ((sorted.len() as f64 * fraction).ceil() as usize).clamp(1, sorted.len()) - 1;

    sorted[at]
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// `Vm<field>` of `/proc/self/status`, in bytes.
fn proc_status_bytes(field: &str) -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();

    status
        .lines()
        .find_map(|line| line.strip_prefix(field)?.strip_prefix(':'))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
        .map_or(0, |kib| kib * 1024)
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

/// Counts of one set of questions.
#[derive(Default)]
struct Accuracy {
    n: usize,
    intent: usize,
    scope: usize,
    kind: usize,
    limit: usize,
    all: usize,
    unreadable: usize,
}

impl Accuracy {
    fn add(&mut self, expected: &ScanQuestion, got: &ScanQuestion, state: &str) {
        let intent = expected.intent == got.intent;
        let scope = norm_scope(&expected.scope) == norm_scope(&got.scope);
        let kind = expected.kind == got.kind;
        let limit = expected.limit == got.limit;

        self.n += 1;
        self.intent += usize::from(intent);
        self.scope += usize::from(scope);
        self.kind += usize::from(kind);
        self.limit += usize::from(limit);
        self.all += usize::from(intent && scope && kind && limit);
        self.unreadable += usize::from(state != "ok");
    }

    fn json(&self) -> serde_json::Value {
        let pct = |n: usize| (n as f64 * 1000.0 / self.n.max(1) as f64).round() / 10.0;

        json!({
            "n": self.n,
            "intent_pct": pct(self.intent), "scope_pct": pct(self.scope), "kind_pct": pct(self.kind),
            "limit_pct": pct(self.limit), "all_fields_pct": pct(self.all), "unreadable": self.unreadable,
        })
    }
}

fn run_model(path: PathBuf, sets: &[String], answers_prefix: Option<String>) {
    let spec = MODEL;
    eprintln!("=== {} ({}) ===", spec.name, path.display());

    let rss_before = proc_status_bytes("VmRSS");
    let started = Instant::now();
    let verified = verify(&spec, &path).expect("the model file matches its pinned size and checksum");
    let verify_ms = started.elapsed().as_millis();
    let mut engine = LlamaEngine::new();
    let info = engine.load(&verified, THREADS).expect("the model loads");
    let load_total_ms = started.elapsed().as_millis();
    let rss_loaded = proc_status_bytes("VmRSS");
    eprintln!(
        "verify {verify_ms} ms, load {} ms, total {load_total_ms} ms; RSS {:.0} -> {:.0} MiB",
        info.millis,
        mib(rss_before),
        mib(rss_loaded)
    );

    let items: Vec<Item> = load()
        .into_iter()
        .filter(|item| sets.contains(&item.set))
        .collect();
    let mut records = Vec::new();
    let mut by_set: BTreeMap<String, Accuracy> = BTreeMap::new();
    let mut confusion: BTreeMap<(String, String), usize> = BTreeMap::new();

    let (mut a_times, mut a_speed, mut a_prompt) = (Vec::new(), Vec::new(), Vec::new());

    for item in &items {
        let question = clean_question(&item.question).expect("a fixture question is valid");
        let expected = item.expected();

        let a = read_question(&mut engine, &question);
        if item.set == "tune" || item.set == "heldout" || item.set == "polish" {
            by_set
                .entry(item.set.clone())
                .or_default()
                .add(&expected, &a.grounded, a.state);
            *confusion
                .entry((
                    expected.intent.as_str().to_owned(),
                    a.grounded.intent.as_str().to_owned(),
                ))
                .or_default() += 1;
        }
        a_times.push(a.seconds);
        a_speed.push(a.tokens_per_s);
        a_prompt.push(a.prompt_tokens as f64);

        let record = json!({
            "model": spec.name, "id": item.id, "set": item.set, "question": item.question,
            "expect": { "intent": expected.intent.as_str(), "scope": expected.scope, "kind": expected.kind.as_str(), "limit": expected.limit },
            "a": {
                "state": a.state, "raw": a.raw, "seconds": a.seconds, "tokens": a.tokens, "prompt_tokens": a.prompt_tokens,
                "read": { "intent": a.asked.intent.as_str(), "scope": a.asked.scope, "kind": a.asked.kind.as_str(), "limit": a.asked.limit },
                "understood": { "intent": a.grounded.intent.as_str(), "scope": a.grounded.scope, "kind": a.grounded.kind.as_str(), "limit": a.grounded.limit },
            },
        });

        records.push(record);
        eprintln!("{} {:.1}s", item.id, a.seconds);
    }

    let rss_after = proc_status_bytes("VmRSS");
    let hwm = proc_status_bytes("VmHWM");

    let summary = json!({
        "model": spec.name, "threads": THREADS,
        "load": { "verify_ms": verify_ms, "load_ms": info.millis, "total_ms": load_total_ms, "rss_before_mib": mib(rss_before), "rss_loaded_mib": mib(rss_loaded) },
        "memory": { "rss_after_questions_mib": mib(rss_after), "peak_rss_mib": mib(hwm) },
        "reading": by_set.iter().map(|(set, acc)| (set.clone(), acc.json())).collect::<BTreeMap<_, _>>(),
        "confusion": confusion.iter().map(|((e, g), n)| format!("{e} -> {g}: {n}")).collect::<Vec<_>>(),
        "timing_s": { "p50": percentile(&a_times, 0.5), "p95": percentile(&a_times, 0.95), "max": percentile(&a_times, 1.0) },
        "tokens_per_s": mean(&a_speed.iter().copied().filter(|v| *v > 0.0).collect::<Vec<_>>()),
        "prompt_tokens": mean(&a_prompt),
    });
    eprintln!("{}", serde_json::to_string_pretty(&summary).unwrap());

    if let Some(prefix) = answers_prefix {
        let lines: Vec<String> = records.iter().map(|record| record.to_string()).collect();
        fs::write(format!("{prefix}.jsonl"), lines.join("\n") + "\n").unwrap();
        fs::write(
            format!("{prefix}.summary.json"),
            serde_json::to_string_pretty(&summary).unwrap(),
        )
        .unwrap();
    }
    engine.unload();
}

#[test]
#[ignore = "needs the real model: set PLATTER_MODEL"]
fn the_model_answers_free_questions_about_a_scan() {
    let sets: Vec<String> = std::env::var("PLATTER_SETS")
        .unwrap_or_else(|_| "tune,heldout,hostile,polish".to_owned())
        .split(',')
        .map(str::to_owned)
        .collect();
    let answers = std::env::var("PLATTER_ANSWERS").ok();
    let path = std::env::var_os("PLATTER_MODEL").expect("set PLATTER_MODEL to the model file");

    run_model(PathBuf::from(path), &sets, answers);
}
