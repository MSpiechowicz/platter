//! What the app and its helper process say to each other: one JSON object a line, over the socket that is
//! the helper's standard input (Unix) or over its standard input and output pipes (Windows).
//!
//! The helper speaks first (`Hello`), then answers requests one at a time. A `Judge` is answered with a
//! `Generation` or an `Error` carrying the same id; while it runs the helper may send `Progress` lines with
//! that id, which tell the app that it has not hung. Every message is a closed set of fields
//! (`deny_unknown_fields`), a line is at most [`MAX_LINE_BYTES`], and the app ignores replies with an id it
//! is no longer waiting for. The text that comes from the helper is cleaned by [`clean_text`] before it is
//! shown anywhere.

use std::io::{self, BufRead, BufReader, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::engine::{AiError, GpuDevice, Placement};

/// Bumped whenever a message changes. The helper is the same binary as the app, so a mismatch means
/// something is badly wrong, not that a version needs support. Version 1 is the Unix protocol (the model is the
/// helper's standard output). Version 2 is the Windows one: the same lines, and `load` names the model's path
/// (see [`Request::Load`]).
pub const PROTOCOL_VERSION: u32 = if cfg!(windows) { 2 } else { 1 };
/// The longest line either side accepts (the newline not counted): a prompt or an answer is a few KiB.
pub const MAX_LINE_BYTES: usize = 1 << 20;

/// What the app asks the helper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Request {
    /// Which graphics cards are there. (A struct variant, so that a field it does not know is refused.)
    Gpus {},
    /// Load the model the helper was given. `card` is the description of the card the app chose, as it saw
    /// it in an earlier `Gpus` answer: the helper refuses when the card at that index is a different one.
    ///
    /// `model` is the absolute path of the model file (Windows only, protocol version 2): the app holds that
    /// file open so that nobody can change or replace it (see `platter_platform::windows::pinned`), and the
    /// helper opens it by that path and loads what it opened. It is left out of the line when there is none, so
    /// that the Unix helper, which is handed the file as its standard output, hears the same line as before and
    /// refuses a path.
    #[serde(rename_all = "camelCase")]
    Load {
        threads: u32,
        placement: Placement,
        card: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Judge {
        id: u64,
        prompt: String,
        grammar: String,
        max_tokens: u32,
    },
    /// Stop the `Judge` with this id. Ignored when that one is not running.
    Cancel { id: u64 },
    /// Free the model and exit.
    Shutdown {},
}

/// What the helper tells the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Reply {
    Hello {
        version: u32,
        pid: u32,
    },
    Gpus {
        cards: Vec<GpuDevice>,
    },
    #[serde(rename_all = "camelCase")]
    Loaded {
        millis: u64,
        threads: u32,
    },
    /// Still working: on a `Judge` (its id), or on a `Load` (no id).
    Progress {
        id: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Generation {
        id: u64,
        text: String,
        tokens: u32,
        tokens_per_s: f64,
    },
    /// A request failed. `id` is that of the `Judge` it answers, none for any other request.
    Error {
        id: Option<u64>,
        kind: ErrorKind,
        text: String,
    },
}

/// Which [`AiError`] a failure was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    Load,
    NotLoaded,
    Cancelled,
    Run,
    Input,
}

impl ErrorKind {
    pub fn of(error: &AiError) -> Self {
        match error {
            AiError::Load(_) => Self::Load,
            AiError::NotLoaded => Self::NotLoaded,
            AiError::Cancelled => Self::Cancelled,
            AiError::Run(_) => Self::Run,
            AiError::Input(_) => Self::Input,
        }
    }

    /// The error back again, with `text` (cleaned) as the text for the user where the error has one.
    pub fn into_error(self, text: &str) -> AiError {
        let text = clean_text(text, MAX_ERROR_CHARS);

        match self {
            Self::Load => AiError::Load(text),
            Self::NotLoaded => AiError::NotLoaded,
            Self::Cancelled => AiError::Cancelled,
            Self::Run => AiError::Run(text),
            Self::Input => AiError::Input(text),
        }
    }
}

/// The most characters of the helper's text that are kept.
pub const MAX_ERROR_CHARS: usize = 300;

/// The most characters of the name and the description of a card. The app keeps a card's text cleaned to this
/// length, and the helper compares the card it is asked to load in the same form.
pub const CARD_TEXT_CHARS: usize = 120;

/// `text` as it may be shown: line breaks and tabs become spaces, other control characters go, the ends are
/// trimmed and at most `max_chars` characters stay (a longer text ends in an ellipsis).
pub fn clean_text(text: &str, max_chars: usize) -> String {
    let spaced: String = text
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    let spaced = spaced.trim();

    if spaced.chars().count() <= max_chars {
        return spaced.to_owned();
    }
    let kept: String = spaced.chars().take(max_chars.saturating_sub(1)).collect();

    format!("{}…", kept.trim_end())
}

/// One message as a line (with its newline). Fails when it would pass [`MAX_LINE_BYTES`].
pub fn encode<T: Serialize>(message: &T) -> io::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    if line.len() > MAX_LINE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the message is too long",
        ));
    }
    line.push(b'\n');

    Ok(line)
}

/// Writes one message as a line, in one call, so that two writers cannot mix their lines.
pub fn write_message<T: Serialize>(mut to: impl Write, message: &T) -> io::Result<()> {
    to.write_all(&encode(message)?)?;
    to.flush()
}

/// Why a line could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("the connection failed: {0}")]
    Io(#[from] io::Error),
    #[error("a line is longer than {MAX_LINE_BYTES} bytes")]
    TooLong,
    #[error("a line is not a message: {0}")]
    Malformed(String),
}

/// Reads messages line by line, never holding more than one line's worth of memory.
pub struct LineReader<R> {
    from: BufReader<R>,
    line: Vec<u8>,
}

impl<R: Read> LineReader<R> {
    pub fn new(from: R) -> Self {
        Self {
            from: BufReader::new(from),
            line: Vec::new(),
        }
    }

    /// The next message; `None` at the end of the stream (a last line without its newline counts: the
    /// writer may have died in the middle of it, which then reads as malformed).
    pub fn read<T: DeserializeOwned>(&mut self) -> Result<Option<T>, ReadError> {
        self.line.clear();

        // One byte more than the limit, so that a line of exactly the limit is told from a longer one.
        let limit = (MAX_LINE_BYTES + 2) as u64;
        let read = (&mut self.from).take(limit).read_until(b'\n', &mut self.line)?;
        if read == 0 {
            return Ok(None);
        }

        let complete = self.line.last() == Some(&b'\n');
        if complete {
            self.line.pop();
        }
        if self.line.len() > MAX_LINE_BYTES {
            return Err(ReadError::TooLong);
        }

        serde_json::from_slice(&self.line)
            .map(Some)
            .map_err(|error| ReadError::Malformed(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(message: T) {
        let line = encode(&message).unwrap();
        assert_eq!(line.iter().filter(|byte| **byte == b'\n').count(), 1, "one line");

        let mut reader = LineReader::new(&line[..]);
        assert_eq!(reader.read::<T>().unwrap(), Some(message));
        assert!(reader.read::<T>().unwrap().is_none());
    }

    #[test]
    fn every_message_survives_a_trip_over_the_wire() {
        round_trip(Request::Gpus {});
        round_trip(Request::Load {
            threads: 8,
            placement: Placement::Gpu { index: 1 },
            card: Some("AMD Radeon RX 7900 XTX (RADV NAVI31)".to_owned()),
            model: None,
        });
        round_trip(Request::Load {
            threads: 2,
            placement: Placement::Cpu,
            card: None,
            model: Some("C:\\Users\\Zoë\\AppData\\Local\\platter\\models\\模型.gguf".to_owned()),
        });
        round_trip(Request::Judge {
            id: 7,
            prompt: "line one\nline \"two\"".to_owned(),
            grammar: "root ::= \"a\"".to_owned(),
            max_tokens: 96,
        });
        round_trip(Request::Cancel { id: 7 });
        round_trip(Request::Shutdown {});

        round_trip(Reply::Hello {
            version: PROTOCOL_VERSION,
            pid: 42,
        });
        round_trip(Reply::Gpus {
            cards: vec![GpuDevice {
                index: 1,
                name: "Vulkan1".to_owned(),
                description: "card".to_owned(),
                total_bytes: 2,
                free_bytes: 1,
                integrated: false,
            }],
        });
        round_trip(Reply::Loaded {
            millis: 5,
            threads: 3,
        });
        round_trip(Reply::Progress { id: Some(7) });
        round_trip(Reply::Progress { id: None });
        round_trip(Reply::Generation {
            id: 7,
            text: "{}".to_owned(),
            tokens: 2,
            tokens_per_s: 31.5,
        });
        round_trip(Reply::Error {
            id: None,
            kind: ErrorKind::NotLoaded,
            text: "no".to_owned(),
        });
    }

    #[test]
    fn a_load_without_a_model_path_is_the_line_it_always_was_and_one_with_a_path_carries_it() {
        let without = Request::Load {
            threads: 2,
            placement: Placement::Cpu,
            card: None,
            model: None,
        };
        let with = Request::Load {
            threads: 2,
            placement: Placement::Cpu,
            card: None,
            model: Some("C:\\m.gguf".to_owned()),
        };

        let line = |request: &Request| String::from_utf8(encode(request).unwrap()).unwrap();

        assert_eq!(
            line(&without),
            "{\"type\":\"load\",\"threads\":2,\"placement\":\"cpu\",\"card\":null}\n"
        );
        assert!(line(&with).contains(r#""model":"C:\\m.gguf""#), "{}", line(&with));
        // A line from the version 1 app has no field for it, and is still read.
        let old = r#"{"type":"load","threads":2,"placement":"cpu","card":null}"#;
        assert_eq!(
            LineReader::new(old.as_bytes()).read::<Request>().unwrap(),
            Some(without)
        );
    }

    #[test]
    fn a_message_with_a_field_it_does_not_know_or_a_type_it_does_not_know_is_refused() {
        for line in [
            r#"{"type":"shutdown","extra":1}"#,
            r#"{"type":"cancel","id":1,"note":"x"}"#,
            r#"{"type":"format"}"#,
            r#"{"id":1}"#,
            r#"{"type":"judge","id":1}"#,
            "not json",
        ] {
            let mut reader = LineReader::new(line.as_bytes());
            assert!(
                matches!(reader.read::<Request>(), Err(ReadError::Malformed(_))),
                "{line:?}"
            );
        }
    }

    #[test]
    fn a_line_of_the_limit_is_read_and_a_longer_one_is_refused_without_reading_it_all() {
        let filler = |size: usize| "x".repeat(size);
        let judge = |size: usize| {
            let message = Request::Judge {
                id: 1,
                prompt: filler(size),
                grammar: String::new(),
                max_tokens: 1,
            };
            serde_json::to_vec(&message).unwrap()
        };
        let overhead = judge(0).len();

        let mut exact = judge(MAX_LINE_BYTES - overhead);
        assert_eq!(exact.len(), MAX_LINE_BYTES);
        exact.push(b'\n');
        assert!(LineReader::new(&exact[..]).read::<Request>().unwrap().is_some());

        let mut over = judge(MAX_LINE_BYTES - overhead + 1);
        over.push(b'\n');
        over.extend_from_slice(b"{\"type\":\"shutdown\"}\n");
        let mut reader = LineReader::new(&over[..]);
        assert!(matches!(reader.read::<Request>(), Err(ReadError::TooLong)));

        // Nothing is made of a message that is too long to send, either.
        let long = Request::Judge {
            id: 1,
            prompt: filler(MAX_LINE_BYTES),
            grammar: String::new(),
            max_tokens: 1,
        };
        assert!(encode(&long).is_err());
    }

    #[test]
    fn several_lines_are_read_in_order_and_a_cut_short_last_line_is_malformed() {
        let wire = b"{\"type\":\"gpus\"}\n{\"type\":\"cancel\",\"id\":3}\n{\"type\":\"shut";
        let mut reader = LineReader::new(&wire[..]);

        assert_eq!(reader.read::<Request>().unwrap(), Some(Request::Gpus {}));
        assert_eq!(reader.read::<Request>().unwrap(), Some(Request::Cancel { id: 3 }));
        assert!(matches!(reader.read::<Request>(), Err(ReadError::Malformed(_))));
    }

    #[test]
    fn text_from_the_helper_is_cleaned_and_cut() {
        assert_eq!(clean_text("a\nb\tc\u{1b}[31md\u{0}", 100), "a b c[31md");
        assert_eq!(clean_text("  padded  ", 100), "padded");
        assert_eq!(clean_text("abcdefghij", 5), "abcd…");
        assert_eq!(clean_text("żółć żółć", 4), "żół…");
        assert_eq!(
            ErrorKind::Run.into_error("bad\u{7}\nthing"),
            AiError::Run("bad thing".to_owned())
        );
    }

    #[test]
    fn an_error_keeps_its_kind_over_the_wire() {
        for error in [
            AiError::Load("a".to_owned()),
            AiError::NotLoaded,
            AiError::Cancelled,
            AiError::Run("b".to_owned()),
            AiError::Input("c".to_owned()),
        ] {
            let text = error.to_string();

            assert_eq!(ErrorKind::of(&error).into_error(&text), error);
        }
    }
}
