//! The helper process's side: serves an [`Engine`] over the connection the app handed it (a socket on Unix, the
//! standard input and output pipes on Windows; see [`crate::protocol`] for what is said, and [`crate::process`]
//! for the app's side).
//!
//! One thread reads requests into a queue; the thread that called [`serve`] owns the engine and works
//! through the queue, one request at a time. The reading thread also acts on a `Cancel` at once, by raising
//! the flag the running `Judge` looks at, and raises the same flag when the app's end of the connection closes,
//! so a helper whose app has gone stops what it is doing and exits.

use std::fs::File;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::engine::{AiError, Engine, Placement};
use crate::protocol::{
    CARD_TEXT_CHARS, LineReader, PROTOCOL_VERSION, Reply, Request, clean_text, write_message,
};
use crate::verify::{VerifiedModel, VerifyError};

/// The helper's end of the connection: a way to read the app's requests and a way to write its replies.
pub trait Connection: Send + 'static {
    /// The reading end and the writing end. Both are used at the same time, from two threads.
    fn split(self) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)>;
}

#[cfg(unix)]
impl Connection for UnixStream {
    fn split(self) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        let reader = self.try_clone()?;

        Ok((Box::new(reader), Box::new(self)))
    }
}

/// Two ends that are separate already: the standard input and output pipes of the helper on Windows.
impl<R: Read + Send + 'static, W: Write + Send + 'static> Connection for (R, W) {
    fn split(self) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        Ok((Box::new(self.0), Box::new(self.1)))
    }
}

/// The exit code of a helper that was told to stop, or whose app went away.
pub const EXIT_CLEAN: i32 = 0;
/// The exit code of a helper whose connection broke.
pub const EXIT_BROKEN: i32 = 1;
/// The least time between two `Progress` lines: an engine that reports every token does not flood the
/// socket.
const PROGRESS_EVERY: Duration = Duration::from_millis(20);

/// Serves `engine` on `end` until the app says `Shutdown` or its end closes; returns the exit code of the
/// process. `model` is the checked file the app handed over (Unix; `None` for a helper that only lists the
/// cards, and on Windows, where a `load` names the path of the model instead) and `bytes` the size the model
/// must have.
pub fn serve<E: Engine>(mut engine: E, end: impl Connection, model: Option<File>, bytes: u64) -> i32 {
    let Ok((reader_end, writer_end)) = end.split() else {
        return EXIT_BROKEN;
    };
    let writer: Writer = Arc::new(Mutex::new(writer_end));
    let running = Arc::new(Running::default());

    let hello = Reply::Hello {
        version: PROTOCOL_VERSION,
        pid: std::process::id(),
    };
    if send(&writer, &hello).is_err() {
        return EXIT_BROKEN;
    }

    let (requests, inbox) = mpsc::channel();
    let reading = Arc::clone(&running);
    thread::spawn(move || read_requests(reader_end, &requests, &reading));

    engine.on_progress(progress_hook(&writer, &running));
    #[cfg(unix)]
    let inherited = model.map(|file| VerifiedModel::inherited(file, bytes));
    #[cfg(not(unix))]
    let inherited: Option<Result<VerifiedModel, VerifyError>> = {
        let _ = model;
        None
    };
    // The model the helper opened itself by the path of a `load` (Windows). Held until the next load or the end,
    // with the share mode that keeps the file as it was checked.
    let mut opened: Option<Result<VerifiedModel, VerifyError>> = None;

    // Whether the engine has a model to free when this ends (a helper that only listed the cards has none).
    let mut loaded = false;

    // The queue runs dry when the reading thread has ended: the app is gone.
    while let Ok(request) = inbox.recv() {
        let reply = match request {
            Request::Shutdown {} => break,
            Request::Gpus {} => Reply::Gpus { cards: engine.gpus() },
            Request::Load {
                threads,
                placement,
                card,
                model: path,
            } => {
                if let Some(path) = path.as_deref() {
                    opened = Some(open_by_path(path, bytes));
                }
                let model = opened.as_ref().or(inherited.as_ref());
                let reply = load(&mut engine, model, threads, placement, card.as_deref());
                loaded = matches!(reply, Reply::Loaded { .. });
                reply
            }
            Request::Judge {
                id,
                prompt,
                grammar,
                max_tokens,
            } => {
                let reply = judge(&mut engine, &running, id, &prompt, &grammar, max_tokens);
                // The next progress line is not about this one, and the app asks again only after it has
                // this reply.
                running.id.store(0, Ordering::SeqCst);
                reply
            }
            // Acted on by the reading thread.
            Request::Cancel { .. } => continue,
        };

        if send(&writer, &reply).is_err() {
            return EXIT_BROKEN;
        }
    }

    if loaded {
        engine.unload();
    }
    EXIT_CLEAN
}

/// The model the `load` names by path, opened by the helper. Only Windows hands a model over that way: the Unix
/// helper has the file it was given as its standard output and refuses a path.
fn open_by_path(path: &str, bytes: u64) -> Result<VerifiedModel, VerifyError> {
    #[cfg(windows)]
    {
        VerifiedModel::opened_by_path(std::path::Path::new(path), bytes)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, bytes);
        Err(VerifyError::Unsupported)
    }
}

/// The `Judge` that runs (0: none) and the flag that stops it.
#[derive(Default)]
struct Running {
    id: AtomicU64,
    cancel: AtomicBool,
}

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

fn read_requests(from: Box<dyn Read + Send>, requests: &Sender<Request>, running: &Running) {
    let mut lines = LineReader::new(from);

    loop {
        match lines.read::<Request>() {
            Ok(Some(Request::Cancel { id })) => {
                if running.id.load(Ordering::SeqCst) == id {
                    running.cancel.store(true, Ordering::SeqCst);
                }
            }
            Ok(Some(request)) => {
                // Set here, not by the thread that runs it, so a `Cancel` right behind its `Judge` finds it.
                if let Request::Judge { id, .. } = &request {
                    running.cancel.store(false, Ordering::SeqCst);
                    running.id.store(*id, Ordering::SeqCst);
                }
                if requests.send(request).is_err() {
                    return;
                }
            }
            // The app is gone, or sent what cannot be read: stop what runs; the queue runs dry as this
            // thread ends.
            Ok(None) | Err(_) => {
                running.cancel.store(true, Ordering::SeqCst);
                return;
            }
        }
    }
}

fn send(writer: &Mutex<Box<dyn Write + Send>>, reply: &Reply) -> io::Result<()> {
    let mut writer = writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    write_message(&mut *writer, reply)
}

/// What the engine calls to say it is working: a `Progress` line, at most one every [`PROGRESS_EVERY`].
fn progress_hook(writer: &Writer, running: &Arc<Running>) -> crate::engine::ProgressHook {
    let (writer, running) = (Arc::clone(writer), Arc::clone(running));
    let last = Mutex::new(None::<Instant>);

    Box::new(move || {
        let mut last = last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.is_some_and(|at| at.elapsed() < PROGRESS_EVERY) {
            return;
        }
        *last = Some(Instant::now());

        let id = running.id.load(Ordering::SeqCst);
        // A write that fails is the app having gone: the reading thread sees that too.
        let _ = send(
            &writer,
            &Reply::Progress {
                id: (id != 0).then_some(id),
            },
        );
    })
}

fn load<E: Engine>(
    engine: &mut E,
    model: Option<&Result<VerifiedModel, VerifyError>>,
    threads: u32,
    placement: Placement,
    card: Option<&str>,
) -> Reply {
    let failed = |text: String| Reply::Error {
        id: None,
        kind: crate::protocol::ErrorKind::Load,
        text,
    };

    let model = match model {
        None => return failed("The helper was given no model.".to_owned()),
        Some(Err(error)) => return failed(error.to_string()),
        Some(Ok(model)) => model,
    };
    if let Placement::Gpu { index } = placement
        && let Some(refusal) = wrong_card(engine, index, card)
    {
        return failed(refusal);
    }

    match engine.load(model, threads, placement) {
        Ok(info) => Reply::Loaded {
            millis: info.millis,
            threads: info.threads,
        },
        Err(error) => failed(error.to_string()),
    }
}

/// Why the card at `index` is not the one the app chose, when it is not: the cards may be listed in
/// another order by another process. The app holds card names cleaned and cut (`CARD_TEXT_CHARS`), so the
/// name found here is compared in that form.
fn wrong_card<E: Engine>(engine: &mut E, index: usize, wanted: Option<&str>) -> Option<String> {
    let found = engine
        .gpus()
        .into_iter()
        .find(|card| card.index == index)
        .map(|card| clean_text(&card.description, CARD_TEXT_CHARS));

    match (found.as_deref(), wanted) {
        (Some(found), Some(wanted)) if found == wanted => None,
        (Some(_), _) => Some("The graphics card is not the one that was chosen.".to_owned()),
        (None, _) => Some("The graphics card is not there.".to_owned()),
    }
}

fn judge<E: Engine>(
    engine: &mut E,
    running: &Running,
    id: u64,
    prompt: &str,
    grammar: &str,
    max_tokens: u32,
) -> Reply {
    match engine.judge(prompt, grammar, max_tokens, &running.cancel) {
        Ok(generation) => Reply::Generation {
            id,
            text: generation.text,
            tokens: generation.tokens,
            // JSON has no number for these.
            tokens_per_s: if generation.tokens_per_s.is_finite() {
                generation.tokens_per_s
            } else {
                0.0
            },
        },
        Err(error) => Reply::Error {
            id: Some(id),
            kind: crate::protocol::ErrorKind::of(&error),
            text: match &error {
                AiError::NotLoaded | AiError::Cancelled => String::new(),
                other => other.to_string(),
            },
        },
    }
}
