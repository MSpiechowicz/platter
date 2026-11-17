//! The model in a helper process: an [`Engine`] that runs the real runtime in another process and talks to it
//! over a socket (see [`crate::protocol`]; the helper's side is [`crate::runtime`]).
//!
//! The helper exists so that what goes wrong in a graphics driver (a crash, a call that never returns) ends
//! a helper and not the app. This side starts one helper per loaded model (and a short-lived one to list the
//! cards), watches it, and turns what it sees into two things: the usual errors ([`AiError`]), so callers
//! that only know errors keep working, and a [`RuntimeIncident`] to be taken with
//! [`Engine::take_incident`] when the process itself failed.
//!
//! What is watched:
//!
//! - A helper that dies (its end of the socket closes): a crash, with how it ended and the last lines it
//!   wrote to its standard error.
//! - On a graphics card, a generation that shows no progress for [`Limits::stall`]: a hang, and the helper is
//!   killed. The helper reports progress at every batch of the prompt and every token, and the load on a card
//!   includes a first run, so compiling the driver's pipelines is covered by the load's own limit.
//! - On the processor there is no stall limit (a slow answer is just slow; the job's own time ends it by
//!   raising the cancel flag), but a helper that has been told to cancel and still does not answer within
//!   [`Limits::cancel_grace`] is killed.
//!
//! Starting a helper is behind [`Spawner`], so tests run the helper's side on a thread (see
//! `testing::ThreadSpawner`) and no test starts a process or touches a graphics card.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use platter_platform::RuntimeMode;

use crate::engine::{
    AiError, Engine, Exit, Generation, GpuDevice, IncidentKind, LoadInfo, Placement, RuntimeIncident,
};
use crate::protocol::{
    ErrorKind, LineReader, MAX_ERROR_CHARS, PROTOCOL_VERSION, Reply, Request, clean_text, write_message,
};
use crate::verify::VerifiedModel;

/// How long the app waits for a helper to do each thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// A helper to start and say hello, and to list the cards.
    pub probe: Duration,
    /// The model to load (on a card, with its first run).
    pub load: Duration,
    /// A question that needs no model work: the list of cards from a helper that is already running.
    pub query: Duration,
    /// A generation on a graphics card may go this long without a sign of progress.
    pub stall: Duration,
    /// A helper on the processor that was told to cancel may take this long to answer.
    pub cancel_grace: Duration,
    /// A helper that was told to stop may take this long to exit.
    pub shutdown: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            probe: Duration::from_secs(15),
            load: Duration::from_secs(120),
            query: Duration::from_secs(5),
            stall: Duration::from_secs(5),
            cancel_grace: Duration::from_secs(30),
            shutdown: Duration::from_secs(2),
        }
    }
}

/// A started helper process, as far as the app needs to know.
pub trait RuntimeProcess: Send {
    /// Its number, as the system knows it.
    fn id(&self) -> u32;

    /// Ends it, without waiting.
    fn kill(&mut self);

    /// Waits up to `timeout` for it to end.
    fn wait_for(&mut self, timeout: Duration) -> Waited;
}

/// Whether a process has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waited {
    Running,
    Exited(Option<Exit>),
}

/// What starting a helper gives.
pub struct Spawned {
    pub process: Box<dyn RuntimeProcess>,
    /// The app's end of the connection to the helper.
    pub socket: UnixStream,
    /// What the helper writes to its standard error.
    pub stderr: Box<dyn Read + Send>,
}

/// Starts helpers. The real one starts a process; tests run the helper's side on a thread.
pub trait Spawner: Send {
    /// Starts a helper for `mode`, handing it `model` (the checked file, none for the probe).
    fn spawn(&mut self, model: Option<File>, mode: RuntimeMode) -> io::Result<Spawned>;
}

/// Starts the helper as a new process of `program` (see [`platter_platform::spawn_advisor_runtime`]).
#[derive(Debug, Clone)]
pub struct RealSpawner {
    pub program: PathBuf,
}

impl RealSpawner {
    /// A spawner that starts this very binary again.
    pub fn current_exe() -> io::Result<Self> {
        Ok(Self {
            program: platter_platform::runtime_program()?,
        })
    }
}

impl Spawner for RealSpawner {
    fn spawn(&mut self, model: Option<File>, mode: RuntimeMode) -> io::Result<Spawned> {
        let child = platter_platform::spawn_advisor_runtime(&self.program, model, mode)?;

        Ok(Spawned {
            process: Box::new(RealProcess { child: child.child }),
            socket: child.socket,
            stderr: Box::new(child.stderr),
        })
    }
}

struct RealProcess {
    child: Child,
}

impl RuntimeProcess for RealProcess {
    fn id(&self) -> u32 {
        self.child.id()
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
    }

    fn wait_for(&mut self, timeout: Duration) -> Waited {
        let deadline = Instant::now() + timeout;

        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return Waited::Exited(platter_platform::describe_exit(&status).map(exit_of));
                }
                // A process that cannot be waited for is as good as gone.
                Err(_) => return Waited::Exited(None),
                Ok(None) => {}
            }
            if Instant::now() >= deadline {
                return Waited::Running;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for RealProcess {
    fn drop(&mut self) {
        // Nothing is left running, and nothing is left to be waited for.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn exit_of(exit: platter_platform::Exit) -> Exit {
    match exit {
        platter_platform::Exit::Code(code) => Exit::Code(code),
        platter_platform::Exit::Signal(signal) => Exit::Signal(signal),
    }
}

/// How long to look at the processes' last words after one has ended.
const STDERR_SETTLE: Duration = Duration::from_millis(300);
/// How long to wait for a process that was killed to be gone.
const KILL_SETTLE: Duration = Duration::from_secs(2);
/// How often a waiting call looks at its cancel flag and its clocks.
const TICK: Duration = Duration::from_millis(10);

/// The model in a helper process.
pub struct ProcessEngine<S: Spawner> {
    spawner: S,
    limits: Limits,
    /// The helper the model is loaded in.
    helper: Option<Helper>,
    /// The cards the last list had, so that a load names the card it was chosen from.
    cards: Vec<GpuDevice>,
    incident: Option<RuntimeIncident>,
    next_id: u64,
}

impl<S: Spawner> ProcessEngine<S> {
    pub fn new(spawner: S) -> Self {
        Self::with_limits(spawner, Limits::default())
    }

    pub fn with_limits(spawner: S, limits: Limits) -> Self {
        Self {
            spawner,
            limits,
            helper: None,
            cards: Vec::new(),
            incident: None,
            next_id: 0,
        }
    }

    /// Starts a helper and waits for it to say hello. A helper that could not start (the system refused
    /// it) is an error with text; one that started and failed is an incident as well.
    fn start(&mut self, model: Option<File>, mode: RuntimeMode) -> Result<Helper, String> {
        let spawned = self
            .spawner
            .spawn(model, mode)
            .map_err(|error| format!("The model runtime could not start: {error}"))?;
        let mut helper = Helper::new(spawned, mode)
            .map_err(|error| format!("The model runtime could not start: {error}"))?;

        match helper.next_reply(self.limits.probe, |_| false) {
            Ok(Reply::Hello { version, .. }) if version == PROTOCOL_VERSION => Ok(helper),
            Ok(other) => {
                let detail = format!("The helper did not start as expected ({}).", describe(&other));
                self.incident = Some(helper.retire(IncidentKind::Crash, &detail));
                Err("The model runtime stopped unexpectedly.".to_owned())
            }
            Err(failure) => {
                self.incident = Some(helper.fail(&failure, "start"));
                Err(failure.sentence())
            }
        }
    }

    /// Lists the cards with a helper of its own, when no model is loaded.
    fn probe(&mut self) -> Vec<GpuDevice> {
        let Ok(mut helper) = self.start(None, RuntimeMode::Probe) else {
            return Vec::new();
        };

        match helper.ask_cards(self.limits.probe) {
            Ok(cards) => {
                self.stop(helper);
                cards
            }
            Err(failure) => {
                self.incident = Some(helper.fail(&failure, "list the graphics cards"));
                Vec::new()
            }
        }
    }

    /// Tells a helper to stop, waits for it to, and kills it when it does not. It is gone, and reaped,
    /// when this returns.
    fn stop(&mut self, mut helper: Helper) {
        if let Waited::Exited(exit) = helper.process.wait_for(Duration::ZERO) {
            // It ended before it was told to: it died while idle.
            self.incident = Some(helper.died(exit, "The helper had ended while it was idle."));
            return;
        }

        let _ = write_message(&helper.socket, &Request::Shutdown {});
        if helper.process.wait_for(self.limits.shutdown) == Waited::Running {
            self.incident = Some(helper.retire(
                IncidentKind::Hang,
                &format!(
                    "The helper did not stop within {} s.",
                    self.limits.shutdown.as_secs()
                ),
            ));
        }
        // Dropped here: its socket closes and its process is reaped.
    }
}

impl<S: Spawner> Engine for ProcessEngine<S> {
    fn gpus(&mut self) -> Vec<GpuDevice> {
        let cards = match self.helper.take() {
            // The helper the model runs in answers.
            Some(mut helper) => match helper.ask_cards(self.limits.query) {
                Ok(cards) => {
                    self.helper = Some(helper);
                    cards
                }
                Err(failure) => {
                    self.incident = Some(helper.fail(&failure, "list the graphics cards"));
                    Vec::new()
                }
            },
            None => self.probe(),
        };

        self.cards.clone_from(&cards);
        cards
    }

    fn load(
        &mut self,
        model: &VerifiedModel,
        threads: u32,
        placement: Placement,
    ) -> Result<LoadInfo, AiError> {
        self.unload();

        let file = model.share().map_err(|error| {
            AiError::Load(format!(
                "The model file could not be handed to the runtime: {error}"
            ))
        })?;
        let mode = match placement {
            Placement::Cpu => RuntimeMode::Cpu,
            Placement::Gpu { .. } => RuntimeMode::Gpu,
        };
        let mut helper = self.start(Some(file), mode).map_err(AiError::Load)?;

        let card = match placement {
            Placement::Cpu => None,
            Placement::Gpu { index } => self
                .cards
                .iter()
                .find(|card| card.index == index)
                .map(|card| card.description.clone()),
        };
        let request = Request::Load {
            threads,
            placement,
            card,
        };
        if write_message(&helper.socket, &request).is_err() {
            self.incident = Some(helper.fail(&Failure::Closed("the helper was gone".to_owned()), "load"));
            return Err(AiError::Load(Failure::lost()));
        }

        let loaded = helper.next_reply(self.limits.load, |reply| matches!(reply, Reply::Progress { .. }));
        match loaded {
            Ok(Reply::Loaded { millis, threads }) => {
                self.helper = Some(helper);
                Ok(LoadInfo { millis, threads })
            }
            Ok(Reply::Error { kind, text, .. }) => {
                self.stop(helper);
                Err(match kind {
                    ErrorKind::Load | ErrorKind::Run => AiError::Load(clean_text(&text, MAX_ERROR_CHARS)),
                    other => other.into_error(&text),
                })
            }
            Ok(other) => {
                let detail = format!("The helper answered a load with {}.", describe(&other));
                self.incident = Some(helper.retire(IncidentKind::Crash, &detail));
                Err(AiError::Load(Failure::lost()))
            }
            Err(failure) => {
                self.incident = Some(helper.fail(&failure, "load the model"));
                Err(AiError::Load(failure.sentence()))
            }
        }
    }

    fn unload(&mut self) {
        if let Some(helper) = self.helper.take() {
            self.stop(helper);
        }
    }

    fn judge(
        &mut self,
        prompt: &str,
        grammar: &str,
        max_tokens: u32,
        cancel: &AtomicBool,
    ) -> Result<Generation, AiError> {
        let Some(mut helper) = self.helper.take() else {
            return Err(AiError::NotLoaded);
        };

        self.next_id += 1;
        let id = self.next_id;
        let request = Request::Judge {
            id,
            prompt: prompt.to_owned(),
            grammar: grammar.to_owned(),
            max_tokens,
        };
        let sent = write_message(&helper.socket, &request);

        let result = match sent {
            Ok(()) => self.wait_for_generation(&mut helper, id, cancel),
            Err(_) => Err(Failure::Closed("the helper was gone".to_owned())),
        };

        match result {
            Ok(done) => {
                self.helper = Some(helper);
                done
            }
            Err(failure) => {
                self.incident = Some(helper.fail(&failure, "answer"));
                Err(AiError::Run(failure.sentence()))
            }
        }
    }

    fn take_incident(&mut self) -> Option<RuntimeIncident> {
        self.incident.take()
    }

    fn process_id(&self) -> Option<u32> {
        self.helper.as_ref().map(|helper| helper.process.id())
    }
}

impl<S: Spawner> ProcessEngine<S> {
    /// Waits for the answer to the `Judge` with `id`: `Ok` for what the helper said (an answer or a
    /// failure it reported), `Err` when the helper itself failed (it died, or hung).
    fn wait_for_generation(
        &self,
        helper: &mut Helper,
        id: u64,
        cancel: &AtomicBool,
    ) -> Result<Result<Generation, AiError>, Failure> {
        let on_card = helper.mode == RuntimeMode::Gpu;
        let mut last_progress = Instant::now();
        let mut cancelled_at: Option<Instant> = None;

        loop {
            if cancelled_at.is_none() && cancel.load(Ordering::Relaxed) {
                // A write that fails is a helper that is gone, and the wait below finds that out.
                let _ = write_message(&helper.socket, &Request::Cancel { id });
                cancelled_at = Some(Instant::now());
            }

            match helper.events.recv_timeout(TICK) {
                Ok(Event::Reply(Reply::Generation {
                    id: of,
                    text,
                    tokens,
                    tokens_per_s,
                })) if of == id => {
                    return Ok(Ok(Generation {
                        text,
                        tokens,
                        tokens_per_s,
                    }));
                }
                Ok(Event::Reply(Reply::Error {
                    id: Some(of),
                    kind,
                    text,
                })) if of == id => {
                    return Ok(Err(kind.into_error(&text)));
                }
                Ok(Event::Reply(Reply::Progress { id: Some(of) })) if of == id => {
                    last_progress = Instant::now()
                }
                // Anything else is not about this call: an answer to an earlier one, a stray line.
                Ok(Event::Reply(_)) => {}
                Ok(Event::Closed(why)) => return Err(Failure::Closed(why)),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Failure::Closed("the connection to the helper ended".to_owned()));
                }
            }

            if on_card && last_progress.elapsed() >= self.limits.stall {
                return Err(Failure::Stalled(self.limits.stall));
            }
            if !on_card
                && let Some(at) = cancelled_at
                && at.elapsed() >= self.limits.cancel_grace
            {
                return Err(Failure::NotCancelled(self.limits.cancel_grace));
            }
        }
    }
}

impl<S: Spawner> Drop for ProcessEngine<S> {
    fn drop(&mut self) {
        self.unload();
    }
}

/// What the helper did wrong, as the app saw it.
enum Failure {
    /// Its end of the connection closed, or what it sent could not be read.
    Closed(String),
    /// No sign of progress on a card.
    Stalled(Duration),
    /// No answer to a request.
    TimedOut(Duration),
    /// Told to cancel, and no answer.
    NotCancelled(Duration),
}

impl Failure {
    fn kind(&self) -> IncidentKind {
        match self {
            Self::Closed(_) => IncidentKind::Crash,
            Self::Stalled(_) | Self::TimedOut(_) | Self::NotCancelled(_) => IncidentKind::Hang,
        }
    }

    /// What the call that was running fails with, in words for the user.
    fn sentence(&self) -> String {
        match self.kind() {
            IncidentKind::Hang => "The model stopped responding.".to_owned(),
            IncidentKind::Crash | IncidentKind::Error => Self::lost(),
        }
    }

    fn lost() -> String {
        "The model runtime stopped unexpectedly.".to_owned()
    }

    fn detail(&self, doing: &str) -> String {
        match self {
            Self::Closed(why) => format!("The helper ended while it was asked to {doing}: {why}."),
            Self::Stalled(after) => {
                format!(
                    "No sign of progress for {} s while it was asked to {doing}.",
                    after.as_secs_f64()
                )
            }
            Self::TimedOut(after) => format!(
                "No answer in {} s when it was asked to {doing}.",
                after.as_secs_f64()
            ),
            Self::NotCancelled(after) => {
                format!(
                    "It did not stop within {} s of being told to.",
                    after.as_secs_f64()
                )
            }
        }
    }
}

/// A helper, from the app's side.
struct Helper {
    process: Box<dyn RuntimeProcess>,
    socket: UnixStream,
    events: Receiver<Event>,
    stderr: Arc<StderrTail>,
    stderr_ended: Receiver<()>,
    mode: RuntimeMode,
}

enum Event {
    Reply(Reply),
    /// The connection ended or what came over it was not a reply; the text says which.
    Closed(String),
}

impl Helper {
    fn new(spawned: Spawned, mode: RuntimeMode) -> io::Result<Self> {
        let Spawned {
            process,
            socket,
            stderr,
        } = spawned;

        let (events, replies) = mpsc::channel();
        let mut lines = LineReader::new(socket.try_clone()?);
        thread::Builder::new()
            .name("platter-runtime-reader".to_owned())
            .spawn(move || {
                loop {
                    let event = match lines.read::<Reply>() {
                        Ok(Some(reply)) => Event::Reply(reply),
                        Ok(None) => Event::Closed("the helper closed the connection".to_owned()),
                        Err(error) => Event::Closed(error.to_string()),
                    };
                    let last = matches!(event, Event::Closed(_));
                    if events.send(event).is_err() || last {
                        return;
                    }
                }
            })?;

        let tail = Arc::new(StderrTail::default());
        let (ended, stderr_ended) = mpsc::channel();
        let draining = Arc::clone(&tail);
        thread::Builder::new()
            .name("platter-runtime-stderr".to_owned())
            .spawn(move || {
                draining.drain(stderr);
                let _ = ended.send(());
            })?;

        Ok(Self {
            process,
            socket,
            events: replies,
            stderr: tail,
            stderr_ended,
            mode,
        })
    }

    /// The next reply that is not one `skip` says to pass over, within `within`.
    fn next_reply(&mut self, within: Duration, skip: impl Fn(&Reply) -> bool) -> Result<Reply, Failure> {
        let deadline = Instant::now() + within;

        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(Event::Reply(reply)) if skip(&reply) => {}
                Ok(Event::Reply(reply)) => return Ok(reply),
                Ok(Event::Closed(why)) => return Err(Failure::Closed(why)),
                Err(RecvTimeoutError::Timeout) => return Err(Failure::TimedOut(within)),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Failure::Closed("the connection to the helper ended".to_owned()));
                }
            }
        }
    }

    /// Asks the helper which graphics cards there are.
    fn ask_cards(&mut self, within: Duration) -> Result<Vec<GpuDevice>, Failure> {
        write_message(&self.socket, &Request::Gpus {})
            .map_err(|_| Failure::Closed("the helper was gone".to_owned()))?;

        match self.next_reply(within, |reply| matches!(reply, Reply::Progress { .. }))? {
            Reply::Gpus { cards } => Ok(cards),
            other => Err(Failure::Closed(format!(
                "it answered the list of cards with {}",
                describe(&other)
            ))),
        }
    }

    /// The incident of a `failure` while the helper was asked to `doing`; a helper that hung is killed.
    fn fail(self, failure: &Failure, doing: &str) -> RuntimeIncident {
        self.retire(failure.kind(), &failure.detail(doing))
    }

    /// Ends the helper (kills it when it still runs) and says what went wrong.
    fn retire(mut self, what: IncidentKind, detail: &str) -> RuntimeIncident {
        self.process.kill();
        let _ = self.socket.shutdown(std::net::Shutdown::Both);

        let exit = match self.process.wait_for(KILL_SETTLE) {
            Waited::Exited(exit) => exit,
            Waited::Running => None,
        };

        self.incident(what, exit, detail)
    }

    /// The incident of a helper that has already ended.
    fn died(self, exit: Option<Exit>, detail: &str) -> RuntimeIncident {
        self.incident(IncidentKind::Crash, exit, detail)
    }

    fn incident(self, what: IncidentKind, exit: Option<Exit>, detail: &str) -> RuntimeIncident {
        // The last words come in a moment after the end.
        let _ = self.stderr_ended.recv_timeout(STDERR_SETTLE);

        RuntimeIncident {
            what,
            exit,
            stderr_tail: self.stderr.text(),
            detail: clean_text(detail, MAX_ERROR_CHARS),
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        // Whatever way a helper is let go of, nothing of it is left running.
        self.process.kill();
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        let _ = self.process.wait_for(KILL_SETTLE);
    }
}

fn describe(reply: &Reply) -> &'static str {
    match reply {
        Reply::Hello { .. } => "a hello",
        Reply::Gpus { .. } => "a list of cards",
        Reply::Loaded { .. } => "a loaded notice",
        Reply::Progress { .. } => "a progress notice",
        Reply::Generation { .. } => "an answer",
        Reply::Error { .. } => "an error",
    }
}

/// The last lines a helper wrote to its standard error: at most [`TAIL_LINES`] lines and [`TAIL_BYTES`]
/// bytes, each line cleaned of control characters and cut at [`LINE_CHARS`] characters. The helper is
/// not trusted to write anything sensible, and nothing here grows without a limit.
#[derive(Default)]
struct StderrTail {
    lines: Mutex<VecDeque<String>>,
}

const TAIL_LINES: usize = 20;
const TAIL_BYTES: usize = 2048;
const LINE_CHARS: usize = 200;
/// Of a line longer than this the rest is not even kept in memory.
const LINE_READ_BYTES: usize = 1024;

impl StderrTail {
    fn drain(&self, mut from: impl Read) {
        let mut chunk = [0u8; 1024];
        let mut line = Vec::new();

        loop {
            let read = match from.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            for byte in &chunk[..read] {
                if *byte == b'\n' {
                    self.push(&line);
                    line.clear();
                } else if line.len() < LINE_READ_BYTES {
                    line.push(*byte);
                }
            }
        }
        if !line.is_empty() {
            self.push(&line);
        }
    }

    fn push(&self, line: &[u8]) {
        let line = clean_text(&String::from_utf8_lossy(line), LINE_CHARS);
        if line.is_empty() {
            return;
        }

        let mut lines = self.lines.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        lines.push_back(line);
        while lines.len() > TAIL_LINES || lines.iter().map(|line| line.len() + 1).sum::<usize>() > TAIL_BYTES
        {
            lines.pop_front();
        }
    }

    fn text(&self) -> String {
        let lines = self.lines.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        lines.iter().map(String::as_str).collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_keeps_the_last_lines_within_its_caps_and_cleaned() {
        let tail = StderrTail::default();
        let mut text = String::new();
        for n in 0..50 {
            text.push_str(&format!("line {n}\u{1b}[0m\n"));
        }

        tail.drain(text.as_bytes());

        let kept = tail.text();
        let lines: Vec<_> = kept.lines().collect();
        assert_eq!(lines.len(), TAIL_LINES);
        assert_eq!(lines.last(), Some(&"line 49[0m"));
        assert!(!kept.contains('\u{1b}'));
    }

    #[test]
    fn a_flood_of_long_lines_is_held_to_the_byte_cap_and_one_endless_line_is_cut() {
        let tail = StderrTail::default();
        let mut text = "y".repeat(10_000_000);
        text.push('\n');
        for _ in 0..40 {
            text.push_str(&"x".repeat(900));
            text.push('\n');
        }

        tail.drain(text.as_bytes());

        let kept = tail.text();
        assert!(kept.len() <= TAIL_BYTES, "{}", kept.len());
        assert!(kept.lines().all(|line| line.chars().count() <= LINE_CHARS));
        assert!(!kept.contains('y'), "the endless line was long gone");
    }

    #[test]
    fn a_last_line_without_a_newline_is_kept() {
        let tail = StderrTail::default();

        tail.drain(&b"first\nlast words"[..]);

        assert_eq!(tail.text(), "first\nlast words");
    }
}
