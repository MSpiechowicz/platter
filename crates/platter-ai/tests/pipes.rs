//! The model in a helper process over pipes: the way the helper on Windows is reached (its standard input and
//! output), tested on every system with a helper in a thread and real pipes between it and the app. What is
//! checked is what a caller sees: the same answers, errors and incidents as over a socket, a write that stalls
//! (a helper that does not read) ending the helper, and a closed connection ending it.

#![cfg(any(unix, windows))]

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use platter_ai::protocol::{LineReader, PROTOCOL_VERSION, Reply, Request, write_message};
use platter_ai::testing::{
    FakeControl, FakeEngine, Fault, Phase, SpawnerControl, ThreadSpawner, card, tiny_spec,
};
use platter_ai::{
    AiError, Engine, Exit, IncidentKind, Limits, PipeWire, Placement, ProcessEngine, RuntimeProcess, Spawned,
    Spawner, VerifiedModel, Waited, Wire, verify,
};
use platter_platform::RuntimeMode;

const XTX: &str = "AMD Radeon RX 7900 XTX (RADV NAVI31)";

fn content() -> Vec<u8> {
    (0..300u32).map(|n| (n * 7 % 251) as u8).collect()
}

fn model() -> (tempfile::TempDir, VerifiedModel) {
    let dir = tempfile::tempdir().unwrap();
    let spec = tiny_spec(&content());
    let path = dir.path().join(spec.file);
    std::fs::write(&path, content()).unwrap();

    (dir, verify(&spec, &path).unwrap())
}

fn quick() -> Limits {
    Limits {
        probe: Duration::from_secs(5),
        load: Duration::from_secs(5),
        query: Duration::from_secs(2),
        stall: Duration::from_millis(150),
        cancel_grace: Duration::from_millis(200),
        shutdown: Duration::from_millis(500),
        write: Duration::from_secs(2),
        kill: Duration::from_secs(2),
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    model: VerifiedModel,
    engine: ProcessEngine<ThreadSpawner>,
    control: FakeControl,
    spawner: SpawnerControl,
}

fn rig() -> Rig {
    let (dir, model) = model();
    let (fake, control) = FakeEngine::new();
    drop(fake);
    let (spawner, remote) = ThreadSpawner::over_pipes(&control);
    control.set_reply("answer");
    control.set_gpus(vec![card(1, XTX, 23_000, false)]);

    Rig {
        _dir: dir,
        model,
        engine: ProcessEngine::with_limits(spawner.expecting(content().len() as u64), quick()),
        control,
        spawner: remote,
    }
}

impl Rig {
    fn load(&mut self, placement: Placement) {
        self.engine.load(&self.model, 2, placement).unwrap();
    }

    fn judge(&mut self) -> Result<String, AiError> {
        self.engine
            .judge("prompt", "root ::= x", 8, &AtomicBool::new(false))
            .map(|generation| generation.text)
    }
}

#[test]
fn the_cards_are_listed_a_model_is_loaded_and_answers_and_the_helper_is_gone_after_the_unload() {
    let mut rig = rig();

    let cards = rig.engine.gpus();
    rig.load(Placement::Cpu);
    let answer = rig.judge();
    let pid = rig.engine.process_id();

    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].description, XTX);
    assert_eq!(answer.unwrap(), "answer");
    assert!(pid.is_some());
    assert_eq!(rig.control.calls(), ["load 2", "judge"]);
    assert_eq!(rig.spawner.spawned(), [RuntimeMode::Probe, RuntimeMode::Cpu]);

    rig.engine.unload();

    assert_eq!(rig.engine.process_id(), None);
    assert_eq!(rig.spawner.running(), 0, "closing the pipes ended the helper");
    assert_eq!(rig.engine.take_incident(), None);
    assert_eq!(rig.judge(), Err(AiError::NotLoaded));
}

#[test]
fn an_error_the_model_returns_keeps_its_kind_over_pipes_and_the_helper_goes_on() {
    let mut rig = rig();
    rig.load(Placement::Cpu);
    rig.control.fail_next_input("too long");

    let failed = rig.judge();
    let next = rig.judge();

    assert_eq!(failed, Err(AiError::Input("too long".to_owned())));
    assert_eq!(next.unwrap(), "answer");
    assert_eq!(rig.engine.take_incident(), None);
}

#[test]
fn a_helper_that_crashes_while_answering_is_an_incident_with_its_exit_and_last_words() {
    let mut rig = rig();
    rig.load(Placement::Cpu);
    rig.spawner.fault(
        RuntimeMode::Cpu,
        Phase::Judge,
        Fault::Crash {
            exit: Exit::Code(-1073741819),
            stderr: vec!["fatal: out of memory".to_owned()],
        },
    );

    let answered = rig.judge();
    let incident = rig.engine.take_incident().expect("an incident");

    assert!(matches!(answered, Err(AiError::Run(_))), "{answered:?}");
    assert_eq!(incident.what, IncidentKind::Crash);
    assert_eq!(incident.exit, Some(Exit::Code(-1073741819)));
    assert!(incident.stderr_tail.contains("out of memory"), "{incident:?}");
    assert_eq!(rig.engine.process_id(), None, "no model is left");
    assert_eq!(rig.spawner.running(), 0);
}

#[test]
fn a_helper_that_hangs_on_a_card_is_killed_and_the_connection_closed() {
    let mut rig = rig();
    assert_eq!(rig.engine.gpus().len(), 1);
    rig.load(Placement::Gpu { index: 1 });
    rig.spawner.fault(RuntimeMode::Gpu, Phase::Judge, Fault::Hang);
    let began = Instant::now();

    let answered = rig.judge();
    let incident = rig.engine.take_incident().expect("an incident");

    assert!(matches!(answered, Err(AiError::Run(_))), "{answered:?}");
    assert_eq!(incident.what, IncidentKind::Hang);
    assert_eq!(incident.exit, Some(Exit::Signal(9)), "it was killed");
    assert!(began.elapsed() < Duration::from_secs(3), "{:?}", began.elapsed());
    assert_eq!(rig.spawner.running(), 0);
}

#[test]
fn a_helper_that_ignores_a_cancel_is_killed_after_the_grace() {
    let mut rig = rig();
    rig.load(Placement::Cpu);
    rig.spawner.fault(RuntimeMode::Cpu, Phase::Judge, Fault::Hang);
    let cancel = Arc::new(AtomicBool::new(false));
    let raise = Arc::clone(&cancel);
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        raise.store(true, Ordering::SeqCst);
    });

    let answered = rig.engine.judge("prompt", "root ::= x", 8, &cancel);
    let incident = rig.engine.take_incident().expect("an incident");

    assert!(matches!(answered, Err(AiError::Run(_))), "{answered:?}");
    assert_eq!(incident.what, IncidentKind::Hang);
    assert_eq!(rig.spawner.running(), 0);
}

#[test]
fn a_helper_that_cannot_be_started_is_an_error_and_no_incident() {
    let mut rig = rig();
    rig.spawner.refuse_spawns("access is denied");

    let loaded = rig.engine.load(&rig.model, 2, Placement::Cpu);

    assert!(matches!(loaded, Err(AiError::Load(ref text)) if text.contains("access is denied")));
    assert_eq!(rig.engine.take_incident(), None);
}

// ---- A write that does not go through ----

/// A helper that takes the load and then never reads again, on pipes of its own. Ending it (the stop of the
/// wire) closes the pipe it should have read, which is what the end of a real process does: the write that
/// stalled then fails.
struct Quiet {
    killed: Arc<AtomicBool>,
    stops: Arc<AtomicUsize>,
}

struct Alive(Arc<AtomicBool>);

impl RuntimeProcess for Alive {
    fn id(&self) -> u32 {
        7
    }

    fn kill(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn wait_for(&mut self, timeout: Duration) -> Waited {
        let deadline = Instant::now() + timeout;
        while !self.0.load(Ordering::SeqCst) {
            if Instant::now() >= deadline {
                return Waited::Running;
            }
            thread::sleep(Duration::from_millis(2));
        }

        Waited::Exited(Some(Exit::Signal(9)))
    }
}

impl Spawner for Quiet {
    fn spawn(&mut self, _model: Option<std::fs::File>, _mode: RuntimeMode) -> io::Result<Spawned> {
        let (requests_read, requests_write) = io::pipe()?;
        let (replies_read, mut replies_write) = io::pipe()?;
        let unread = Arc::new(Mutex::new(Some(requests_read)));
        let killed = Arc::clone(&self.killed);

        let helper_input = Arc::clone(&unread);
        let helper_killed = Arc::clone(&killed);
        thread::spawn(move || {
            let hello = Reply::Hello {
                version: PROTOCOL_VERSION,
                pid: 7,
            };
            write_message(&mut replies_write, &hello).unwrap();

            // The load, a byte at a time so that nothing more is taken off the pipe than that one line.
            {
                let mut input = helper_input.lock().unwrap();
                let input = input.as_mut().expect("the pipe is open");
                let mut byte = [0u8];
                while input.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {}
            }
            write_message(
                &mut replies_write,
                &Reply::Loaded {
                    millis: 1,
                    threads: 2,
                },
            )
            .unwrap();

            // From here on it reads nothing, and keeps its end of the replies open until it is killed.
            while !helper_killed.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(2));
            }
        });

        let stops = Arc::clone(&self.stops);
        let stop_killed = Arc::clone(&killed);
        let stop = move || {
            stops.fetch_add(1, Ordering::SeqCst);
            stop_killed.store(true, Ordering::SeqCst);
            // The process is gone, so nothing holds its end of the pipe any more.
            *unread.lock().unwrap() = None;
        };

        Ok(Spawned {
            process: Box::new(Alive(killed)),
            wire: Box::new(PipeWire::new(replies_read, requests_write, stop)),
            stderr: Box::new(io::empty()),
        })
    }
}

#[test]
fn a_helper_that_stops_reading_is_a_hang_that_ends_it_when_a_request_cannot_be_written() {
    let killed = Arc::new(AtomicBool::new(false));
    let stops = Arc::new(AtomicUsize::new(0));
    let limits = Limits {
        write: Duration::from_millis(300),
        ..quick()
    };
    let mut engine = ProcessEngine::with_limits(
        Quiet {
            killed: Arc::clone(&killed),
            stops: Arc::clone(&stops),
        },
        limits,
    );
    let (_dir, model) = model();
    engine
        .load(&model, 2, Placement::Cpu)
        .expect("the helper takes the load");
    // Far bigger than any pipe holds, so the write cannot finish by itself.
    let prompt = "x".repeat(900 * 1024);
    let began = Instant::now();

    let answered = engine.judge(&prompt, "root ::= x", 8, &AtomicBool::new(false));
    let incident = engine.take_incident().expect("an incident");

    assert!(matches!(answered, Err(AiError::Run(_))), "{answered:?}");
    assert_eq!(incident.what, IncidentKind::Hang);
    assert!(
        incident.detail.contains("did not take a request"),
        "{}",
        incident.detail
    );
    assert_eq!(
        stops.load(Ordering::SeqCst),
        1,
        "the watchdog ended the helper once"
    );
    assert!(killed.load(Ordering::SeqCst));
    assert!(began.elapsed() < Duration::from_secs(5), "{:?}", began.elapsed());
    assert_eq!(engine.process_id(), None);
}

#[test]
fn a_write_that_stalls_ends_the_helper_and_fails_as_timed_out_while_a_write_that_goes_through_does_not() {
    let (requests_read, requests_write) = io::pipe().unwrap();
    let (replies_read, _replies_write) = io::pipe().unwrap();
    let unread = Arc::new(Mutex::new(Some(requests_read)));
    let stopped = Arc::new(AtomicUsize::new(0));
    let (held, count) = (Arc::clone(&unread), Arc::clone(&stopped));
    let mut wire = PipeWire::new(replies_read, requests_write, move || {
        count.fetch_add(1, Ordering::SeqCst);
        *held.lock().unwrap() = None;
    });
    wire.set_write_timeout(Duration::from_millis(300)).unwrap();

    // A little goes through at once, and the watchdog never fires.
    wire.write_all(b"small\n").unwrap();
    thread::sleep(Duration::from_millis(400));
    assert_eq!(
        stopped.load(Ordering::SeqCst),
        0,
        "a finished write is not answered with a kill"
    );

    // Far more than a pipe holds, with nobody reading: the write stalls, the watchdog ends the helper, and the
    // closed pipe ends the write.
    let began = Instant::now();
    let big = vec![b'x'; 4 * 1024 * 1024];
    let error = wire.write_all(&big).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
    assert_eq!(stopped.load(Ordering::SeqCst), 1);
    assert!(began.elapsed() < Duration::from_secs(5), "{:?}", began.elapsed());
}

#[test]
fn a_write_to_a_pipe_nobody_reads_any_more_fails_as_a_gone_helper_not_a_timeout() {
    let (requests_read, requests_write) = io::pipe().unwrap();
    let (replies_read, _replies_write) = io::pipe().unwrap();
    let mut wire = PipeWire::new(replies_read, requests_write, || {});
    wire.set_write_timeout(Duration::from_secs(5)).unwrap();
    drop(requests_read);

    let error = wire.write_all(b"hello\n").unwrap_err();

    assert_ne!(error.kind(), io::ErrorKind::TimedOut, "{error}");
}

#[test]
fn closing_the_wire_ends_the_helpers_input_and_a_write_after_it_fails() {
    let (mut requests_read, requests_write) = io::pipe().unwrap();
    let (replies_read, _replies_write) = io::pipe().unwrap();
    let mut wire = PipeWire::new(replies_read, requests_write, || {});

    wire.write_all(b"one\n").unwrap();
    wire.close();
    let mut seen = String::new();
    requests_read.read_to_string(&mut seen).unwrap();
    let after = wire.write_all(b"two\n");

    assert_eq!(seen, "one\n", "the helper reads what was sent and then the end");
    assert!(after.is_err());
}

#[test]
fn lines_of_the_protocol_cross_the_pipes_whole_and_in_order() {
    let (requests_read, requests_write) = io::pipe().unwrap();
    let (replies_read, mut replies_write) = io::pipe().unwrap();
    let mut wire = PipeWire::new(replies_read, requests_write, || {});
    let mut incoming = LineReader::new(wire.reader().unwrap());

    let helper = thread::spawn(move || {
        let mut requests = LineReader::new(requests_read);
        while let Some(request) = requests.read::<Request>().unwrap() {
            let reply = match request {
                Request::Judge { id, prompt, .. } => Reply::Generation {
                    id,
                    text: prompt,
                    tokens: 1,
                    tokens_per_s: 1.0,
                },
                _ => continue,
            };
            write_message(&mut replies_write, &reply).unwrap();
        }
    });

    // The replies are read as they come, as the app's reading thread does: a pipe holds little, and a helper
    // that cannot write its reply stops reading the requests.
    let reading = thread::spawn(move || {
        (1..=3u64)
            .map(|_| incoming.read::<Reply>().unwrap().expect("a reply"))
            .collect::<Vec<_>>()
    });

    let prompt = "żółć \"quoted\"\nsecond line 模型".repeat(2000);
    for id in 1..=3u64 {
        let line = platter_ai::protocol::encode(&Request::Judge {
            id,
            prompt: prompt.clone(),
            grammar: String::new(),
            max_tokens: 8,
        })
        .unwrap();
        wire.write_all(&line).unwrap();
    }

    for (id, reply) in (1..=3u64).zip(reading.join().unwrap()) {
        let Reply::Generation { id: of, text, .. } = reply else {
            panic!("a generation comes back");
        };
        assert_eq!((of, text), (id, prompt.clone()));
    }
    wire.close();
    helper.join().unwrap();
}

#[test]
fn a_flush_is_part_of_the_write() {
    // A writer that buffers until flushed must still deliver: `write_all` on the wire flushes.
    struct Buffered {
        pending: Vec<u8>,
        sent: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for Buffered {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.pending.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.sent.lock().unwrap().append(&mut self.pending);
            Ok(())
        }
    }

    let sent = Arc::new(Mutex::new(Vec::new()));
    let mut wire = PipeWire::new(
        io::empty(),
        Buffered {
            pending: Vec::new(),
            sent: Arc::clone(&sent),
        },
        || {},
    );

    wire.write_all(b"line\n").unwrap();

    assert_eq!(*sent.lock().unwrap(), b"line\n");
}
