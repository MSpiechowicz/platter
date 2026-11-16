//! What the question log keeps of one question (see [`super::super::log`]): the words' reading, the model's,
//! what the question came to, and the sentences that are not part of the answer's text.

use std::time::Duration;

use platter_ai::signals;
use serde_json::{Value, json};

use super::reading::{Reading, Subject};
use super::{Collected, ScanIntentDto, ScanKindDto};
use crate::commands::advisor::ModelRead;
use crate::commands::advisor::log::{self, Entry};

/// What is known of a question besides its answer.
pub struct Trace<'a> {
    pub scan: u64,
    /// The question as it was taken (cleaned).
    pub question: &'a str,
    /// The choice of a question asked back that this question made.
    pub pick: Option<u8>,
    /// `None`: the words settled the question and the model was not asked.
    pub model: Option<ModelRead>,
    /// The words and the model, until the question was read.
    pub stage_a: Duration,
}

/// The log's line for a question that was answered.
pub fn entry(
    trace: &Trace<'_>,
    reading: &Reading,
    collected: &Collected,
    answer: &str,
    total: Duration,
) -> Entry {
    Entry {
        time: log::now(),
        scan: trace.scan,
        question: trace.question.to_owned(),
        polish: reading.polish,
        signals: signals_of(trace.question),
        model: model_of(trace.model.as_ref()),
        understood: understood_of(reading, collected),
        clarify: collected.clarify.as_ref().map(|clarify| {
            json!({
                "prompt": clarify.prompt,
                "choices": clarify
                    .choices
                    .iter()
                    .map(|choice| json!({ "label": choice.label, "path": choice.path }))
                    .collect::<Vec<_>>(),
            })
        }),
        pick: trace.pick,
        rows: collected.rows.len(),
        meta: collected.meta.clone(),
        answer: answer.to_owned(),
        stage_a_ms: millis(trace.stage_a),
        total_ms: millis(total),
    }
}

fn millis(time: Duration) -> u64 {
    u64::try_from(time.as_millis()).unwrap_or(u64::MAX)
}

/// What the words of the question say by themselves.
fn signals_of(question: &str) -> Value {
    let signals = signals(question);

    json!({
        "aboutDisk": signals.about_disk,
        "intent": signals.intent.map(|intent| value_of(ScanIntentDto::from(intent))),
        "kind": signals.kind.map(|kind| value_of(ScanKindDto::from(kind))),
        "media": signals.media.map(|media| media.as_str()),
        "roles": signals.roles.iter().map(|role| role.english()).collect::<Vec<_>>(),
        "path": signals.path,
        "hidden": signals.hidden,
        "limit": signals.limit,
        "polish": signals.polish,
        "everything": signals.everything,
        "tokens": signals.tokens,
    })
}

/// What the model wrote, or why there is nothing of it.
fn model_of(model: Option<&ModelRead>) -> Value {
    match model {
        None => json!("not asked"),
        Some(ModelRead::TimedOut) => json!("timeout"),
        Some(ModelRead::Unreadable) => json!("unreadable"),
        Some(ModelRead::Read(query)) => json!({
            "intent": value_of(ScanIntentDto::from(query.intent)),
            "scope": query.scope,
            "kind": value_of(ScanKindDto::from(query.kind)),
            "limit": query.limit,
        }),
    }
}

/// The question as the backend took it: intent, kind, limit and what it names (a folder of the scan, a text
/// that is not in the scan, or the tool of the rules).
fn understood_of(reading: &Reading, collected: &Collected) -> Value {
    let understood = &collected.understood;
    let tool = match &reading.subject {
        Subject::Rules { said, ids } => Some(json!({ "said": said, "rules": ids })),
        _ => None,
    };

    json!({
        "intent": value_of(understood.intent),
        "kind": value_of(understood.kind),
        "limit": understood.limit,
        "scope": understood.scope.as_ref().map(|scope| scope.path.clone()),
        "scopeText": understood.scope_text,
        "tool": tool,
    })
}

/// A unit enum as the word the web view sees (`largest`, `cache`).
fn value_of(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}
