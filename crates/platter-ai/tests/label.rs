//! The labelling loop with a scripted model: verdicts, skipped folders, cancel, and the two clocks.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use platter_ai::testing::{FakeControl, FakeEngine, tiny_spec};
use platter_ai::{Budget, Engine, FolderFacts, StageLimit, Stopped, label, verify};
use platter_core::{ModelVerdict, NodeId};

const REVIEW: &str = r#"{"verdict":"review","reason":"Cache the system rebuilds."}"#;
const KEEP: &str = r#"{"verdict":"keep","reason":"Personal photos."}"#;

/// A fake model that is loaded, and the remote control of it.
fn loaded() -> (FakeEngine, FakeControl) {
    let content = b"stand-in for the model";
    let dir = tempfile::tempdir().unwrap();
    let spec = tiny_spec(content);
    let path = dir.path().join(spec.file);
    std::fs::write(&path, content).unwrap();

    let (mut engine, control) = FakeEngine::new();
    engine.load(&verify(&spec, &path).unwrap(), 2).unwrap();
    (engine, control)
}

fn items(count: u32) -> Vec<(NodeId, FolderFacts)> {
    (1..=count)
        .map(|n| {
            let facts = FolderFacts {
                path: format!("~/folder{n}"),
                size: 300 << 20,
                ..FolderFacts::default()
            };
            (NodeId(n), facts)
        })
        .collect()
}

fn budget() -> Budget {
    Budget {
        per_folder: Duration::from_secs(10),
        total: Duration::from_secs(60),
        max_tokens: 96,
        ..Budget::default()
    }
}

fn no_progress(_: usize, _: usize) {}

#[test]
fn the_default_limits_are_15_seconds_a_folder_5_minutes_and_96_tokens() {
    assert_eq!(
        Budget::default(),
        Budget {
            per_folder: Duration::from_secs(15),
            total: Duration::from_secs(300),
            max_tokens: 96,
            interpret: StageLimit {
                time: Duration::from_secs(15),
                max_tokens: 64
            },
        }
    );
}

#[test]
fn every_answer_becomes_a_label_and_a_safe_verdict_is_only_review() {
    let (mut engine, control) = loaded();
    control.push_reply(REVIEW);
    control.push_reply(r#"{"verdict":"safe","reason":"Looks disposable."}"#);
    control.push_reply(KEEP);

    let run = label(
        &mut engine,
        &items(3),
        &budget(),
        &AtomicBool::new(false),
        &no_progress,
    );

    assert_eq!(run.stopped, None);
    let seen: Vec<_> = run
        .labels
        .iter()
        .map(|label| (label.node, label.verdict, label.reason.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            (NodeId(1), ModelVerdict::Review, "Cache the system rebuilds."),
            (NodeId(2), ModelVerdict::Review, "Looks disposable."),
            (NodeId(3), ModelVerdict::Keep, "Personal photos."),
        ]
    );
    assert_eq!(run.tokens_per_s, Some(20.0));
}

#[test]
fn each_folder_is_asked_about_with_its_own_facts() {
    let (mut engine, control) = loaded();

    label(
        &mut engine,
        &items(2),
        &budget(),
        &AtomicBool::new(false),
        &no_progress,
    );

    let prompts = control.prompts();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].contains("~/folder1") && !prompts[0].contains("~/folder2"));
    assert!(prompts[1].contains("~/folder2") && !prompts[1].contains("~/folder1"));
}

#[test]
fn a_folder_whose_answer_cannot_be_read_is_skipped_and_progress_still_counts_it() {
    let (mut engine, control) = loaded();
    control.push_reply(REVIEW);
    control.push_reply(r#"{"verdict":"review","reason":"cut off"#);
    control.push_reply(r#"{"verdict":"review","reason":"x","extra":true}"#);
    control.push_reply(KEEP);
    let progress = Mutex::new(Vec::new());

    let run = label(
        &mut engine,
        &items(4),
        &budget(),
        &AtomicBool::new(false),
        &|done, total| {
            progress.lock().unwrap().push((done, total));
        },
    );

    let nodes: Vec<_> = run.labels.iter().map(|label| label.node).collect();
    assert_eq!(
        nodes,
        [NodeId(1), NodeId(4)],
        "nothing is made up for the skipped ones"
    );
    assert_eq!(run.stopped, None);
    assert_eq!(*progress.lock().unwrap(), [(1, 4), (2, 4), (3, 4), (4, 4)]);
}

#[test]
fn a_model_that_fails_skips_the_folders() {
    let (mut engine, _) = FakeEngine::new();

    let run = label(
        &mut engine,
        &items(2),
        &budget(),
        &AtomicBool::new(false),
        &no_progress,
    );

    assert!(run.labels.is_empty());
    assert_eq!(run.stopped, None);
}

#[test]
fn no_folders_is_an_empty_run() {
    let (mut engine, control) = loaded();

    let run = label(&mut engine, &[], &budget(), &AtomicBool::new(false), &no_progress);

    assert!(run.labels.is_empty() && run.stopped.is_none() && run.tokens_per_s.is_none());
    assert_eq!(control.judges(), 0);
}

#[test]
fn a_cancel_between_folders_keeps_the_labels_made_so_far() {
    let (mut engine, control) = loaded();
    control.set_reply(REVIEW);
    let cancel = AtomicBool::new(false);

    let run = label(&mut engine, &items(4), &budget(), &cancel, &|done, _| {
        if done == 2 {
            cancel.store(true, Ordering::SeqCst);
        }
    });

    assert_eq!(run.stopped, Some(Stopped::Cancelled));
    assert_eq!(run.labels.len(), 2);
    assert_eq!(control.judges(), 2, "no question was started after the cancel");
}

#[test]
fn a_cancel_while_the_model_is_writing_stops_it_at_once() {
    let (mut engine, control) = loaded();
    control.push_reply(REVIEW);
    control.push_stall();
    let cancel = AtomicBool::new(false);

    let started = Instant::now();
    let run = thread::scope(|scope| {
        scope.spawn(|| {
            while control.judges() < 2 {
                thread::sleep(Duration::from_millis(1));
            }
            cancel.store(true, Ordering::SeqCst);
        });

        label(&mut engine, &items(3), &budget(), &cancel, &no_progress)
    });

    assert_eq!(run.stopped, Some(Stopped::Cancelled));
    assert_eq!(run.labels.len(), 1);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the cancel waited for the clock"
    );
}

#[test]
fn a_folder_that_takes_longer_than_its_time_is_skipped_and_the_job_goes_on() {
    let (mut engine, control) = loaded();
    control.push_stall();
    control.push_reply(REVIEW);
    control.push_reply(KEEP);
    let limits = Budget {
        per_folder: Duration::from_millis(60),
        ..budget()
    };
    let progress = Mutex::new(Vec::new());

    let started = Instant::now();
    let run = label(
        &mut engine,
        &items(3),
        &limits,
        &AtomicBool::new(false),
        &|done, total| {
            progress.lock().unwrap().push((done, total));
        },
    );

    assert_eq!(run.stopped, None, "one slow folder does not end the job");
    let nodes: Vec<_> = run.labels.iter().map(|label| label.node).collect();
    assert_eq!(nodes, [NodeId(2), NodeId(3)]);
    assert_eq!(*progress.lock().unwrap(), [(1, 3), (2, 3), (3, 3)]);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn the_whole_job_stops_when_its_time_is_used_up_and_keeps_the_labels() {
    let (mut engine, control) = loaded();
    for _ in 0..10 {
        control.push_slow_reply(REVIEW, Duration::from_millis(100));
    }
    let limits = Budget {
        total: Duration::from_millis(250),
        ..budget()
    };

    let started = Instant::now();
    let run = label(
        &mut engine,
        &items(10),
        &limits,
        &AtomicBool::new(false),
        &no_progress,
    );

    assert_eq!(run.stopped, Some(Stopped::TimeBudget));
    assert_eq!(
        run.labels.len(),
        2,
        "the third folder was cut by the end of the job's time"
    );
    assert!(run.ms >= 250 && started.elapsed() < Duration::from_secs(3));
    assert!(control.judges() < 10);
}

#[test]
fn a_job_that_was_cancelled_before_it_began_asks_nothing() {
    let (mut engine, control) = loaded();

    let run = label(
        &mut engine,
        &items(3),
        &budget(),
        &AtomicBool::new(true),
        &no_progress,
    );

    assert_eq!(run.stopped, Some(Stopped::Cancelled));
    assert!(run.labels.is_empty());
    assert_eq!(control.judges(), 0);
}
