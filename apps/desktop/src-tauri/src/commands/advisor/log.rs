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

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::error::{CommandError, CommandResult};

/// The log's file name, in the app's data folder.
pub const FILE_NAME: &str = "advisor-questions.jsonl";
/// The log is cut to this size (about 1 MB) when a line would pass it.
pub const MAX_BYTES: usize = 1 << 20;
/// A file larger than this is not read to be trimmed (something else made it): it is started again.
const MAX_READ_BYTES: u64 = 8 * MAX_BYTES as u64;

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
        let _ = write_line(path, &line);
    }

    /// Deletes the log. Succeeds when there is none. A link, a folder or anything else that is not a
    /// regular file at its name is left alone and reported (`io`).
    pub fn clear(&self) -> CommandResult<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let _busy = self.busy.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.is_file() => match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(CommandError::io(format!(
                    "The question log could not be deleted: {error}"
                ))),
            },
            Ok(_) => Err(CommandError::io(
                "The question log is not a regular file, so Platter left it alone.",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CommandError::io(format!(
                "The question log could not be deleted: {error}"
            ))),
        }
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

/// Adds `line` (ending in a newline) to the file at `path`, making the file and its folder when they are
/// missing, and trimming the oldest lines when the file would pass the cap.
fn write_line(path: &Path, line: &[u8]) -> io::Result<()> {
    if line.len() > MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the line is larger than the log",
        ));
    }
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the log has no folder"))?;
    platter_platform::create_private_dir(dir)?;

    // A link, a folder or any other file that is not regular is refused, before and when it is opened.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the log is not a regular file",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut file = platter_platform::open_private_file(path)?;
    close_to_others(&file)?;

    let size = file.metadata()?.len();
    if size.saturating_add(line.len() as u64) <= MAX_BYTES as u64 {
        return append_to(&mut file, size, line);
    }

    // Over the cap: the newest lines that fit, then the new one, written again as a whole.
    let old = if size <= MAX_READ_BYTES {
        let mut old = Vec::with_capacity(size as usize);
        file.read_to_end(&mut old)?;
        old
    } else {
        Vec::new()
    };
    drop(file);

    replace(path, dir, &trimmed(&old, line))
}

/// Writes `line` at the end of the file (which holds `size` bytes), after a newline when the last line was
/// cut short, so every line of the file stays a line of its own.
fn append_to(file: &mut File, size: u64, line: &[u8]) -> io::Result<()> {
    let mut piece = Vec::with_capacity(line.len() + 1);

    if size > 0 {
        let mut last = [0u8; 1];
        file.seek(SeekFrom::Start(size - 1))?;
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            piece.push(b'\n');
        }
    }
    piece.extend_from_slice(line);

    file.seek(SeekFrom::End(0))?;
    file.write_all(&piece)
}

/// What the file holds after `line` is added to `old` and the oldest lines are dropped until it is no
/// larger than the cap. Starts at the beginning of a line.
fn trimmed(old: &[u8], line: &[u8]) -> Vec<u8> {
    let mut all = old.to_vec();
    if !all.is_empty() && all.last() != Some(&b'\n') {
        all.push(b'\n');
    }
    all.extend_from_slice(line);

    let excess = all.len().saturating_sub(MAX_BYTES);
    if excess == 0 {
        return all;
    }

    // Dropping `excess` bytes may cut a line: go on to the start of the next one.
    let from = all[excess - 1..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(all.len(), |at| excess + at);

    all.split_off(from)
}

/// Writes `bytes` to a temporary file next to `path` (mode 0600), syncs it, renames it over `path` and
/// syncs the folder.
fn replace(path: &Path, dir: &Path, bytes: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let mut name = path.as_os_str().to_owned();
    name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = PathBuf::from(name);

    let written = write_synced(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }

    File::open(dir)?.sync_all()
}

/// Creates `path` (it must not exist, and a link there is never followed), writes `bytes` and syncs.
fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Closes a log that was made open to group or others.
#[cfg(unix)]
fn close_to_others(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if file.metadata()?.permissions().mode() & 0o077 == 0 {
        return Ok(());
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn close_to_others(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
