//! The model in a helper process, without a process: the helper's side runs on a thread (`ThreadSpawner`,
//! with the fake model inside) or is played by a script, so that no test starts a process or touches a
//! graphics card. What is checked is what a caller sees: answers, errors, and the incident of a helper that
//! crashed or hung.

#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use platter_ai::protocol::{LineReader, MAX_LINE_BYTES, PROTOCOL_VERSION, Reply, Request, write_message};
use platter_ai::testing::{
    FakeControl, FakeEngine, Fault, Phase, SpawnerControl, ThreadSpawner, card, tiny_spec,
};
use platter_ai::{
    AiError, Engine, Exit, IncidentKind, Limits, Placement, ProcessEngine, RuntimeProcess, Spawned, Spawner,
    VerifiedModel, Waited, verify,
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
    rig_with(quick())
}

fn rig_with(limits: Limits) -> Rig {
    let (dir, model) = model();
    let (fake, control) = FakeEngine::new();
    drop(fake);
    let (spawner, remote) = ThreadSpawner::new(&control);
    control.set_reply("answer");
    control.set_gpus(vec![card(1, XTX, 23_000, false)]);

    Rig {
        _dir: dir,
        model,
        engine: ProcessEngine::with_limits(spawner, limits),
        control,
        spawner: remote,
    }
}

impl Rig {
    fn load(&mut self, placement: Placement) {
        self.engine.load(&self.model, 2, placement).unwrap();
    }

    fn judge(&mut self) -> Result<String, AiError> {
        self.judge_until(&AtomicBool::new(false))
    }

    fn judge_until(&mut self, cancel: &AtomicBool) -> Result<String, AiError> {
        self.engine
            .judge("prompt", "root ::= x", 8, cancel)
            .map(|generation| generation.text)
    }

    /// A model on the card, listed first as the worker does.
    fn load_on_card(&mut self) {
        assert_eq!(self.engine.gpus().len(), 1);
        self.load(Placement::Gpu { index: 1 });
    }
}

fn raise_after(flag: &'static AtomicBool, after: Duration) {
    thread::spawn(move || {
        thread::sleep(after);
        flag.store(true, Ordering::SeqCst);
    });
}

// ---- A helper in a thread, with the fake model inside ----

#[test]
fn the_cards_are_listed_by_a_helper_of_its_own_that_is_gone_afterwards() {
    let mut rig = rig();

    let cards = rig.engine.gpus();

    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].description, XTX);
    assert_eq!(rig.spawner.spawned(), [RuntimeMode::Probe]);
    assert_eq!(
        rig.spawner.running(),
        0,
        "the probe helper was stopped and reaped"
    );
    assert_eq!(rig.engine.take_incident(), None);
    assert_eq!(rig.engine.process_id(), None);
}

#[test]
fn a_model_is_loaded_in_a_helper_answers_and_the_helper_is_reaped_on_unload() {
    let mut rig = rig();

    rig.load(Placement::Cpu);
    let pid = rig.engine.process_id();

    assert!(pid.is_some(), "the model runs in a process of its own");
    assert_eq!(rig.judge().unwrap(), "answer");
    assert_eq!(rig.control.calls(), ["load 2", "judge"]);
    assert_eq!(rig.spawner.spawned(), [RuntimeMode::Cpu]);

    rig.engine.unload();

    assert_eq!(rig.engine.process_id(), None);
    assert_eq!(rig.spawner.running(), 0);
    assert_eq!(rig.judge(), Err(AiError::NotLoaded));
    rig.engine.unload();
}

#[test]
fn a_model_on_a_card_is_loaded_by_a_helper_for_the_card() {
    let mut rig = rig();

    rig.load_on_card();

    assert_eq!(rig.spawner.spawned(), [RuntimeMode::Probe, RuntimeMode::Gpu]);
    assert_eq!(rig.control.calls(), ["load 2 gpu 1"]);
    assert_eq!(rig.judge().unwrap(), "answer");
}

#[test]
fn a_loaded_helper_lists_the_cards_itself_without_another_helper() {
    let mut rig = rig();
    rig.load_on_card();
    rig.control.set_gpus(vec![card(1, XTX, 20_000, false)]);

    let cards = rig.engine.gpus();

    assert_eq!(cards[0].free_bytes, 20_000 << 20);
    assert_eq!(rig.spawner.spawned().len(), 2, "no third helper");
    assert_eq!(rig.judge().unwrap(), "answer", "the model is still loaded");
}

#[test]
fn a_card_that_is_not_the_one_that_was_listed_is_refused() {
    let mut rig = rig();
    assert_eq!(rig.engine.gpus().len(), 1);
    // Another process lists the cards in another order.
    rig.control
        .set_gpus(vec![card(1, "Some Other Card", 23_000, false)]);

    let loaded = rig.engine.load(&rig.model, 2, Placement::Gpu { index: 1 });

    assert!(matches!(loaded, Err(AiError::Load(_))), "{loaded:?}");
    assert_eq!(rig.engine.take_incident(), None, "an error, not a crash");
    assert_eq!(rig.engine.process_id(), None);
    assert_eq!(rig.spawner.running(), 0);
    assert!(rig.control.calls().iter().all(|call| !call.starts_with("load")));
}

#[test]
fn loading_again_replaces_the_helper() {
    let mut rig = rig();
    rig.load(Placement::Cpu);
    let first = rig.engine.process_id();

    rig.load(Placement::Cpu);

    assert_ne!(rig.engine.process_id(), first);
    assert_eq!(rig.spawner.running(), 1);
}

#[test]
fn dropping_the_engine_ends_its_helper() {
    let mut rig = rig();
    rig.load(Placement::Cpu);

    let Rig { engine, spawner, .. } = rig;
    drop(engine);

    assert_eq!(spawner.running(), 0);
}

#[test]
fn an_error_the_runtime_returns_keeps_its_kind_and_leaves_the_helper_running() {
    let mut rig = rig();
    rig.load_on_card();
    rig.control.fail_gpu_runs("vk::DeviceLostError");

    let failed = rig.judge();

    assert_eq!(failed, Err(AiError::Run("vk::DeviceLostError".to_owned())));
    assert_eq!(
        rig.engine.take_incident(),
        None,
        "the worker makes an error of this"
    );
    assert!(rig.engine.process_id().is_some());

    let mut other = self::rig();
    other.control.fail_gpu_loads("out of device memory");
    other.engine.gpus();
    let load = other.engine.load(&other.model, 2, Placement::Gpu { index: 1 });
    assert_eq!(
        load.unwrap_err(),
        AiError::Load("out of device memory".to_owned())
    );
    assert_eq!(other.spawner.running(), 0, "nothing is left behind a failed load");
}

#[test]
fn text_the_model_cannot_take_is_no_incident_and_the_helper_goes_on() {
    let mut rig = rig();
    rig.load_on_card();
    rig.control.fail_next_input("The text is too long for the model.");

    let first = rig.judge();

    assert_eq!(
        first,
        Err(AiError::Input("The text is too long for the model.".to_owned()))
    );
    assert_eq!(rig.engine.take_incident(), None);
    assert_eq!(rig.judge().unwrap(), "answer");
}

#[test]
fn a_helper_that_cannot_be_started_is_an_error_and_no_incident() {
    let mut rig = rig();
    rig.spawner.refuse_spawns("no more processes");

    let loaded = rig.engine.load(&rig.model, 2, Placement::Cpu);

    assert!(matches!(loaded, Err(AiError::Load(_))));
    assert!(rig.engine.gpus().is_empty());
    assert_eq!(rig.engine.take_incident(), None);
}

#[test]
fn a_cancel_stops_a_running_answer() {
    static CANCEL: AtomicBool = AtomicBool::new(false);
    let mut rig = rig();
    rig.load(Placement::Cpu);
    rig.control.push_slow_reply("late", Duration::from_secs(30));
    raise_after(&CANCEL, Duration::from_millis(50));
    let began = Instant::now();

    let stopped = rig.judge_until(&CANCEL);

    assert_eq!(stopped, Err(AiError::Cancelled));
    assert!(began.elapsed() < Duration::from_secs(3));
    assert_eq!(rig.engine.take_incident(), None);
    assert_eq!(rig.judge().unwrap(), "answer", "the helper is fine");
}

// ---- Crashes ----

fn crash(signal: i32, lines: usize) -> Fault {
    Fault::Crash {
        exit: Exit::Signal(signal),
        stderr: (0..lines).map(|n| format!("driver line {n}\u{1b}[31m")).collect(),
    }
}

#[test]
fn a_crash_while_listing_the_cards_gives_no_cards_and_an_incident_with_the_last_words() {
    let mut rig = rig();
    rig.spawner.fault(RuntimeMode::Probe, Phase::Probe, crash(11, 60));

    let cards = rig.engine.gpus();
    let incident = rig.engine.take_incident().expect("an incident");

    assert!(cards.is_empty());
    assert_eq!(incident.what, IncidentKind::Crash);
    assert_eq!(incident.exit, Some(Exit::Signal(11)));
    let tail: Vec<_> = incident.stderr_tail.lines().collect();
    assert_eq!(tail.len(), 20, "the last twenty lines");
    assert_eq!(
        tail.last(),
        Some(&"driver line 59[31m"),
        "the newest, cleaned of control characters"
    );
    assert!(incident.stderr_tail.len() <= 2048);
    assert_eq!(rig.engine.take_incident(), None, "told once");

    // The next listing starts a helper again, and is fine.
    assert_eq!(rig.engine.gpus().len(), 1);
}

#[test]
fn a_crash_while_loading_on_a_card_fails_the_load_with_an_incident() {
    let mut rig = rig();
    assert_eq!(rig.engine.gpus().len(), 1);
    rig.spawner.fault(RuntimeMode::Gpu, Phase::Load, crash(6, 1));

    let loaded = rig.engine.load(&rig.model, 2, Placement::Gpu { index: 1 });
    let incident = rig.engine.take_incident().expect("an incident");

    assert!(matches!(loaded, Err(AiError::Load(_))), "{loaded:?}");
    assert_eq!(
        (incident.what, incident.exit),
        (IncidentKind::Crash, Some(Exit::Signal(6)))
    );
    assert_eq!(rig.engine.process_id(), None);
}

#[test]
fn a_crash_while_answering_fails_the_answer_and_leaves_no_model() {
    let mut rig = rig();
    rig.load(Placement::Cpu);
    rig.spawner.fault(RuntimeMode::Cpu, Phase::Judge, crash(11, 2));

    let failed = rig.judge();
    let incident = rig.engine.take_incident().expect("an incident");

    assert!(matches!(failed, Err(AiError::Run(_))), "{failed:?}");
    assert_eq!(incident.what, IncidentKind::Crash);
    assert_eq!(incident.stderr_tail, "driver line 0[31m\ndriver line 1[31m");
    assert_eq!(rig.engine.process_id(), None);
    assert_eq!(rig.judge(), Err(AiError::NotLoaded));
    assert_eq!(rig.spawner.running(), 0);
}

// ---- Hangs ----

#[test]
fn on_a_card_an_answer_that_shows_no_progress_is_a_hang_and_the_helper_is_killed() {
    let mut rig = rig();
    rig.load_on_card();
    rig.control.push_stall();
    let began = Instant::now();

    let failed = rig.judge();
    let incident = rig.engine.take_incident().expect("an incident");

    assert!(matches!(failed, Err(AiError::Run(_))), "{failed:?}");
    assert_eq!(incident.what, IncidentKind::Hang);
    assert_eq!(incident.exit, Some(Exit::Signal(9)), "it was killed");
    assert!(began.elapsed() < Duration::from_secs(3), "{:?}", began.elapsed());
    assert_eq!(rig.spawner.running(), 0);
    assert_eq!(rig.engine.process_id(), None);
}

#[test]
fn a_slow_answer_on_a_card_that_shows_progress_is_not_a_hang() {
    let mut rig = rig();
    rig.load_on_card();
    rig.control.push_slow_reply("slow", Duration::from_millis(700));

    assert_eq!(
        rig.judge().unwrap(),
        "slow",
        "several times the stall limit, with progress"
    );
    assert_eq!(rig.engine.take_incident(), None);
}

#[test]
fn on_the_processor_a_quiet_answer_is_not_a_hang_until_a_cancel_goes_unanswered() {
    static CANCEL: AtomicBool = AtomicBool::new(false);
    let mut rig = rig();
    rig.load(Placement::Cpu);
    rig.control.push_stall();
    // Far past the card's stall limit.
    raise_after(&CANCEL, Duration::from_millis(500));

    let stopped = rig.judge_until(&CANCEL);

    assert_eq!(stopped, Err(AiError::Cancelled), "the model answered the cancel");
    assert_eq!(rig.engine.take_incident(), None);
    assert!(rig.engine.process_id().is_some());
}

#[test]
fn on_the_processor_a_helper_that_ignores_a_cancel_is_killed_after_the_grace() {
    static CANCEL: AtomicBool = AtomicBool::new(false);
    let mut rig = rig();
    rig.load(Placement::Cpu);
    rig.spawner.fault(RuntimeMode::Cpu, Phase::Judge, Fault::Hang);
    raise_after(&CANCEL, Duration::from_millis(100));
    let began = Instant::now();

    let failed = rig.judge_until(&CANCEL);
    let incident = rig.engine.take_incident().expect("an incident");

    assert!(matches!(failed, Err(AiError::Run(_))), "{failed:?}");
    assert_eq!(incident.what, IncidentKind::Hang);
    assert!(
        began.elapsed() >= Duration::from_millis(300),
        "after the cancel and the whole grace: {:?}",
        began.elapsed()
    );
    assert_eq!(rig.spawner.running(), 0);
}

#[test]
fn a_helper_that_hangs_while_loading_is_killed_when_the_load_runs_out_of_time() {
    let mut rig = rig_with(Limits {
        load: Duration::from_millis(200),
        ..quick()
    });
    assert_eq!(rig.engine.gpus().len(), 1);
    rig.spawner.fault(RuntimeMode::Gpu, Phase::Load, Fault::Hang);

    let loaded = rig.engine.load(&rig.model, 2, Placement::Gpu { index: 1 });
    let incident = rig.engine.take_incident().expect("an incident");

    assert!(matches!(loaded, Err(AiError::Load(_))), "{loaded:?}");
    assert_eq!(incident.what, IncidentKind::Hang);
    assert_eq!(rig.spawner.running(), 0);
}

// ---- A helper that is played by a script ----

struct Scripted {
    script: Option<Box<dyn FnOnce(UnixStream) + Send>>,
}

struct Stand;

impl RuntimeProcess for Stand {
    fn id(&self) -> u32 {
        1
    }

    fn kill(&mut self) {}

    fn wait_for(&mut self, _timeout: Duration) -> Waited {
        Waited::Exited(None)
    }
}

impl Spawner for Scripted {
    fn spawn(&mut self, _model: Option<std::fs::File>, _mode: RuntimeMode) -> std::io::Result<Spawned> {
        let (app, helper) = UnixStream::pair()?;
        let script = self.script.take().expect("one helper");
        thread::spawn(move || script(helper));

        Ok(Spawned {
            process: Box::new(Stand),
            socket: app,
            stderr: Box::new(std::io::empty()),
        })
    }
}

fn scripted(limits: Limits, script: impl FnOnce(UnixStream) + Send + 'static) -> ProcessEngine<Scripted> {
    ProcessEngine::with_limits(
        Scripted {
            script: Some(Box::new(script)),
        },
        limits,
    )
}

fn hello(peer: &UnixStream, version: u32) {
    write_message(peer, &Reply::Hello { version, pid: 7 }).unwrap();
}

fn next_request(lines: &mut LineReader<UnixStream>) -> Request {
    lines.read::<Request>().unwrap().expect("a request")
}

#[test]
fn a_helper_that_speaks_another_protocol_version_is_refused() {
    let mut engine = scripted(quick(), |peer| {
        hello(&peer, PROTOCOL_VERSION + 1);
        thread::sleep(Duration::from_millis(300));
    });

    let cards = engine.gpus();
    let incident = engine.take_incident().expect("an incident");

    assert!(cards.is_empty());
    assert_eq!(incident.what, IncidentKind::Crash);
    assert_eq!(engine.process_id(), None);
}

#[test]
fn a_helper_that_says_nothing_is_a_hang() {
    let mut engine = scripted(
        Limits {
            probe: Duration::from_millis(100),
            ..quick()
        },
        |peer| {
            thread::sleep(Duration::from_millis(600));
            drop(peer);
        },
    );

    assert!(engine.gpus().is_empty());
    assert_eq!(
        engine.take_incident().map(|incident| incident.what),
        Some(IncidentKind::Hang)
    );
}

#[test]
fn a_line_over_the_limit_ends_the_helper_as_a_crash() {
    let (_dir, model) = model();
    let mut engine = scripted(quick(), |peer| {
        hello(&peer, PROTOCOL_VERSION);
        let mut lines = LineReader::new(peer.try_clone().unwrap());
        let _load = next_request(&mut lines);

        let mut peer = peer;
        let long = vec![b'x'; MAX_LINE_BYTES + 100];
        let _ = peer.write_all(&long);
        let _ = peer.write_all(b"\n");
        thread::sleep(Duration::from_millis(300));
    });

    let loaded = engine.load(&model, 1, Placement::Cpu);

    assert!(matches!(loaded, Err(AiError::Load(_))), "{loaded:?}");
    assert_eq!(
        engine.take_incident().map(|incident| incident.what),
        Some(IncidentKind::Crash)
    );
}

#[test]
fn a_reply_that_is_not_a_message_ends_the_helper_as_a_crash() {
    let mut engine = scripted(quick(), |mut peer| {
        hello(&peer, PROTOCOL_VERSION);
        let mut lines = LineReader::new(peer.try_clone().unwrap());
        let _gpus = next_request(&mut lines);
        let _ = peer.write_all(b"this is not json\n");
        thread::sleep(Duration::from_millis(300));
    });

    assert!(engine.gpus().is_empty());
    assert_eq!(
        engine.take_incident().map(|incident| incident.what),
        Some(IncidentKind::Crash)
    );
}

#[test]
fn a_reply_for_another_question_is_ignored() {
    let (_dir, model) = model();
    let mut engine = scripted(quick(), |peer| {
        hello(&peer, PROTOCOL_VERSION);
        let mut lines = LineReader::new(peer.try_clone().unwrap());

        let Request::Load { threads, .. } = next_request(&mut lines) else {
            panic!("a load comes first");
        };
        write_message(&peer, &Reply::Loaded { millis: 1, threads }).unwrap();

        let Request::Judge { id, .. } = next_request(&mut lines) else {
            panic!("a question comes next");
        };
        let stale = id + 50;
        for reply in [
            Reply::Generation {
                id: stale,
                text: "stale".to_owned(),
                tokens: 1,
                tokens_per_s: 1.0,
            },
            Reply::Progress { id: Some(stale) },
            Reply::Error {
                id: Some(stale),
                kind: platter_ai::protocol::ErrorKind::Run,
                text: "stale".to_owned(),
            },
            Reply::Generation {
                id,
                text: "fresh".to_owned(),
                tokens: 1,
                tokens_per_s: 1.0,
            },
        ] {
            write_message(&peer, &reply).unwrap();
        }
        let mut sink = Vec::new();
        let _ = (&peer).read_to_end(&mut sink);
    });
    engine.load(&model, 1, Placement::Cpu).unwrap();

    let answer = engine.judge("p", "g", 4, &AtomicBool::new(false));

    assert_eq!(answer.unwrap().text, "fresh");
    assert_eq!(engine.take_incident(), None);
}
