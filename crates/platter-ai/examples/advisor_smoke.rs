//! A manual check of the whole chain on a real machine: finds the model in the real data folder,
//! downloads it only when it is missing, verifies it, loads it and asks it about one folder.
//!
//! Run: `cargo run --release -p platter-ai --example advisor_smoke --features llama`.
//!
//! The prompt is the tuned one (`platter_ai::prompt`), so this also shows what the app would ask.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use platter_ai::facts::facts_json;
use platter_ai::grammar::VERDICT_GRAMMAR;
use platter_ai::llama::LlamaEngine;
use platter_ai::prompt::prompt;
use platter_ai::{Engine, FolderFacts, Helpers, MODEL, UreqFetch, download, parse, verify};

/// The folder to judge.
fn folder() -> FolderFacts {
    FolderFacts {
        path: "~/.cache/pipx-like".to_owned(),
        size: 1_503_238_554,
        files: 9031,
        newest_days: Some(40),
        kinds: vec!["other".to_owned()],
        largest_items: ["wheels", "http", "selfcheck"].map(String::from).to_vec(),
        ..FolderFacts::default()
    }
}

fn mib(bytes: Option<u64>) -> String {
    bytes.map_or_else(
        || "unknown".to_owned(),
        |bytes| format!("{:.0} MiB", bytes as f64 / 1_048_576.0),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let models = platter_platform::data_dir()
        .ok_or("no home folder")?
        .join("models");
    let spec = &MODEL;
    let path = models.join(spec.file);

    if !path.exists() {
        println!("{} is missing; downloading {} bytes", path.display(), spec.bytes);
        let started = Instant::now();
        download(
            spec,
            &models,
            Arc::new(UreqFetch::new()),
            &AtomicBool::new(false),
            &|got, total| {
                if got % (64 << 20) < (64 << 10) {
                    eprint!(
                        "\r{:.0} / {:.0} MiB",
                        got as f64 / 1_048_576.0,
                        total as f64 / 1_048_576.0
                    );
                }
            },
            &Helpers::new(),
        )?;
        println!("\ndownloaded in {:.0} s", started.elapsed().as_secs_f64());
    } else {
        println!("using {}", path.display());
    }

    let started = Instant::now();
    let verified = verify(spec, Path::new(&path))?;
    println!(
        "verified size and SHA-256 in {} ms",
        started.elapsed().as_millis()
    );

    let threads = platter_ai::default_threads();
    let before = platter_platform::resident_bytes();
    let mut engine = LlamaEngine::new();
    let info = engine.load(&verified, threads, platter_ai::Placement::Cpu)?;
    println!(
        "loaded in {} ms with {} threads; resident {} -> {}",
        info.millis,
        info.threads,
        mib(before),
        mib(platter_platform::resident_bytes())
    );

    let folder = folder();
    println!("facts: {}", facts_json(&folder));
    let started = Instant::now();
    let answer = engine.judge(&prompt(&folder), VERDICT_GRAMMAR, 96, &AtomicBool::new(false))?;
    let took = started.elapsed();

    let judgement = parse(&answer.text)?;
    println!("raw: {}", answer.text);
    println!("verdict: {:?}", judgement.verdict);
    println!("reason: {}", judgement.reason);
    println!(
        "judged in {:.2} s ({} tokens, {:.1} tokens/s); resident {}",
        took.as_secs_f64(),
        answer.tokens,
        answer.tokens_per_s,
        mib(platter_platform::resident_bytes())
    );

    engine.unload();
    println!("unloaded; resident {}", mib(platter_platform::resident_bytes()));
    Ok(())
}
