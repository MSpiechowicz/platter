//! The local log of free questions: one JSON line per `ask_scan`, in `$XDG_DATA_HOME/platter/
//! advisor-questions.jsonl`, so that how Platter read a question can be analysed later (docs/architecture.md,
//! "Questions about a scan" and "Files on disk").
//!
//! The log is never sent anywhere and the web view never reads it; it can be switched off
//! (`AdvisorOptionsDto::log_questions`) and deleted (`clear_question_log`). It is kept to about 1 MiB: a line
//! that would pass the cap drops the oldest lines, and the file is then written again atomically (a
//! temporary file, mode 0600, synced and renamed over it, like the rules file). The file is only ever opened
//! as a regular file, a link at its name is never followed, and a failure to write is never reported: the
//! question is answered all the same.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use super::jsonl;
use crate::error::CommandResult;

/// The log's file name, in the app's data folder.
pub const FILE_NAME: &str = "advisor-questions.jsonl";
/// The log is cut to this size (about 1 MB) when a line would pass it.
pub const MAX_BYTES: usize = 1 << 20;

/// What the log keeps of one question. Paths are display paths, as in the answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// UTC, to the second: `2026-11-16T21:05:33Z`.
    pub time: String,
    pub scan: u64,
    pub question: String,
    pub polish: bool,
    /// What the words of the question said by themselves.
    pub signals: Value,
    /// `"not asked"`, `"timeout"`, `"unreadable"`, or the model's query as it wrote it.
    pub model: Value,
    /// What the question came to: intent, kind, limit and the scope path, the scope text or the rule tool.
    pub understood: Value,
    /// The question asked back (prompt and choices), when there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clarify: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pick: Option<u8>,
    /// How many rows the answer lists.
    pub rows: usize,
    /// The sentences that are left out of the answer's text.
    pub meta: Vec<String>,
    pub answer: String,
    /// Reading the question (the words and the model), in milliseconds.
    pub stage_a_ms: u64,
    pub total_ms: u64,
}

/// The question log of this user. Without a place for it (no home folder) nothing is written.
pub struct QuestionLog {
    path: Option<PathBuf>,
    /// One writer (or the delete) at a time.
    busy: Mutex<()>,
}

impl QuestionLog {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            busy: Mutex::new(()),
        }
    }

    /// The log in the app's data folder (`$XDG_DATA_HOME/platter/advisor-questions.jsonl`).
    pub fn production() -> Self {
        Self::new(platter_platform::data_dir().map(|dir| dir.join(FILE_NAME)))
    }

    /// Appends `entry` as one line. A failure (no place, a link or a folder at the name, a full disk) is
    /// skipped without a word: the log never fails a question.
    pub fn append(&self, entry: &Entry) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(mut line) = serde_json::to_vec(entry) else {
            return;
        };
        line.push(b'\n');

        let _busy = self.busy.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = jsonl::append_line(path, &line, MAX_BYTES);
    }

    /// Deletes the log. Succeeds when there is none. A link, a folder or anything else that is not a
    /// regular file at its name is left alone and reported (`io`).
    pub fn clear(&self) -> CommandResult<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let _busy = self.busy.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        jsonl::remove_regular(path, "question log")
    }
}

/// Unix seconds as UTC `YYYY-MM-DDThh:mm:ssZ`.
pub fn utc_time(at: i64) -> String {
    let days = at.div_euclid(86_400);
    let rest = at.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The time now, as [`utc_time`].
pub fn now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64);

    utc_time(seconds)
}

/// The date of a day number counted from 1970-01-01 (the civil calendar, proleptic Gregorian).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    (year, month, day)
}

#[cfg(test)]
mod tests;
