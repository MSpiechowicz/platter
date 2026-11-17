//! `ask_advisor`: what Platter says about one folder, from its rules alone (A5.11). There is no model call
//! and no model wording in the answer; the model's reason for a folder it labelled earlier goes to the local
//! question log ([`super::log`]) and nowhere else.
//!
//! The order is the one of [`super::question`]'s rows: the guard, then a rule that covers the folder (the
//! built-in table or the user's own), then a label made earlier in this scan. A folder none of them decides
//! gets "Platter has no rule for it. Deleting it is your call." and, when the model labelled it, which way it
//! marked it. Every answer ends with the folder's size and, where it can be worked out, how much of it is
//! older than a year.

use std::time::Duration;

use platter_ai::{Units, format_size};
use platter_core::NodeId;
use platter_core::dto::{AgeBucket, ScanId, SuggestionSource, Verdict};
use platter_core::query::summary;
use serde_json::{Value, json};

use super::log::{self, Entry};
use super::question::percent;
use super::{AdvisorAnswerDto, AnswerSourceDto, Review, clamp, verdict_of};
use crate::state::LabelSet;

/// What the log calls the question of the row "Ask".
const QUESTION: &str = "ask about folder";

const NO_RULE: &str = "Platter has no rule for it. Deleting it is your call.";
const MARKED_FOR_REVIEW: &str = "The local model marked it for review.";
const MARKED_TO_KEEP: &str = "The local model marked it to keep.";
const LEFT_ALONE: &str = "Left alone by Platter.";

/// The answer, and what stays out of it.
pub struct Answered {
    pub dto: AdvisorAnswerDto,
    /// The reason the local model gave when it labelled the folder: for the log only.
    pub model_reason: Option<String>,
}

/// The answer, in this order: the guard, a rule covering the folder or one above it, a label made earlier
/// in this scan, then "no rule". Walks the folder's subtree for its age: use it off the async runtime.
pub fn answer(review: &Review, set: Option<&LabelSet>, node: NodeId, units: Units) -> Answered {
    let reply =
        |verdict: Verdict, reason: String, source: AnswerSourceDto, who: Option<String>| AdvisorAnswerDto {
            node,
            path: review.display(node),
            verdict,
            reason,
            source,
            detail: detail(review, node, who, units),
        };

    if let Some(why) = review.guard(node) {
        return Answered {
            dto: reply(
                Verdict::Keep,
                format!("{LEFT_ALONE} {why}"),
                AnswerSourceDto::Guard,
                None,
            ),
            model_reason: None,
        };
    }

    if let Some(covering) = review.explainer.covering(node) {
        let (source, who) = match covering.source {
            SuggestionSource::User => (AnswerSourceDto::User, format!("Your rule: {}.", covering.title)),
            _ => (AnswerSourceDto::Rule, format!("Rule: {}.", covering.title)),
        };

        return Answered {
            dto: reply(covering.verdict, covering.reason, source, Some(who)),
            model_reason: None,
        };
    }

    let label = set.and_then(|set| set.labels.iter().find(|label| label.node == node));
    let Some(label) = label else {
        return Answered {
            dto: reply(
                Verdict::Review,
                NO_RULE.to_owned(),
                AnswerSourceDto::Unruled,
                None,
            ),
            model_reason: None,
        };
    };

    let marked = match verdict_of(label.verdict) {
        Verdict::Keep => MARKED_TO_KEEP,
        _ => MARKED_FOR_REVIEW,
    };

    Answered {
        dto: reply(
            verdict_of(label.verdict),
            format!("{NO_RULE} {marked}"),
            AnswerSourceDto::Model,
            None,
        ),
        model_reason: Some(label.reason.clone()).filter(|reason| !reason.trim().is_empty()),
    }
}

/// Who decided (a rule's title), then the folder's size and its share older than a year.
fn detail(review: &Review, node: NodeId, who: Option<String>, units: Units) -> String {
    let mut parts: Vec<String> = who.into_iter().collect();

    // A folder that could not be read has no size to give.
    if review.tree.is_unreadable(node) {
        parts.push("Platter could not read it, so its size is unknown.".to_owned());
        return parts.join(" ");
    }

    let size = review.tree.allocated(node);
    parts.push(format!("{}.", format_size(clamp(size), units)));
    parts.extend(older_than_a_year(review, node, size, units));

    parts.join(" ")
}

/// How much of the folder is older than a year, as the answer to "why is it big?" says it. Nothing when the
/// folder is empty or its ages cannot be told.
fn older_than_a_year(review: &Review, node: NodeId, size: u64, units: Units) -> Option<String> {
    if size == 0 {
        return None;
    }

    let summary = summary(review.tree, node).ok()?;
    let older: u64 = summary
        .by_age
        .iter()
        .filter(|stat| matches!(stat.bucket, AgeBucket::TwoYears | AgeBucket::Older))
        .map(|stat| stat.allocated)
        .sum();
    let share = percent(older, size);
    let bytes = format_size(clamp(older), units);

    Some(match (older, share) {
        (0, _) => "None of it is older than a year.".to_owned(),
        (_, 0) => format!("Less than 1% of it ({bytes}) is older than a year."),
        _ => format!("{share}% of it ({bytes}) is older than a year."),
    })
}

/// The log's line for one folder ask: the path, the verdict and who gave it, the model's reason when the
/// model labelled the folder, and the answer as the web view got it.
pub fn entry(scan: ScanId, answered: &Answered, total: Duration) -> Entry {
    let dto = &answered.dto;
    let millis = u64::try_from(total.as_millis()).unwrap_or(u64::MAX);

    Entry {
        time: log::now(),
        scan,
        question: QUESTION.to_owned(),
        polish: false,
        signals: json!({}),
        model: json!("not asked"),
        understood: json!({
            "intent": "folder",
            "scope": dto.path,
            "verdict": value_of(dto.verdict),
            "source": value_of(dto.source),
        }),
        clarify: None,
        pick: None,
        rows: 1,
        meta: answered
            .model_reason
            .iter()
            .map(|reason| format!("The local model's reason: {reason}"))
            .collect(),
        answer: format!("{} {}", dto.reason, dto.detail),
        stage_a_ms: 0,
        total_ms: millis,
    }
}

/// A unit enum as the word the web view sees (`review`, `unruled`).
fn value_of(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}
