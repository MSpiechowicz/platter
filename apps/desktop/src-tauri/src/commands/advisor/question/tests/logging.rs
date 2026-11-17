//! The question log as `ask_scan` writes it: with the reading, the answer and the sentences kept out of the
//! text, on a temporary data folder only.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use platter_core::{ModelLabel, ModelVerdict};

use super::super::super::tests::{World, block_on, node_at, register, world};
use super::super::super::{AdvisorOptionsDto, ask_about};
use super::flow::{APT, QUERY_CLEANABLE, ask, disk, has_home, message_of, options};
use crate::commands::advisor::log::{FILE_NAME, QuestionLog};
use crate::state::LabelSet;

/// A world whose question log is in its own temporary folder.
fn logged(model_on_disk: bool) -> (World, PathBuf) {
    let mut w = world(model_on_disk);
    let path = w._root.path().join("data").join(FILE_NAME);
    w.state.question_log = Arc::new(QuestionLog::new(Some(path.clone())));

    (w, path)
}

fn lines(path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[cfg(target_os = "linux")]
#[test]
fn a_question_the_model_read_is_logged_with_both_readings_and_the_answer() {
    if !has_home() {
        return;
    }
    let (w, path) = logged(true);
    let scan = register(&w.state, &disk());
    w.control.push_reply(QUERY_CLEANABLE);

    let answer = ask(&w, scan, "what can I clean?").unwrap();

    let written = lines(&path);
    assert_eq!(written.len(), 1);
    let line = &written[0];
    assert_eq!(line["scan"], scan);
    assert_eq!(line["question"], "what can I clean?");
    assert_eq!(line["polish"], false);
    assert_eq!(line["answer"], answer.answer);
    assert_eq!(line["rows"], answer.rows.len());
    assert_eq!(line["model"]["intent"], "cleanable");
    assert_eq!(line["understood"]["intent"], "cleanable");
    assert_eq!(line["understood"]["kind"], "any");
    assert_eq!(line["understood"]["limit"], 5);
    assert_eq!(line["signals"]["aboutDisk"], true);
    assert_eq!(line["signals"]["intent"], "cleanable");
    assert_eq!(line["meta"], serde_json::json!([]));
    let time = line["time"].as_str().unwrap();
    assert!(
        time.len() == 20 && time.ends_with('Z') && &time[10..11] == "T",
        "{time}"
    );
    assert!(line["stageAMs"].as_u64().unwrap() <= line["totalMs"].as_u64().unwrap());
    assert!(answer.answer.contains(APT));
}

#[cfg(target_os = "linux")]
#[test]
fn what_the_words_settle_is_logged_as_not_asked_and_what_the_model_cannot_write_as_unreadable() {
    if !has_home() {
        return;
    }
    let (w, path) = logged(true);
    let scan = register(&w.state, &disk());

    ask(&w, scan, "what is cleanable in /var/cache/pacman?").unwrap();
    w.control.push_reply("this is not a query");
    ask(&w, scan, "how is my disk doing?").unwrap();

    let written = lines(&path);
    assert_eq!(written.len(), 2);
    assert_eq!(written[0]["model"], "not asked");
    assert_eq!(written[1]["model"], "unreadable");
    assert_eq!(written[1]["understood"]["intent"], "general");
}

#[cfg(target_os = "linux")]
#[test]
fn the_sentences_kept_out_of_the_answer_are_in_the_log() {
    let (w, path) = logged(true);
    let scan = register(&w.state, &disk());

    let answer = ask(&w, scan, "why is /var/cache/pacman/pkg big?").unwrap();

    let line = &lines(&path)[0];
    let meta: Vec<&str> = line["meta"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| note.as_str().unwrap())
        .collect();
    assert!(
        meta.iter().any(|note| note.contains("covered by the rule")),
        "{meta:?}\n{}",
        answer.answer
    );
    assert!(!answer.answer.contains("covered by"), "{}", answer.answer);
    assert!(answer.notes.iter().all(|note| !note.contains("covered by")));
    assert!(
        line["understood"]["scope"]
            .as_str()
            .unwrap()
            .ends_with("pacman/pkg")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_question_about_a_file_is_logged_with_the_file_as_its_scope() {
    let (w, path) = logged(true);
    let scan = register(&w.state, &disk());

    let answer = ask(&w, scan, "how big is /mnt/photos/img?").unwrap();

    assert_eq!(answer.rows.len(), 1);
    assert!(answer.rows[0].file);
    assert_eq!(
        answer.understood.scope.as_ref().map(|scope| scope.file),
        Some(true)
    );
    let written = lines(&path);
    assert_eq!(written.len(), 1);
    let line = &written[0];
    assert_eq!(line["understood"]["scope"], "/mnt/photos/img");
    assert_eq!(line["understood"]["scopeText"], Value::Null);
    assert_eq!(line["rows"], 1);
    assert_eq!(line["answer"], answer.answer);
    assert_eq!(line["model"], "not asked");
}

#[cfg(target_os = "linux")]
#[test]
fn nothing_is_written_while_the_switch_is_off_and_it_is_read_for_each_question() {
    let (w, path) = logged(true);
    let scan = register(&w.state, &disk());
    let set = |log_questions: bool| {
        w.state
            .advisor
            .configure(AdvisorOptionsDto {
                log_questions,
                ..options(true)
            })
            .unwrap()
    };

    let off = set(false);
    ask(&w, scan, "what is cleanable in /var/cache/pacman?").unwrap();
    assert!(!off.options.log_questions);
    assert!(!path.exists(), "no log, not even an empty file");

    let on = set(true);
    ask(&w, scan, "what is cleanable in /var/cache/pacman?").unwrap();
    assert!(on.options.log_questions);
    assert_eq!(lines(&path).len(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn a_log_that_cannot_be_written_does_not_fail_the_question() {
    let (mut w, _) = logged(true);
    let blocker = w._root.path().join("a-file");
    std::fs::write(&blocker, b"x").unwrap();
    w.state.question_log = Arc::new(QuestionLog::new(Some(blocker.join(FILE_NAME))));
    let scan = register(&w.state, &disk());

    let answer = ask(&w, scan, "what is cleanable in /var/cache/pacman?");

    assert!(answer.is_ok(), "{:?}", answer.map(|answer| answer.answer));
    assert_eq!(std::fs::read(&blocker).unwrap(), b"x");
}

#[cfg(all(target_os = "linux", unix))]
#[test]
fn a_link_where_the_log_belongs_is_not_followed_by_a_question() {
    let (w, path) = logged(true);
    let scan = register(&w.state, &disk());
    let target = w._root.path().join("precious");
    std::fs::write(&target, b"do not touch").unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();

    let answer = ask(&w, scan, "what is cleanable in /var/cache/pacman?");

    assert!(answer.is_ok());
    assert_eq!(std::fs::read(&target).unwrap(), b"do not touch");
}

#[cfg(target_os = "linux")]
#[test]
fn a_refused_question_is_not_logged() {
    let (w, path) = logged(true);
    let scan = register(&w.state, &disk());

    assert_eq!(message_of(ask(&w, scan, " ")), "the question is empty");

    assert!(!path.exists());
}

#[test]
fn clear_question_log_deletes_the_log_and_is_fine_when_there_is_none() {
    let (w, path) = logged(false);
    let clear = || block_on(crate::commands::advisor::clear_question_log_for(&w.state));

    assert!(clear().is_ok(), "no log yet");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{}\n").unwrap();

    assert!(clear().is_ok());
    assert!(!path.exists());
    assert!(clear().is_ok());
}

/// The model's labels of a scan, as a labelling run would have stored them.
fn labelled(w: &World, scan: u64, labels: &[(&str, ModelVerdict, &str)]) {
    let labels = labels
        .iter()
        .map(|(path, verdict, reason)| ModelLabel {
            node: node_at(&w.state, scan, path),
            verdict: *verdict,
            reason: (*reason).to_owned(),
        })
        .collect();

    w.state.scans.set_labels(
        scan,
        LabelSet {
            labels,
            model_ms: 1,
            stopped: None,
        },
    );
}

const PHOTOS_REASON: &str = "Personal photos of the family.";

#[cfg(target_os = "linux")]
#[test]
fn a_folder_ask_is_logged_with_the_models_reason_when_the_switch_is_on_and_not_when_it_is_off() {
    let (w, path) = logged(true);
    let scan = register(&w.state, &disk());
    labelled(&w, scan, &[("/mnt/photos", ModelVerdict::Keep, PHOTOS_REASON)]);
    let photos = node_at(&w.state, scan, "/mnt/photos");
    let set = |log_questions: bool| {
        w.state
            .advisor
            .configure(AdvisorOptionsDto {
                log_questions,
                ..options(true)
            })
            .unwrap()
    };

    set(false);
    block_on(ask_about(&w.state, scan, photos)).unwrap();
    assert!(!path.exists(), "no log, not even an empty file");

    set(true);
    let answer = block_on(ask_about(&w.state, scan, photos)).unwrap();
    assert!(!serde_json::to_string(&answer).unwrap().contains(PHOTOS_REASON));

    let written = lines(&path);
    assert_eq!(written.len(), 1);
    let line = &written[0];
    assert_eq!(line["question"], "ask about folder");
    assert_eq!(line["scan"], scan);
    assert_eq!(line["model"], "not asked");
    assert_eq!(line["understood"]["scope"], "/mnt/photos");
    assert_eq!(line["understood"]["verdict"], "keep");
    assert_eq!(line["understood"]["source"], "model");
    assert_eq!(
        line["meta"],
        serde_json::json!([format!("The local model's reason: {PHOTOS_REASON}")])
    );
    assert!(line["answer"].as_str().unwrap().contains("marked it to keep"));
    assert_eq!(line["rows"], 1);
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_ask_without_a_model_reason_is_logged_with_no_meta() {
    let (w, path) = logged(true);
    let scan = register(&w.state, &disk());

    block_on(ask_about(&w.state, scan, node_at(&w.state, scan, "/mnt/photos"))).unwrap();
    block_on(ask_about(
        &w.state,
        scan,
        node_at(&w.state, scan, "/var/cache/pacman/pkg"),
    ))
    .unwrap();

    let written = lines(&path);
    assert_eq!(written.len(), 2);
    assert_eq!(written[0]["understood"]["source"], "unruled");
    assert_eq!(written[1]["understood"]["source"], "rule");
    assert!(written.iter().all(|line| line["meta"] == serde_json::json!([])));
}

#[cfg(target_os = "linux")]
#[test]
fn what_did_the_model_label_lists_folders_and_verdicts_and_keeps_the_reasons_for_the_log() {
    if !has_home() {
        return;
    }
    let (w, path) = logged(true);
    let scan = register(&w.state, &disk());
    labelled(
        &w,
        scan,
        &[
            ("/mnt/photos", ModelVerdict::Keep, PHOTOS_REASON),
            ("/mnt/cache", ModelVerdict::Review, "Cache nobody needs."),
        ],
    );

    w.control
        .push_reply(r#"{"intent":"labelled","scope":"","kind":"any","limit":5}"#);

    let answer = ask(&w, scan, "what did the model label?").unwrap();

    assert!(
        answer.answer.contains("The model marked “/mnt/photos” (") && answer.answer.contains(") to keep."),
        "{}",
        answer.answer
    );
    let everything = format!(
        "{} {:?} {:?} {:?}",
        answer.answer, answer.segments, answer.rows, answer.notes
    );
    for reason in [PHOTOS_REASON, "Cache nobody needs."] {
        assert!(!everything.contains(reason), "{everything}");
    }
    assert!(
        answer.rows.iter().all(|row| row.note.is_none()),
        "{:?}",
        answer.rows
    );

    let line = &lines(&path)[0];
    let meta = line["meta"].to_string();
    assert!(
        meta.contains(PHOTOS_REASON) && meta.contains("Cache nobody needs."),
        "{meta}"
    );
}
