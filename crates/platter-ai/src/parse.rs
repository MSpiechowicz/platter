//! Reads the model's answer. The grammar already forces the shape; this is the second lock: strict JSON
//! with exactly the two fields, a verdict that can only be `review` or `keep`, and a reason that is safe
//! to show as text.

use platter_core::ModelVerdict;
use serde::Deserialize;

use crate::facts::is_invisible;

/// Longest reason kept, in characters.
pub const REASON_CHARS: usize = 200;

/// The model's verdict on one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgement {
    pub verdict: ModelVerdict,
    /// One sentence of plain text: trimmed, without control, bidirectional, line or paragraph separator
    /// characters, at most 200 characters.
    pub reason: String,
}

/// The answer could not be used. The folder is skipped; nothing is assumed in its place.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Not JSON of exactly `{"verdict": text, "reason": text}`, or cut short.
    #[error("The model's answer is not in the expected form: {0}")]
    Malformed(String),
    /// Nothing is left of the reason once it is cleaned.
    #[error("The model's answer has no reason.")]
    EmptyReason,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    verdict: String,
    reason: String,
}

/// Reads one answer. Any verdict other than `keep` (`safe`, an unknown word, another case) becomes
/// `Review`, so the model can never produce more than a request to look at the folder.
pub fn parse(text: &str) -> Result<Judgement, ParseError> {
    let text = text.trim();
    // A derived struct also reads a JSON array of two items; only an object is the answer.
    if !text.starts_with('{') {
        return Err(ParseError::Malformed("not a JSON object".to_owned()));
    }
    let raw: Raw = serde_json::from_str(text).map_err(|error| ParseError::Malformed(error.to_string()))?;

    let verdict = if raw.verdict == "keep" {
        ModelVerdict::Keep
    } else {
        ModelVerdict::Review
    };
    let reason = clean_reason(&raw.reason);
    if reason.is_empty() {
        return Err(ParseError::EmptyReason);
    }

    Ok(Judgement { verdict, reason })
}

fn clean_reason(reason: &str) -> String {
    let visible: String = reason.chars().filter(|c| !is_invisible(*c)).collect();
    let cut: String = visible.trim().chars().take(REASON_CHARS).collect();

    cut.trim_end().to_owned()
}
