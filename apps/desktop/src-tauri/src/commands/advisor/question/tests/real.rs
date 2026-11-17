//! A real end-to-end check of `ask_scan`: a real scan of a real folder, a real local model (llama.cpp) and the
//! real answer flow. Never part of a normal run (`#[ignore]`); it reads the disk and writes nothing but its
//! own output file.
//!
//! ```text
//! PLATTER_MODEL=/path/Qwen3.5-4B-Q4_K_M.gguf PLATTER_SCAN_ROOT=$HOME PLATTER_OUT=/path/out.jsonl \
//!     cargo test -p platter-desktop --release --lib -- --ignored --nocapture real_questions
//! ```
//!
//! - `PLATTER_MODEL`: the gguf file (it must be the pinned model's file name; the folder it is in is the
//!   models folder, checked against the pinned size and hash like the app's, and the app's start-up
//!   removal of the retired model's files applies to it). Required.
//! - `PLATTER_SCAN_ROOT`: the folder to scan, read only (default `$HOME`).
//! - `PLATTER_QUESTIONS`: a file with one question per line (default: [`QUESTIONS`]).
//! - `PLATTER_OUT`: the JSON lines file written, one object per question (default: none).
//! - `PLATTER_ONE_FS`: `0` to scan across mount points too (default: stay on one file system, as the app's
//!   scan of a disk does).
//! - `PLATTER_DEVICE`: `gpu` (a build with `--features gpu-vulkan`) switches "Use the graphics card" on, so the
//!   worker loads on a card as the app does; anything else is the CPU (default).
//! - `PLATTER_RUNTIME_EXE`: the path of a built `platter` binary (`cargo build --release -p platter-desktop`): the
//!   model then runs in that binary's helper process (`--advisor-runtime`), as in the app. Without it the model
//!   runs inside the test.
//! - `PLATTER_EVAL`: `0` to skip the score of the evaluation set (`reading.json`: 60 questions, 40 to tune on
//!   and 20 held out), read by the model and by the words alone (default: scored, a model call each).
//!
//! The scan, the rules (the user's own rules file is only read), the guard and the answer are the app's.
//! Stage A (the model reading the question) is timed with a first reading of the question, then the whole
//! answer, which reads it again, is timed: `stage_a_ms` is the first, `total_ms` the second.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use platter_ai::{Engine, MODEL, ProcessEngine, RealSpawner, WorkerConfig};
use serde_json::json;

use super::super::super::tests::block_on;
use super::super::super::{AdvisorOptionsDto, AdvisorService, UnloadAfterDto};
use super::super::reading::read;
use super::*;
use crate::commands::scan::{resolve_root, run_scan, scan_config};
use crate::state::AppState;
use crate::user_rules::UserRulesStore;

const QUESTIONS: [&str; 25] = [
    "which folder does take the biggest amount of data?",
    "can i safely delete the docker images",
    "what's eating my disk",
    "what can I clean up?",
    "can I delete node_modules?",
    "is it safe to remove the Steam shader cache",
    "why is ~/.cache so big?",
    "where are my rust build folders?",
    "how much do my videos take?",
    "what did the model label?",
    "can I delete my Downloads?",
    "is ~/Documents safe to delete?",
    "what are my biggest caches?",
    "clean everything",
    "co zajmuje najwięcej miejsca?",
    "co mogę usunąć?",
    "czy mogę usunąć obrazy dockera?",
    "write me a poem",
    "how big is my docker data?",
    "where is docker?",
    "how much space does steam use?",
    "what's in ~/.cache?",
    "can I delete ~/Pobrane?",
    "jakie foldery są największe?",
    "ile miejsca zajmują filmy?",
];

fn questions() -> Vec<String> {
    let Some(file) = std::env::var_os("PLATTER_QUESTIONS") else {
        return QUESTIONS.iter().map(|question| (*question).to_owned()).collect();
    };

    std::fs::read_to_string(file)
        .expect("PLATTER_QUESTIONS can be read")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whether `PLATTER_DEVICE` asks for the graphics card (the worker then chooses it as the app does).
fn use_gpu() -> bool {
    let use_gpu = std::env::var("PLATTER_DEVICE").as_deref() == Ok("gpu");
    println!("use the graphics card: {use_gpu}");
    use_gpu
}

/// Video memory in use on every card, in MiB, as the driver reports it.
fn video_memory_used() -> Vec<(String, u64)> {
    platter_ai::llama::gpus()
        .unwrap_or_default()
        .into_iter()
        .map(|card| (card.name, card.total_bytes.saturating_sub(card.free_bytes) >> 20))
        .collect()
}

/// The model's runtime: in the helper process of the binary `PLATTER_RUNTIME_EXE` names, or else in this process.
fn runtime() -> Box<dyn Engine> {
    match std::env::var_os("PLATTER_RUNTIME_EXE") {
        Some(program) => {
            println!("the model runs in a helper process of {program:?}");
            Box::new(ProcessEngine::new(RealSpawner {
                program: PathBuf::from(program),
            }))
        }
        None => Box::new(platter_ai::llama::LlamaEngine::new()),
    }
}

/// The advisor on the given model file, on the app's worker and llama.cpp engine.
fn advisor_on(model: &Path) -> AdvisorService {
    let file = model
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a file name");
    assert_eq!(file, MODEL.file, "{file} is not the pinned model file");
    let dir = model.parent().expect("a models folder").to_path_buf();

    let config = WorkerConfig::new(MODEL, dir);
    let fetch = Arc::new(platter_ai::UreqFetch::new());
    let service = AdvisorService::spawn(config, runtime(), fetch).expect("the worker starts");

    service
        .configure(AdvisorOptionsDto {
            enabled: true,
            unload_after: UnloadAfterDto::WindowClose,
            log_questions: true,
            use_gpu: use_gpu(),
        })
        .expect("the advisor is switched on");

    service
}

/// Scans `root` as the app does and returns the scan's id. Reads only.
fn scan_real(state: &AppState, root: &str, one_file_system: bool) -> ScanId {
    let _ = platter_platform::raise_fd_limit();
    let resolved = resolve_root(root).expect("the root can be scanned");
    let config = scan_config(one_file_system, resolved.identity);
    let (id, cancel) = state.scans.begin().expect("a scan can begin");

    run_scan(
        &state.platform.dir_source,
        &state.scans,
        id,
        &cancel,
        &resolved.path,
        &config,
        &|_| {},
    );
    assert!(state.scans.tree(id).is_ok(), "the scan finished");

    id
}

/// Reads every question of the evaluation set with the real model (the words put before its reading, as the
/// app does) and with the words alone, and prints both scores and every miss.
fn score_evaluation_set(state: &AppState, out: &mut Option<std::fs::File>) {
    let rules = platter_rules::RuleSet::builtin();
    let os = platter_core::Os::Linux;

    let times = std::cell::RefCell::new(Vec::new());
    let with_model = super::eval::score(|question| {
        let began = Instant::now();
        let model = state.advisor.interpret(question).ok();
        times.borrow_mut().push(began.elapsed());

        read(question, model, &rules, os)
    });
    let mut times = times.into_inner();
    times.sort();
    if !times.is_empty() {
        let at = |share: usize| times[(times.len() * share / 100).min(times.len() - 1)];
        println!(
            "\nevaluation set, model reading ({} questions): p50 {:.2} s, p95 {:.2} s, max {:.2} s; model memory {:?} bytes; this process {:?} bytes; video memory used {:?} MiB",
            times.len(),
            at(50).as_secs_f64(),
            at(95).as_secs_f64(),
            times.last().map_or(0.0, |time| time.as_secs_f64()),
            state.advisor.status().ram_bytes,
            platter_platform::resident_bytes(),
            video_memory_used()
        );
    }
    let words_only = super::eval::score(super::eval::words_only);

    for (name, score) in [("with the model", &with_model), ("words only", &words_only)] {
        println!("\nevaluation set, {name}: {}", score.report());
        for miss in &score.misses {
            println!("  miss {miss}");
        }
        if let Some(out) = out {
            let line = json!({
                "evaluation": name,
                "tune": [score.tune.0, score.tune.1],
                "heldout": [score.heldout.0, score.heldout.1],
                "misses": score.misses,
            });
            writeln!(out, "{line}").expect("PLATTER_OUT can be written");
        }
    }
}

fn ms(time: Duration) -> u64 {
    u64::try_from(time.as_millis()).unwrap_or(u64::MAX)
}

#[test]
#[ignore = "needs a real model, a real disk and minutes: see the module's documentation"]
fn real_questions() {
    let model = PathBuf::from(std::env::var_os("PLATTER_MODEL").expect("PLATTER_MODEL is the gguf file"));
    let root = std::env::var("PLATTER_SCAN_ROOT")
        .ok()
        .or_else(|| std::env::var("HOME").ok())
        .expect("PLATTER_SCAN_ROOT or HOME");
    let one_file_system = std::env::var("PLATTER_ONE_FS").map_or(true, |value| value != "0");
    let mut out = std::env::var_os("PLATTER_OUT")
        .map(|path| std::fs::File::create(path).expect("PLATTER_OUT can be written"));

    // The user's rules file is read (never written: nothing here changes a rule).
    let state = AppState::with_parts(advisor_on(&model), UserRulesStore::production());

    let began = Instant::now();
    let scan = scan_real(&state, &root, one_file_system);
    let tree = state.scans.tree(scan).expect("the scan is there");
    let totals = tree.totals();
    println!(
        "scanned {root}: {} s, {} files, {} folders, {} bytes, {} errors",
        began.elapsed().as_secs_f64(),
        totals.files,
        totals.dirs,
        totals.allocated,
        totals.errors,
    );

    let began = Instant::now();
    state.advisor.load().expect("the model loads");
    println!(
        "model loaded in {:.1} s on {:?}; model memory {:?} bytes (its helper's, when it runs in one); this process {:?} bytes; video memory used {:?} MiB",
        began.elapsed().as_secs_f64(),
        state.advisor.status().device,
        state.advisor.status().ram_bytes,
        platter_platform::resident_bytes(),
        video_memory_used()
    );

    let mut stage_a = Vec::new();
    let mut total = Vec::new();

    for question in questions() {
        let began = Instant::now();
        let read = state.advisor.interpret(&question);
        let stage_a_time = began.elapsed();

        let began = Instant::now();
        let answered = block_on(answer_scan(&state, scan, question.clone(), SizeUnitsDto::Gb));
        let total_time = began.elapsed();

        stage_a.push(stage_a_time);
        total.push(total_time);

        println!("\n=== {question}");
        println!(
            "stage A {:.1} s, total {:.1} s",
            stage_a_time.as_secs_f64(),
            total_time.as_secs_f64()
        );
        let line = match answered {
            Ok(answer) => {
                let understood = serde_json::to_value(&answer.understood).unwrap();
                println!("understood: {understood}");
                println!("answer: {}", answer.answer);
                for row in &answer.rows {
                    println!(
                        "  row {} | {} B | {:?} | {:?}",
                        row.path, row.allocated, row.verdict, row.source
                    );
                }
                for note in &answer.notes {
                    println!("  note: {note}");
                }

                json!({
                    "question": question,
                    "stage_a_ms": ms(stage_a_time),
                    "total_ms": ms(total_time),
                    "first_reading": read.as_ref().ok().map(|query| format!("{query:?}")),
                    "understood": understood,
                    "answer": answer.answer,
                    "segments": answer.segments,
                    "rows": answer.rows,
                    "more": answer.more,
                    "notes": answer.notes,
                })
            }
            Err(error) => {
                println!("refused: {error:?}");

                json!({
                    "question": question,
                    "stage_a_ms": ms(stage_a_time),
                    "total_ms": ms(total_time),
                    "error": format!("{error:?}"),
                })
            }
        };

        if let Some(out) = &mut out {
            writeln!(out, "{line}").expect("PLATTER_OUT can be written");
        }
    }

    if std::env::var("PLATTER_EVAL").map_or(true, |value| value != "0") {
        score_evaluation_set(&state, &mut out);
    }

    for (name, times) in [("stage A", &mut stage_a), ("total", &mut total)] {
        times.sort();
        let at = |share: usize| times[(times.len() * share / 100).min(times.len() - 1)];
        println!(
            "\n{name}: p50 {:.1} s, p95 {:.1} s, max {:.1} s",
            at(50).as_secs_f64(),
            at(95).as_secs_f64(),
            times.last().map_or(0.0, |time| time.as_secs_f64()),
        );
    }

    state.advisor.shutdown();
}
