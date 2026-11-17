//! The local log of what went wrong with the advisor's helper process or the graphics card: one JSON line per
//! incident and one per answer to the question Platter asks after a graphics card failure, in
//! `$XDG_DATA_HOME/platter/advisor-gpu.log` (docs/architecture.md, "Loading" and "Files on disk").
//!
//! ```text
//! {"type":"incident","id":1763370000123,"at":1763370000123,"time":"2025-11-17T09:00:00Z","device":"gpu",
//!  "card":"AMD Radeon RX 7900 XTX (RADV NAVI31)","what":"crash","exit":{"signal":11},"stderr":"...","detail":"..."}
//! {"type":"answer","id":1763370000123,"time":"2025-11-17T09:00:09Z","choice":"useCpu"}
//! ```
//!
//! The log is never sent anywhere and the web view never reads it; it is only told, in the status, which GPU
//! incident is waiting for an answer (`gpuProblem`) and which was the last (`lastGpuProblem`). A GPU incident
//! with no answer after it is still waiting, also at the next start, when the card is blocked until the user
//! answers. An incident on the processor is logged and never asked about. The file is kept to about 256 KiB
//! (the file handling, with its 0600 mode, its refusal to follow a link and its cap, is [`super::jsonl`]'s); at
//! most twice that is read at start, and a line that cannot be read is skipped. A failure to write is never
//! reported: the incident is held in memory and the question is asked all the same.
//!
//! What the log says now is kept in memory and is read without touching the file: the status asks for it while the
//! advisor's worker holds its own lock, so it must not wait for the disk. A write takes the writers' lock first,
//! then changes the memory under the short lock of the state, and only then writes the file. Lines are written
//! in the order of the changes, and a slow home folder holds up the next write and nothing else.
//!
//! Text that came from the helper is cut before it is written (a card name to 120 characters, the rest to 8192),
//! so a line always fits the cap. What is read back is not trusted either: a card name is cleaned and cut again,
//! and an id or a time that a script could not hold (above 2^53 - 1) is ignored.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use platter_ai::{Exit, GpuIncident, IncidentKind};
use serde_json::{Value, json};

use super::jsonl;
use super::log::utc_time;
use crate::error::{CommandError, CommandResult};

#[cfg_attr(not(all(feature = "advisor", unix)), allow(dead_code))]
/// The log's file name, in the app's data folder.
pub const FILE_NAME: &str = "advisor-gpu.log";
/// The log is cut to this size (about 256 KiB) when a line would pass it.
pub const MAX_BYTES: usize = 256 * 1024;
/// The greatest number the web view can hold exactly (2^53 - 1). A greater id or time is not used.
const MAX_SAFE_ID: u64 = (1 << 53) - 1;
/// A card name is cut to this many characters.
const MAX_CARD_CHARS: usize = 120;
/// The helper's last words and the detail are cut to this many characters before they are written.
const MAX_TEXT_CHARS: usize = 8192;

/// What went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    Crash,
    Hang,
    Error,
}

impl What {
    fn name(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::Hang => "hang",
            Self::Error => "error",
        }
    }

    #[cfg_attr(not(all(feature = "advisor", unix)), allow(dead_code))]
    fn named(name: &str) -> Option<Self> {
        match name {
            "crash" => Some(Self::Crash),
            "hang" => Some(Self::Hang),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

impl From<IncidentKind> for What {
    fn from(kind: IncidentKind) -> Self {
        match kind {
            IncidentKind::Crash => Self::Crash,
            IncidentKind::Hang => Self::Hang,
            IncidentKind::Error => Self::Error,
        }
    }
}

/// What the user chose to do about a graphics card problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Use the processor from now on.
    UseCpu,
    /// Try the card again at the next load.
    TryAgain,
}

impl Choice {
    fn name(self) -> &'static str {
        match self {
            Self::UseCpu => "useCpu",
            Self::TryAgain => "tryAgain",
        }
    }
}

/// A problem on a graphics card, as far as the web view is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Unix milliseconds of the incident, made unique: it names the problem when it is answered.
    pub id: u64,
    /// Unix milliseconds.
    pub at: u64,
    pub card: Option<String>,
    pub what: What,
}

#[derive(Default)]
struct State {
    /// The last GPU incident that has no answer after it.
    pending: Option<Problem>,
    /// The last GPU incident, answered or not.
    last: Option<Problem>,
    /// The greatest id in the log (or made this session): the next is above it.
    newest_id: u64,
}

/// The graphics card problem log of this user. Without a place for it (no home folder) nothing is written.
pub struct GpuLog {
    path: Option<PathBuf>,
    /// What the log says now. Held only for a moment, never over file work.
    state: Mutex<State>,
    /// One writer (or the delete) at a time, so that lines are written in the order of the changes. Always taken
    /// before `state`.
    writers: Mutex<()>,
}

impl GpuLog {
    /// A log at `path` that is not read: it starts empty. (A test that wants the log of a file reads it with
    /// [`GpuLog::open`].)
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            state: Mutex::new(State::default()),
            writers: Mutex::new(()),
        }
    }

    /// The log at `path`, as it is now. A file that is not there, cannot be read or is not a regular file is an
    /// empty log (and is left alone).
    #[cfg_attr(not(all(feature = "advisor", unix)), allow(dead_code))]
    pub fn open(path: PathBuf) -> Self {
        let state = jsonl::read_tail(&path, 2 * MAX_BYTES).map_or_else(
            |_| State::default(),
            |bytes| read(&bytes, latest_id(SystemTime::now())),
        );

        Self {
            path: Some(path),
            state: Mutex::new(state),
            writers: Mutex::new(()),
        }
    }

    /// The log in the app's data folder, as it is now.
    #[cfg_attr(not(all(feature = "advisor", unix)), allow(dead_code))]
    pub fn production() -> Self {
        match platter_platform::data_dir() {
            Some(dir) => Self::open(dir.join(FILE_NAME)),
            None => Self::new(None),
        }
    }

    /// The GPU incident that waits for an answer.
    pub fn pending(&self) -> Option<Problem> {
        self.lock().pending.clone()
    }

    /// The last GPU incident in the log, answered or not.
    pub fn last(&self) -> Option<Problem> {
        self.lock().last.clone()
    }

    /// Writes an incident. One on a graphics card becomes the pending problem (and the last one).
    pub fn record(&self, incident: &GpuIncident) {
        let _writing = self.lock_writers();

        let at = millis(incident.at);
        let card = incident.card.as_deref().and_then(clean_card);
        let id = {
            let mut state = self.lock();
            let id = at.max(state.newest_id.saturating_add(1)).min(MAX_SAFE_ID);
            state.newest_id = id;

            if incident.on_gpu {
                let problem = Problem {
                    id,
                    at,
                    card: card.clone(),
                    what: incident.what.into(),
                };
                state.last = Some(problem.clone());
                state.pending = Some(problem);
            }

            id
        };

        self.append(&incident_line(id, at, card.as_deref(), incident));
    }

    /// Writes the answer to the pending problem `id`. `invalid` when that is not the problem that waits (it
    /// was answered, or a newer one came).
    pub fn answer(&self, id: u64, choice: Choice) -> CommandResult<()> {
        let _writing = self.lock_writers();

        {
            let mut state = self.lock();
            if state.pending.as_ref().map(|problem| problem.id) != Some(id) {
                return Err(CommandError::invalid(
                    "that graphics card problem is not waiting for an answer",
                ));
            }
            state.pending = None;
        }

        self.append(&answer_line(id, choice));
        Ok(())
    }

    /// The user's settings already say to use the processor: a pending problem is answered so, without being
    /// asked about. Returns whether there was one.
    pub fn answer_for_cpu_only(&self) -> bool {
        let Some(problem) = self.pending() else {
            return false;
        };

        self.answer(problem.id, Choice::UseCpu).is_ok()
    }

    /// Deletes the log. Succeeds when there is none. A problem that waits for an answer still waits, and
    /// answering it writes the file again. A link, a folder or anything else that is not a regular file at
    /// its name is left alone and reported (`io`).
    pub fn clear(&self) -> CommandResult<()> {
        let _writing = self.lock_writers();

        if let Some(path) = &self.path {
            jsonl::remove_regular(path, "graphics card problem log")?;
        }
        self.lock().last = None;
        Ok(())
    }

    /// Adds a line. Called with the writers' lock held (and not the state's), so lines are written in order, one
    /// at a time, and the status never waits for the file.
    fn append(&self, line: &[u8]) {
        if let Some(path) = &self.path {
            let _ = jsonl::append_line(path, line, MAX_BYTES);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_writers(&self) -> std::sync::MutexGuard<'_, ()> {
        self.writers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn millis(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// A card name as it is shown and kept: no control characters, at most [`MAX_CARD_CHARS`] characters, no
/// space at either end. `None` when nothing is left.
fn clean_card(card: &str) -> Option<String> {
    let visible: String = card.chars().filter(|c| !c.is_control()).collect();
    let cleaned: String = visible.trim().chars().take(MAX_CARD_CHARS).collect();
    let cleaned = cleaned.trim_end();

    (!cleaned.is_empty()).then(|| cleaned.to_owned())
}

fn cut(text: &str) -> String {
    text.chars().take(MAX_TEXT_CHARS).collect()
}

fn incident_line(id: u64, at: u64, card: Option<&str>, incident: &GpuIncident) -> Vec<u8> {
    let exit = match incident.exit {
        Some(Exit::Code(code)) => json!({ "code": code }),
        Some(Exit::Signal(signal)) => json!({ "signal": signal }),
        None => Value::Null,
    };
    let line = json!({
        "type": "incident",
        "id": id,
        "at": at,
        "time": utc_time((at / 1000) as i64),
        "device": if incident.on_gpu { "gpu" } else { "cpu" },
        "card": card,
        "what": What::from(incident.what).name(),
        "exit": exit,
        "stderr": cut(&incident.stderr_tail),
        "detail": cut(&incident.detail),
    });

    to_line(&line)
}

fn answer_line(id: u64, choice: Choice) -> Vec<u8> {
    let now = millis(SystemTime::now());

    to_line(&json!({
        "type": "answer",
        "id": id,
        "time": utc_time((now / 1000) as i64),
        "choice": choice.name(),
    }))
}

fn to_line(value: &Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(value).unwrap_or_default();
    line.push(b'\n');

    line
}

/// The greatest id a line may have when the log is read at `now`: ids are times in milliseconds, so one more
/// than a year ahead was not written by Platter. Skipping those keeps every new id below [`MAX_SAFE_ID`] and
/// unique.
fn latest_id(now: SystemTime) -> u64 {
    const YEAR_MS: u64 = 366 * 24 * 60 * 60 * 1000;

    millis(now).saturating_add(YEAR_MS).min(MAX_SAFE_ID - 1)
}

/// What the log says: the lines it can read, in order. A line that is not JSON, is of another type, lacks a
/// field or has an id above `latest` is skipped.
#[cfg_attr(not(all(feature = "advisor", unix)), allow(dead_code))]
fn read(bytes: &[u8], latest: u64) -> State {
    let mut state = State::default();

    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(line) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let Some(id) = line["id"].as_u64().filter(|id| *id <= latest) else {
            continue;
        };

        match line["type"].as_str() {
            Some("incident") => {
                let Some(what) = line["what"].as_str().and_then(What::named) else {
                    continue;
                };
                state.newest_id = state.newest_id.max(id);
                if line["device"] != "gpu" {
                    continue;
                }

                let problem = Problem {
                    id,
                    at: line["at"].as_u64().filter(|at| *at <= MAX_SAFE_ID).unwrap_or(id),
                    card: line["card"].as_str().and_then(clean_card),
                    what,
                };
                state.last = Some(problem.clone());
                state.pending = Some(problem);
            }
            Some("answer") => {
                state.newest_id = state.newest_id.max(id);
                if state.pending.as_ref().map(|problem| problem.id) == Some(id) {
                    state.pending = None;
                }
            }
            _ => {}
        }
    }

    state
}

#[cfg(test)]
mod tests;
