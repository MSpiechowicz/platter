use std::fs;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use platter_ai::MODEL;
use platter_ai::testing::{FakeControl, FakeEngine, FakeFetch, Gate, tiny_spec};
use platter_core::dto::ApiErrorCode;
use platter_core::testing::{FakeFs, scan_fake};
use platter_core::{ModelLabel, ModelVerdict, UserDir};
use serde_json::{Value, json};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewWindow, WebviewWindowBuilder};

use super::*;
use crate::commands::cleanup::Confirm;

const WAIT: Duration = Duration::from_secs(10);

pub(super) fn content() -> Vec<u8> {
    (0..300u32).map(|n| (n * 7 % 251) as u8).collect()
}

pub(super) struct Rig {
    pub(super) _root: tempfile::TempDir,
    pub(super) dir: std::path::PathBuf,
    pub(super) service: AdvisorService,
    pub(super) control: FakeControl,
    pub(super) fetch: Arc<FakeFetch>,
}

/// The user's answer to the native dialog, and what the dialog was asked.
struct Dialog {
    approve: bool,
    asked: Mutex<Vec<Question>>,
}

impl Dialog {
    fn answering(approve: bool) -> Arc<Self> {
        Arc::new(Self {
            approve,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<Question> {
        lock(&self.asked).clone()
    }
}

impl Confirm for Dialog {
    fn confirm(&self, question: &Question) -> bool {
        lock(&self.asked).push(question.clone());

        self.approve
    }
}

struct Panics;

impl Confirm for Panics {
    fn confirm(&self, _question: &Question) -> bool {
        panic!("the dialog fails");
    }
}

/// An advisor on a fake model and a fake server, switched on as the web view does at start; the model
/// file is already there when `on_disk`.
fn rig(on_disk: bool) -> Rig {
    let rig = fresh(on_disk);
    rig.service
        .configure(AdvisorOptionsDto {
            enabled: true,
            unload_after: UnloadAfterDto::FiveMinutes,
            log_questions: true,
            use_gpu: true,
        })
        .unwrap();

    rig
}

/// Like [`rig`], but with the options the service starts with (the web view has not sent its own yet).
pub(super) fn fresh(on_disk: bool) -> Rig {
    fresh_with(on_disk, FakeFetch::new([]).then_serve(&content()))
}

pub(super) fn fresh_with(on_disk: bool, fetch: FakeFetch) -> Rig {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("models");
    let spec = tiny_spec(&content());
    if on_disk {
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(spec.file), content()).unwrap();
    }

    let mut config = WorkerConfig::new(spec, dir.clone());
    config.threads = 2;
    config.progress_every = Duration::ZERO;
    let (engine, control) = FakeEngine::new();
    let fetch = Arc::new(fetch);
    let service =
        AdvisorService::spawn(config, Box::new(engine), Arc::clone(&fetch) as Arc<dyn Fetch>).unwrap();

    Rig {
        _root: root,
        dir,
        service,
        control,
        fetch,
    }
}

/// Starts the download the way the command does after the user approved it.
fn start(service: &AdvisorService) -> CommandResult<()> {
    let (_, ticket) = service.download_question()?;

    service.start_download(ticket)
}

/// A watcher that has been told the status it started with (the first event of every watch).
fn watched(service: &AdvisorService) -> Receiver<AdvisorStatusDto> {
    let receiver = watched_from_start(service);
    receiver
        .recv_timeout(WAIT)
        .expect("the current status arrives first");

    receiver
}

fn watched_from_start(service: &AdvisorService) -> Receiver<AdvisorStatusDto> {
    let (sender, receiver) = mpsc::channel();
    service.watch(Arc::new(move |status| {
        let _ = sender.send(status);
    }));

    receiver
}

/// Waits for an event in `state`, returning it with the states seen on the way.
fn until(
    events: &Receiver<AdvisorStatusDto>,
    state: ModelStateDto,
) -> (AdvisorStatusDto, Vec<ModelStateDto>) {
    let mut seen = Vec::new();
    let started = Instant::now();
    loop {
        let left = WAIT.saturating_sub(started.elapsed());
        let status = events.recv_timeout(left).expect("the status arrives");
        if seen.last() != Some(&status.state) {
            seen.push(status.state);
        }
        if status.state == state {
            return (status, seen);
        }
    }
}

#[test]
fn the_status_is_sent_in_the_contracts_shape() {
    let rig = fresh(false);

    let json = serde_json::to_value(rig.service.status()).unwrap();

    assert_eq!(json["state"], "notDownloaded");
    assert!(json["model"].get("id").is_none());
    assert_eq!(json["model"]["name"], "Tiny");
    assert_eq!(json["model"]["format"], "test");
    assert_eq!(json["model"]["bytes"], 300);
    assert_eq!(json["model"]["host"], "models.example");
    assert_eq!(
        json["options"],
        json!({ "enabled": false, "unloadAfter": "fiveMinutes", "logQuestions": true, "useGpu": true })
    );
    assert_eq!(json["gpuSupported"], cfg!(feature = "gpu-vulkan"));
    for gone in ["models", "recommended"] {
        assert!(json.get(gone).is_none(), "{gone} is not in the status");
    }
    assert!(json["ramTotal"].is_u64());
    for absent in [
        "download",
        "problem",
        "ramBytes",
        "tokensPerSecond",
        "labelling",
        "device",
        "videoBytes",
        "gpuProblem",
        "lastGpuProblem",
    ] {
        assert!(
            json.get(absent).is_none(),
            "{absent} is left out until it has a value"
        );
    }
}

#[test]
fn the_real_model_is_described_by_its_host() {
    let info = model_info(&MODEL);

    assert_eq!(info.host, "huggingface.co");
    assert_eq!(info.name, "Qwen3.5-4B");
    assert_eq!(info.bytes, 2_740_937_888);
    assert_eq!(
        serde_json::to_value(info).unwrap(),
        json!({
            "name": "Qwen3.5-4B",
            "format": "GGUF · Q4_K_M",
            "bytes": 2_740_937_888u64,
            "host": "huggingface.co",
        })
    );
}

#[test]
fn a_download_a_load_and_an_unload_push_their_changes_to_the_watcher() {
    let rig = rig(false);
    let events = watched(&rig.service);

    start(&rig.service).unwrap();
    let (on_disk, seen) = until(&events, ModelStateDto::OnDisk);
    assert_eq!(seen, [ModelStateDto::Downloading, ModelStateDto::OnDisk]);
    assert_eq!(on_disk.download, None);

    let loaded = rig.service.load().unwrap();
    assert_eq!(loaded.state, ModelStateDto::Loaded);
    assert!(loaded.ram_bytes.is_some());
    let (_, seen) = until(&events, ModelStateDto::Loaded);
    assert_eq!(seen, [ModelStateDto::Loading, ModelStateDto::Loaded]);

    let unloaded = rig.service.unload().unwrap();
    assert_eq!(unloaded.state, ModelStateDto::OnDisk);
    assert_eq!(unloaded.ram_bytes, None);
    until(&events, ModelStateDto::OnDisk);
    assert_eq!(rig.control.calls(), ["load 2", "unload"]);
}

#[test]
fn download_progress_reaches_the_watcher_with_the_total() {
    let rig = rig(false);
    let events = watched(&rig.service);

    start(&rig.service).unwrap();
    let first = events.recv_timeout(WAIT).unwrap();

    assert_eq!(first.state, ModelStateDto::Downloading);
    assert_eq!(
        first.download,
        Some(DownloadDto {
            received: 0,
            total: 300
        })
    );
}

#[test]
fn nothing_loads_before_the_web_view_sends_its_options() {
    let rig = fresh(true);

    assert_eq!(rig.service.load().unwrap_err().code(), ApiErrorCode::Invalid);
    assert!(rig.control.calls().is_empty());
}

#[test]
fn a_new_watcher_is_told_the_current_status_first() {
    let rig = rig(true);
    rig.service.load().unwrap();

    let events = watched_from_start(&rig.service);

    let first = events.recv_timeout(WAIT).unwrap();
    assert_eq!(first.state, ModelStateDto::Loaded);
    assert_eq!(first.options, rig.service.status().options);
    assert!(
        events.recv_timeout(Duration::from_millis(100)).is_err(),
        "nothing else until something changes"
    );
}

#[test]
fn a_new_watcher_replaces_the_old_one() {
    let rig = rig(true);
    let old = watched(&rig.service);
    let new = watched(&rig.service);

    rig.service.load().unwrap();

    assert_eq!(
        until(&new, ModelStateDto::Loaded).1,
        [ModelStateDto::Loading, ModelStateDto::Loaded]
    );
    assert!(
        old.recv_timeout(Duration::from_millis(100)).is_err(),
        "the old channel hears nothing"
    );
}

#[test]
fn a_status_change_without_a_watcher_is_not_an_error() {
    let rig = rig(true);

    assert_eq!(rig.service.load().unwrap().state, ModelStateDto::Loaded);
}

#[test]
fn the_options_are_kept_and_switching_off_unloads_and_refuses_a_load() {
    let rig = rig(true);
    rig.service.load().unwrap();

    let off = rig
        .service
        .configure(AdvisorOptionsDto {
            enabled: false,
            unload_after: UnloadAfterDto::WindowClose,
            log_questions: true,
            use_gpu: true,
        })
        .unwrap();

    assert_eq!(off.state, ModelStateDto::OnDisk);
    assert_eq!(
        off.options,
        AdvisorOptionsDto {
            enabled: false,
            unload_after: UnloadAfterDto::WindowClose,
            log_questions: true,
            use_gpu: true,
        }
    );
    assert_eq!(rig.service.status().options, off.options);
    assert_eq!(rig.service.load().unwrap_err().code(), ApiErrorCode::Invalid);
}

#[test]
fn requests_that_do_not_fit_the_state_are_invalid() {
    let missing = rig(false);
    let present = rig(true);

    assert_eq!(missing.service.load().unwrap_err().code(), ApiErrorCode::Invalid);
    assert_eq!(start(&present.service).unwrap_err().code(), ApiErrorCode::Invalid);

    start(&missing.service).unwrap();
    assert_eq!(start(&missing.service).unwrap_err().code(), ApiErrorCode::Invalid);
}

#[test]
fn cancelling_with_no_download_and_unloading_an_unloaded_model_are_fine() {
    let rig = rig(true);

    rig.service.cancel_download();

    assert_eq!(rig.service.unload().unwrap().state, ModelStateDto::OnDisk);
}

#[test]
fn quitting_unloads_the_model_and_later_requests_are_refused() {
    let rig = rig(true);
    rig.service.load().unwrap();

    rig.service.shutdown();

    assert!(!rig.control.is_loaded());
    assert_eq!(rig.service.load().unwrap_err().code(), ApiErrorCode::Invalid);
}

#[test]
fn without_a_runtime_the_state_is_unavailable_and_only_the_options_work() {
    let service = AdvisorService::unavailable();

    let status = service.status();
    assert_eq!(status.state, ModelStateDto::Unavailable);
    assert_eq!(status.model.name, "Qwen3.5-4B");

    assert_eq!(start(&service).unwrap_err().code(), ApiErrorCode::Invalid);
    assert_eq!(service.load().unwrap_err().code(), ApiErrorCode::Invalid);
    assert_eq!(service.unload().unwrap_err().code(), ApiErrorCode::Invalid);
    service.cancel_download();
    service.shutdown();

    let kept = service
        .configure(AdvisorOptionsDto {
            enabled: false,
            unload_after: UnloadAfterDto::OneMinute,
            log_questions: true,
            use_gpu: true,
        })
        .unwrap();
    assert_eq!(kept.state, ModelStateDto::Unavailable);
    assert!(!kept.options.enabled);
    assert_eq!(service.status().options.unload_after, UnloadAfterDto::OneMinute);
    assert_eq!(kept.model.name, "Qwen3.5-4B");
}

// ---- Through Tauri's IPC: names of commands and arguments, and what the web view may send ----

pub(super) fn app(rig_service: AdvisorService) -> (App<MockRuntime>, WebviewWindow<MockRuntime>) {
    app_asking(rig_service, Dialog::answering(true))
}

/// The commands over Tauri's IPC, with `dialog` as the user's answer to the native confirmation.
fn app_asking(
    rig_service: AdvisorService,
    dialog: Arc<dyn Confirm>,
) -> (App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let state = AppState::with_advisor(rig_service);
    state.set_confirm(dialog);

    let app = mock_builder()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            advisor_status,
            watch_advisor,
            configure_advisor,
            start_model_download,
            cancel_model_download,
            load_model,
            unload_model,
            label_folders,
            cancel_labelling,
            get_advisor_labels,
            ask_advisor,
            find_folders,
            clear_question_log,
            answer_gpu_problem,
            clear_gpu_log,
            crate::commands::suggestions::get_suggestions,
        ])
        .build(tauri::generate_context!())
        .expect("the app builds");
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("the main window opens");

    (app, window)
}

pub(super) fn invoke(window: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    let url = if cfg!(any(windows, target_os = "android")) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    };

    let response = get_ipc_response(
        window,
        InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: url.parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )?;

    Ok(response.deserialize().expect("a JSON response"))
}

fn wait_for_state(window: &WebviewWindow<MockRuntime>, state: &str) -> Value {
    let started = Instant::now();
    loop {
        let status = invoke(window, "advisor_status", json!({})).unwrap();
        if status["state"] == state {
            return status;
        }
        assert!(
            started.elapsed() < WAIT,
            "timed out waiting for {state}: {status}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_commands_work_over_ipc_with_the_contracts_names() {
    let Rig { _root, service, .. } = rig(false);
    let (_app, window) = app(service);

    assert_eq!(
        invoke(&window, "advisor_status", json!({})).unwrap()["state"],
        "notDownloaded"
    );
    assert_eq!(
        invoke(&window, "watch_advisor", json!({ "onEvent": "__CHANNEL__:7" })),
        Ok(Value::Null)
    );
    assert_eq!(
        invoke(&window, "cancel_model_download", json!({})),
        Ok(Value::Null)
    );

    assert_eq!(
        invoke(&window, "start_model_download", json!({})),
        Ok(Value::Null)
    );
    wait_for_state(&window, "onDisk");

    let loaded = invoke(&window, "load_model", json!({})).unwrap();
    assert_eq!(loaded["state"], "loaded");
    let configured = invoke(
        &window,
        "configure_advisor",
        json!({ "options": { "enabled": true, "unloadAfter": "windowClose", "logQuestions": true } }),
    )
    .unwrap();
    assert_eq!(
        configured["options"],
        json!({ "enabled": true, "unloadAfter": "windowClose", "logQuestions": true, "useGpu": true })
    );
    assert_eq!(
        configured["device"],
        json!({ "kind": "cpu" }),
        "the fake has no card"
    );
    assert_eq!(
        invoke(&window, "unload_model", json!({})).unwrap()["state"],
        "onDisk"
    );
}

// ---- The download is confirmed by the backend, not by the web view ----

fn started_over_ipc(window: &WebviewWindow<MockRuntime>) -> Result<Value, Value> {
    invoke(window, "start_model_download", json!({}))
}

#[test]
fn a_declined_download_is_cancelled_asks_the_server_for_nothing_and_changes_nothing() {
    let Rig {
        _root,
        service,
        fetch,
        ..
    } = rig(false);
    let dialog = Dialog::answering(false);
    let (_app, window) = app_asking(service, Arc::clone(&dialog) as Arc<dyn Confirm>);

    let declined = started_over_ipc(&window).unwrap_err();

    assert_eq!(declined["code"], "cancelled");
    assert_eq!(dialog.asked().len(), 1, "the user was asked once");
    assert_eq!(
        invoke(&window, "advisor_status", json!({})).unwrap()["state"],
        "notDownloaded"
    );
    std::thread::sleep(Duration::from_millis(50));
    assert!(fetch.requests().is_empty(), "nothing was downloaded");
}

#[test]
fn without_a_dialog_and_with_a_dialog_that_fails_the_download_is_cancelled_too() {
    let silent = rig(false);
    let (_app, window) = app_asking(silent.service, Arc::new(Panics));
    assert_eq!(started_over_ipc(&window).unwrap_err()["code"], "cancelled");
    assert!(silent.fetch.requests().is_empty());

    // The state of an app whose setup has not set a dialog (yet) approves nothing.
    let unset = world(false);
    assert_eq!(
        code_of(block_on(download_model(&unset.state))),
        ApiErrorCode::Cancelled
    );
    assert_eq!(unset.state.advisor.status().state, ModelStateDto::NotDownloaded);
}

#[test]
fn an_approved_download_runs_and_the_dialog_said_what_it_is_about() {
    let Rig {
        _root,
        service,
        fetch,
        ..
    } = rig(false);
    let dialog = Dialog::answering(true);
    let (_app, window) = app_asking(service, Arc::clone(&dialog) as Arc<dyn Confirm>);

    assert_eq!(started_over_ipc(&window), Ok(Value::Null));

    wait_for_state(&window, "onDisk");
    assert_eq!(fetch.requests().len(), 1);
    assert_eq!(
        dialog.asked(),
        [Question {
            title: "Download the advisor model?".to_owned(),
            text: "Platter will download Tiny (0.0 GB) from models.example. This is Platter's only network call. The file is checked before it is used.".to_owned(),
            accept: "Download",
        }]
    );
}

#[test]
fn the_question_names_the_real_model_and_says_when_a_download_is_continued() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("models");
    let (engine, _control) = FakeEngine::new();
    let fetch = Arc::new(FakeFetch::new([]));
    let service =
        AdvisorService::spawn(WorkerConfig::new(MODEL, dir.clone()), Box::new(engine), fetch).unwrap();

    let (fresh, _) = service.download_question().expect("a download can start");
    assert_eq!(fresh.title, "Download the advisor model?");
    assert_eq!(
        fresh.text,
        "Platter will download Qwen3.5-4B (2.7 GB) from huggingface.co. This is Platter's only network call. The file is checked before it is used."
    );
    service.shutdown();

    // A partial file from an earlier try is there when the app starts again.
    fs::create_dir_all(&dir).unwrap();
    let partial = fs::File::create(dir.join(MODEL.partial_file())).unwrap();
    partial.set_len(400_000_000).unwrap();
    let (engine, _control) = FakeEngine::new();
    let again = AdvisorService::spawn(
        WorkerConfig::new(MODEL, dir),
        Box::new(engine),
        Arc::new(FakeFetch::new([])),
    )
    .unwrap();

    let (continued, _) = again.download_question().expect("a download can continue");
    assert_eq!(
        continued.text,
        "Platter will continue the download of Qwen3.5-4B (0.4 of 2.7 GB received) from huggingface.co. This is Platter's only network call. The file is checked before it is used."
    );
    assert_eq!(continued.accept, "Download");
}

#[test]
fn the_options_are_enabled_unload_after_and_log_questions() {
    let sent = |value: Value| serde_json::from_value::<AdvisorOptionsDto>(value);
    let options = AdvisorOptionsDto {
        enabled: true,
        unload_after: UnloadAfterDto::OneMinute,
        log_questions: false,
        use_gpu: true,
    };

    assert_eq!(
        sent(json!({ "enabled": true, "unloadAfter": "oneMinute", "logQuestions": false })).unwrap(),
        options
    );
    assert_eq!(
        serde_json::to_value(options).unwrap(),
        json!({ "enabled": true, "unloadAfter": "oneMinute", "logQuestions": false, "useGpu": true })
    );
    assert!(
        !sent(json!({ "enabled": true, "unloadAfter": "oneMinute", "useGpu": false }))
            .unwrap()
            .use_gpu
    );
    assert!(
        sent(json!({ "enabled": true, "unloadAfter": "oneMinute" }))
            .unwrap()
            .use_gpu,
        "the graphics card is used unless the options say otherwise"
    );
    assert!(
        sent(json!({ "enabled": true, "unloadAfter": "oneMinute" }))
            .unwrap()
            .log_questions,
        "the question log is on unless the options say otherwise"
    );

    for refused in [
        json!({ "enabled": true }),
        json!({ "enabled": true, "unloadAfter": "oneMinute", "logQuestions": "yes" }),
        json!({ "enabled": true, "unloadAfter": "oneMinute", "useGpu": "yes" }),
        json!({ "enabled": true, "unloadAfter": "oneMinute", "useGpu": true, "device": 1 }),
        json!({ "enabled": true, "unloadAfter": "oneMinute", "model": "qwen3.5-4b" }),
        json!({ "enabled": true, "unloadAfter": "oneMinute", "model": "qwen3.5-2b" }),
        json!({ "enabled": true, "unloadAfter": "oneMinute", "path": "/tmp/x" }),
        json!({ "enabled": true, "unloadAfter": "oneMinute", "logQuestions": true, "log": "/tmp/x" }),
    ] {
        assert!(sent(refused.clone()).is_err(), "{refused}");
    }
}

#[test]
fn clear_question_log_takes_no_arguments_and_answers_ok_with_no_log() {
    let Rig { _root, service, .. } = rig(true);
    let (_app, window) = app(service);

    assert_eq!(
        invoke(&window, "clear_question_log", json!({})),
        Ok(Value::Null),
        "ok, though the state has no log at all"
    );
}

#[test]
fn configuring_over_ipc_answers_without_a_model_choice_and_an_old_model_field_is_refused() {
    let Rig { _root, service, .. } = rig(true);
    let (_app, window) = app(service);

    let set = invoke(
        &window,
        "configure_advisor",
        json!({ "options": { "enabled": true, "unloadAfter": "oneMinute", "logQuestions": false } }),
    )
    .unwrap();

    assert_eq!(
        set["options"],
        json!({ "enabled": true, "unloadAfter": "oneMinute", "logQuestions": false, "useGpu": true })
    );
    assert_eq!(set["state"], "onDisk");
    assert!(set["model"].get("id").is_none());
    assert!(set.get("models").is_none() && set.get("recommended").is_none());

    assert!(
        invoke(
            &window,
            "configure_advisor",
            json!({ "options": { "enabled": true, "unloadAfter": "fiveMinutes", "model": "qwen3.5-2b" } }),
        )
        .is_err(),
        "an old `model` field is not part of the contract"
    );
    assert_eq!(
        invoke(&window, "advisor_status", json!({})).unwrap()["options"]["unloadAfter"],
        "oneMinute",
        "the refused call changed nothing"
    );
}

#[test]
fn the_status_shows_a_partial_file_that_is_on_disk_at_start() {
    let w = fresh(false);
    fs::create_dir_all(&w.dir).unwrap();
    fs::write(w.dir.join(tiny_spec(&content()).partial_file()), [1u8; 120]).unwrap();
    // The partial file is looked for at start: a service on the same folder sees it.
    let (engine, _control) = FakeEngine::new();
    let config = WorkerConfig::new(tiny_spec(&content()), w.dir.clone());
    let service = AdvisorService::spawn(config, Box::new(engine), Arc::new(FakeFetch::new([]))).unwrap();

    let status = service.status();

    assert_eq!(status.state, ModelStateDto::NotDownloaded);
    assert_eq!(
        status.download,
        Some(DownloadDto {
            received: 120,
            total: 300
        })
    );
}

#[test]
fn the_download_question_names_the_model_and_its_size() {
    let w = rig(false);
    let (tiny, _) = w.service.download_question().unwrap();
    assert!(
        tiny.text
            .starts_with("Platter will download Tiny (0.0 GB) from models.example.")
    );

    // The real model: the dialog says which one and how large it is.
    let root = tempfile::tempdir().unwrap();
    let (engine, _control) = FakeEngine::new();
    let real = AdvisorService::spawn(
        WorkerConfig::new(MODEL, root.path().join("models")),
        Box::new(engine),
        Arc::new(FakeFetch::new([])),
    )
    .unwrap();
    let (question, _) = real.download_question().unwrap();
    assert_eq!(
        question.text,
        "Platter will download Qwen3.5-4B (2.7 GB) from huggingface.co. This is Platter's only network call. The file is checked before it is used."
    );
}

#[test]
fn the_old_models_files_are_removed_when_the_service_starts() {
    let w = fresh(true);
    fs::write(w.dir.join("Qwen3.5-2B-Q4_K_M.gguf"), b"old").unwrap();
    fs::write(w.dir.join("Qwen3.5-2B-Q4_K_M.gguf.partial"), b"old").unwrap();
    let (engine, _control) = FakeEngine::new();
    let config = WorkerConfig::new(tiny_spec(&content()), w.dir.clone());

    let service = AdvisorService::spawn(config, Box::new(engine), Arc::new(FakeFetch::new([]))).unwrap();

    assert!(!w.dir.join("Qwen3.5-2B-Q4_K_M.gguf").exists());
    assert!(!w.dir.join("Qwen3.5-2B-Q4_K_M.gguf.partial").exists());
    assert!(w.dir.join(tiny_spec(&content()).file).exists());
    assert_eq!(service.status().state, ModelStateDto::OnDisk);
}

/// A user who approves the dialog, while a download is started behind it.
struct StartsWhileAsking {
    advisor: Arc<AdvisorService>,
}

impl Confirm for StartsWhileAsking {
    fn confirm(&self, _question: &Question) -> bool {
        start(&self.advisor).unwrap();

        true
    }
}

#[test]
fn a_dialog_approved_after_the_state_changed_starts_nothing_more() {
    let w = rig(false);
    let state = AppState::with_advisor(w.service);
    state.set_confirm(Arc::new(StartsWhileAsking {
        advisor: Arc::clone(&state.advisor),
    }));

    let result = block_on(download_model(&state));

    assert_eq!(
        result,
        Err(CommandError::invalid("The model's state changed. Try again."))
    );
    let started = Instant::now();
    while state.advisor.status().state != ModelStateDto::OnDisk {
        assert!(started.elapsed() < WAIT, "the download behind the dialog ends");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        w.fetch.requests().len(),
        1,
        "only the download started behind the dialog"
    );
}

#[test]
fn nothing_is_asked_when_no_download_can_start() {
    let present = rig(true);
    let dialog = Dialog::answering(true);
    let state = AppState::with_advisor(present.service);
    state.set_confirm(Arc::clone(&dialog) as Arc<dyn Confirm>);

    assert_eq!(code_of(block_on(download_model(&state))), ApiErrorCode::Invalid);
    let none = AppState::with_advisor(AdvisorService::unavailable());
    none.set_confirm(Arc::clone(&dialog) as Arc<dyn Confirm>);
    assert_eq!(code_of(block_on(download_model(&none))), ApiErrorCode::Invalid);

    assert!(dialog.asked().is_empty());
    assert!(present.fetch.requests().is_empty());
}

#[test]
fn a_ticket_from_an_earlier_state_does_not_start_a_download() {
    let w = rig(false);
    let events = watched(&w.service);
    let (_, ticket) = w.service.download_question().unwrap();

    // The model arrives, then its file goes missing and the next load finds out: not downloaded again,
    // but not the state the user was asked about.
    start(&w.service).unwrap();
    until(&events, ModelStateDto::OnDisk);
    fs::remove_file(w.dir.join(tiny_spec(&content()).file)).unwrap();
    assert_eq!(w.service.load().unwrap().state, ModelStateDto::NotDownloaded);
    let requests = w.fetch.requests().len();

    let stale = w.service.start_download(ticket).unwrap_err();

    assert_eq!(
        stale,
        CommandError::invalid("The model's state changed. Try again.")
    );
    assert_eq!(w.service.status().state, ModelStateDto::NotDownloaded);
    assert_eq!(w.fetch.requests().len(), requests, "nothing was requested");

    let (_, fresh) = w.service.download_question().unwrap();
    w.service.start_download(fresh).unwrap();
}

#[test]
fn no_download_starts_without_a_dialog_while_the_state_changes_under_the_command() {
    // The race is a matter of timing: many short rounds give it many chances.
    for _ in 0..25 {
        let w = rig(true);
        fs::remove_file(w.dir.join(tiny_spec(&content()).file)).unwrap();
        // The user's answer is no, every time; the dialog never pauses the next request.
        let state = AppState::with_advisor(w.service.with_decline_pause(Duration::ZERO));
        let stop = AtomicBool::new(false);

        let codes = std::thread::scope(|scope| {
            let requests = scope.spawn(|| {
                let mut codes = Vec::new();
                while !stop.load(Ordering::SeqCst) {
                    codes.push(code_of(block_on(download_model(&state))));
                }

                codes
            });

            // On disk as far as the state knows, until this load finds the file missing.
            std::thread::sleep(Duration::from_millis(3));
            let loaded = state.advisor.load().unwrap();
            assert_eq!(loaded.state, ModelStateDto::NotDownloaded);
            std::thread::sleep(Duration::from_millis(3));
            stop.store(true, Ordering::SeqCst);

            requests.join().unwrap()
        });

        assert!(
            codes
                .iter()
                .all(|code| matches!(code, ApiErrorCode::Invalid | ApiErrorCode::Cancelled)),
            "{codes:?}"
        );
        assert!(w.fetch.requests().is_empty(), "nothing was downloaded");
    }
}

#[test]
fn an_approved_download_that_was_cancelled_asks_again() {
    let gate = Gate::new();
    let w = fresh_with(false, FakeFetch::new([]).then_serve(&content()).stall_at(&gate));
    w.service
        .configure(AdvisorOptionsDto {
            enabled: true,
            unload_after: UnloadAfterDto::FiveMinutes,
            log_questions: true,
            use_gpu: true,
        })
        .unwrap();
    let dialog = Dialog::answering(true);
    let state = AppState::with_advisor(w.service);
    state.set_confirm(Arc::clone(&dialog) as Arc<dyn Confirm>);

    block_on(download_model(&state)).unwrap();
    assert!(gate.wait_for_visitor(WAIT), "the download began");
    state.advisor.cancel_download();
    let started = Instant::now();
    while state.advisor.status().state != ModelStateDto::NotDownloaded {
        assert!(started.elapsed() < WAIT, "the cancel took effect");
        std::thread::sleep(Duration::from_millis(5));
    }
    gate.open();

    block_on(download_model(&state)).unwrap();

    assert_eq!(dialog.asked().len(), 2, "the user was asked again");
}

/// A dialog that stays open until its gate is opened, and says no.
struct Held {
    gate: Arc<Gate>,
    asked: AtomicUsize,
}

impl Confirm for Held {
    fn confirm(&self, _question: &Question) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.gate.pass();

        false
    }
}

#[test]
fn only_one_download_dialog_is_open_at_a_time() {
    let w = world(false);
    let held = Arc::new(Held {
        gate: Gate::new(),
        asked: AtomicUsize::new(0),
    });
    w.state.set_confirm(Arc::clone(&held) as Arc<dyn Confirm>);

    std::thread::scope(|scope| {
        let first = scope.spawn(|| block_on(download_model(&w.state)));
        assert!(held.gate.wait_for_visitor(WAIT), "the first dialog is open");

        let second = Instant::now();
        assert_eq!(code_of(block_on(download_model(&w.state))), ApiErrorCode::Invalid);
        assert!(second.elapsed() < Duration::from_secs(1), "refused at once");
        assert_eq!(held.asked.load(Ordering::SeqCst), 1, "no second dialog");

        held.gate.open();
        assert_eq!(code_of(first.join().unwrap()), ApiErrorCode::Cancelled);
    });
    assert!(w.control.calls().is_empty());
}

#[test]
fn after_a_declined_download_a_new_request_waits_a_moment() {
    let pause = Duration::from_millis(150);
    let Rig {
        _root,
        service,
        fetch,
        ..
    } = rig(false);
    let dialog = Dialog::answering(false);
    let state = AppState::with_advisor(service.with_decline_pause(pause));
    state.set_confirm(Arc::clone(&dialog) as Arc<dyn Confirm>);

    assert_eq!(code_of(block_on(download_model(&state))), ApiErrorCode::Cancelled);
    assert_eq!(code_of(block_on(download_model(&state))), ApiErrorCode::Invalid);
    assert_eq!(dialog.asked().len(), 1, "not asked again right away");

    std::thread::sleep(pause + Duration::from_millis(50));
    assert_eq!(code_of(block_on(download_model(&state))), ApiErrorCode::Cancelled);
    assert_eq!(dialog.asked().len(), 2, "asked again after the pause");
    assert!(fetch.requests().is_empty());
}

#[test]
fn a_request_racing_a_decline_is_refused_and_only_one_dialog_is_asked() {
    let gate = DialogGate::new(Duration::from_secs(60));
    let first = Mutex::new(Some(DialogSlot::take(&gate).unwrap()));

    // The second request is between its checks while the first dialog is declined and closes.
    let second = DialogSlot::take_between(&gate, || {
        let slot = first.lock().unwrap().take().unwrap();
        slot.declined();
    });

    assert_eq!(code_of(second.map(drop)), ApiErrorCode::Invalid);
}

#[test]
fn a_request_arriving_just_after_a_decline_is_refused_and_gives_the_slot_back() {
    let pause = Duration::from_millis(100);
    let gate = DialogGate::new(pause);
    let first = DialogSlot::take(&gate).unwrap();
    first.declined();
    drop(first);

    let mut ran = false;
    let second = DialogSlot::take_between(&gate, || ran = true);

    assert!(ran, "the slot was won before the pause was looked at");
    assert_eq!(code_of(second.map(drop)), ApiErrorCode::Invalid);
    assert!(!gate.open.load(Ordering::SeqCst), "a refusal gives the slot back");

    std::thread::sleep(pause + Duration::from_millis(50));
    assert!(DialogSlot::take(&gate).is_ok());
}

#[test]
fn a_download_held_back_by_another_window_ends_with_its_reason() {
    let w = rig(false);
    let events = watched(&w.service);
    let other_window = platter_ai::ModelLock::try_acquire(&{
        fs::create_dir_all(&w.dir).unwrap();
        w.dir.clone()
    })
    .unwrap()
    .unwrap();

    start(&w.service).unwrap();
    let (status, _) = until(&events, ModelStateDto::NotDownloaded);

    assert_eq!(
        status.problem.as_deref(),
        Some("Another Platter window is downloading the model.")
    );
    drop(other_window);
}

#[test]
fn a_refused_request_is_an_invalid_error_object() {
    let Rig { _root, service, .. } = rig(false);
    let (_app, window) = app(service);

    let refused = invoke(&window, "load_model", json!({})).unwrap_err();

    assert_eq!(refused["code"], "invalid");
    assert!(refused["message"].as_str().is_some_and(|text| !text.is_empty()));
}

#[test]
fn options_with_an_unknown_field_or_value_are_not_accepted() {
    let Rig { _root, service, .. } = rig(false);
    let (_app, window) = app(service);

    for options in [
        json!({ "enabled": true, "unloadAfter": "fiveMinutes", "url": "https://elsewhere.example/m.gguf" }),
        json!({ "enabled": true, "unloadAfter": "never" }),
        json!({ "enabled": "yes", "unloadAfter": "oneMinute" }),
        json!({ "enabled": true }),
    ] {
        assert!(
            invoke(&window, "configure_advisor", json!({ "options": options })).is_err(),
            "{options}"
        );
    }
}

// ---- Labelling, asking and finding folders ----

pub(super) const MIB: u64 = 1 << 20;
const REVIEW: &str = r#"{"verdict":"review","reason":"Cache the system rebuilds."}"#;
const KEEP: &str = r#"{"verdict":"keep","reason":"Personal photos."}"#;

/// The app state around an advisor on a fake model, and the remote control of that model.
pub(super) struct World {
    pub(super) _root: tempfile::TempDir,
    pub(super) state: AppState,
    pub(super) control: FakeControl,
}

pub(super) fn world(on_disk: bool) -> World {
    let Rig {
        _root,
        service,
        control,
        ..
    } = rig(on_disk);

    World {
        _root,
        state: AppState::with_advisor(service),
        control,
    }
}

pub(super) fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(future)
}

/// Makes `fs` a finished scan.
pub(super) fn register(state: &AppState, fs: &FakeFs) -> ScanId {
    let (id, _) = state.scans.begin().unwrap();
    state.scans.finish(id, scan_fake(fs, false)).unwrap();

    id
}

pub(super) fn node_at(state: &AppState, scan: ScanId, path: &str) -> NodeId {
    let tree = state.scans.tree(scan).unwrap();
    let mut current = tree.root();

    for part in path.split('/').filter(|part| !part.is_empty()) {
        current = tree
            .children(current)
            .find(|&child| tree.name(child) == part.as_bytes())
            .unwrap_or_else(|| panic!("{path} is in the scan"));
    }

    current
}

/// A disk with two folders the model may be asked about (`/mnt/cache`, `/mnt/photos`), one the rules
/// speak for (`/var/cache/pacman/pkg`), one inside a protected place (`/etc/big`) and one unreadable.
pub(super) fn disk() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/mnt/cache/blob", 400 * MIB)
        .file("/mnt/photos/img", 300 * MIB)
        .file("/mnt/small/x", 10 * MIB)
        .unreadable_dir("/mnt/locked")
        .file("/var/cache/pacman/pkg/a.zst", 500 * MIB)
        .file("/var/cache/pacman/pkg/sub/f", MIB)
        .file("/etc/big/q", 300 * MIB);

    fs
}

/// Statuses until the labelling the watcher saw has ended: the first of them names a scan, the last does
/// not.
fn labelling_events(events: &Receiver<AdvisorStatusDto>) -> Vec<AdvisorStatusDto> {
    let mut seen = Vec::new();
    let mut began = false;
    let started = Instant::now();

    loop {
        let left = WAIT.saturating_sub(started.elapsed());
        let status = events.recv_timeout(left).expect("the status arrives");
        let labelling = status.labelling.is_some();
        seen.push(status);

        began |= labelling;
        if began && !labelling {
            return seen;
        }
    }
}

pub(super) fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();

    while !condition() {
        assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Labels `scan` with the model's next replies and waits until the run has ended and its labels are stored.
pub(super) fn label_and_wait(state: &AppState, control: &FakeControl, scan: ScanId, replies: &[&str]) {
    for reply in replies {
        control.push_reply(reply);
    }

    block_on(label_scan(state, scan)).expect("the job is taken");
    wait_until("the labelling to end", || !state.advisor.is_labelling());
}

pub(super) fn switch(state: &AppState, enabled: bool) {
    switch_service(&state.advisor, enabled);
}

fn switch_service(service: &AdvisorService, enabled: bool) {
    service
        .configure(AdvisorOptionsDto {
            enabled,
            unload_after: UnloadAfterDto::FiveMinutes,
            log_questions: true,
            use_gpu: true,
        })
        .unwrap();
}

pub(super) fn code_of<T: std::fmt::Debug>(result: CommandResult<T>) -> ApiErrorCode {
    result.expect_err("refused").code()
}

#[cfg(target_os = "linux")]
#[test]
fn a_labelling_run_stores_its_labels_and_pushes_progress_naming_the_scan() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    let events = watched(&w.state.advisor);
    w.control.push_reply(REVIEW);
    w.control.push_reply(KEEP);

    block_on(label_scan(&w.state, scan)).expect("the job is taken");
    let seen = labelling_events(&events);

    let progress: Vec<LabellingDto> = seen.iter().filter_map(|status| status.labelling).collect();
    assert!(progress.iter().all(|step| step.scan == scan && step.total == 2));
    assert_eq!(progress.first().map(|step| step.done), Some(0));
    assert_eq!(progress.last().map(|step| step.done), Some(2));

    // The status stops naming the scan only when the labels can be fetched.
    let set = w
        .state
        .scans
        .labels(scan)
        .unwrap()
        .expect("the labels are stored");
    assert_eq!(set.labelled(), 2);
    assert_eq!(
        set.labels
            .iter()
            .map(|label| (label.node, label.verdict))
            .collect::<Vec<_>>(),
        [
            (node_at(&w.state, scan, "/mnt/cache"), ModelVerdict::Review),
            (node_at(&w.state, scan, "/mnt/photos"), ModelVerdict::Keep),
        ],
        "largest folder first, as asked"
    );
    assert_eq!(set.stopped, None);
    assert!(!w.state.advisor.is_labelling());
    assert_eq!(w.state.advisor.status().labelling, None);
    assert_eq!(w.state.advisor.status().tokens_per_second, Some(20.0));
}

#[cfg(target_os = "linux")]
#[test]
fn the_model_is_asked_about_each_candidate_with_the_rules_that_matched_beside_it() {
    let mut fs = disk();
    fs.file("/var/cache/pacman/other/o", 300 * MIB);
    let w = world(true);
    let scan = register(&w.state, &fs);

    label_and_wait(&w.state, &w.control, scan, &[REVIEW, KEEP, KEEP]);

    let prompts = w.control.prompts();
    assert_eq!(prompts.len(), 3, "the largest three, largest first");
    assert!(prompts[0].contains("/mnt/cache"), "{}", prompts[0]);
    let beside = prompts
        .iter()
        .find(|prompt| prompt.contains("/var/cache/pacman/other"))
        .expect("the folder beside the cache is asked about");
    assert!(beside.contains("pacman-cache beside"), "{beside}");
    assert!(!prompts[0].contains("beside"), "{}", prompts[0]);
}

#[test]
fn labelling_is_refused_when_the_scan_the_model_or_the_job_does_not_allow_it() {
    let w = world(true);
    let scan = register(&w.state, &FakeFs::new());
    let (running, _) = w.state.scans.begin().unwrap();

    assert_eq!(
        code_of(block_on(label_scan(&w.state, 4242))),
        ApiErrorCode::NotFound
    );
    assert_eq!(
        code_of(block_on(label_scan(&w.state, running))),
        ApiErrorCode::Invalid,
        "a scan that is still running"
    );

    switch(&w.state, false);
    assert_eq!(
        code_of(block_on(label_scan(&w.state, scan))),
        ApiErrorCode::Invalid,
        "the model is switched off"
    );

    let missing = world(false);
    let scan = register(&missing.state, &FakeFs::new());
    assert_eq!(
        code_of(block_on(label_scan(&missing.state, scan))),
        ApiErrorCode::Invalid,
        "no model file"
    );

    let none = AppState::with_advisor(AdvisorService::unavailable());
    let scan = register(&none, &FakeFs::new());
    assert_eq!(
        code_of(block_on(label_scan(&none, scan))),
        ApiErrorCode::Invalid,
        "no runtime in this build"
    );

    assert!(w.control.calls().is_empty(), "the model was never touched");
}

#[test]
fn labelling_an_outdated_scan_is_refused_and_a_finished_one_with_nothing_to_ask_has_empty_labels() {
    let w = world(true);
    let scan = register(&w.state, &FakeFs::new());

    block_on(label_scan(&w.state, scan)).expect("nothing to ask about is not an error");
    let set = w
        .state
        .scans
        .labels(scan)
        .unwrap()
        .expect("an empty run is stored");
    assert_eq!(set.labelled(), 0);
    assert!(w.control.calls().is_empty(), "no folder, no model");

    w.state.scans.disk_changed();
    assert_eq!(
        code_of(block_on(label_scan(&w.state, scan))),
        ApiErrorCode::Invalid
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_second_job_is_refused_while_labelling_and_cancel_keeps_what_was_labelled() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    let events = watched(&w.state.advisor);
    w.control.push_reply(REVIEW);
    w.control.push_stall();

    block_on(label_scan(&w.state, scan)).expect("the job is taken");
    assert!(w.state.advisor.is_labelling());

    assert_eq!(
        code_of(block_on(label_scan(&w.state, scan))),
        ApiErrorCode::Invalid
    );
    // The row "Ask" uses no model, so labelling does not hold it back.
    let cache = node_at(&w.state, scan, "/mnt/cache");
    assert!(block_on(ask_about(&w.state, scan, cache)).is_ok());

    // One folder is done; the second is stuck.
    let mut done = 0;
    while done < 1 {
        done = events
            .recv_timeout(WAIT)
            .unwrap()
            .labelling
            .map_or(0, |step| step.done);
    }
    assert_eq!(
        serde_json::to_value(w.state.advisor.status()).unwrap()["labelling"],
        json!({ "scan": scan, "done": 1, "total": 2 })
    );

    w.state.advisor.cancel_labelling();
    wait_until("the job to end", || !w.state.advisor.is_labelling());

    let set = w
        .state
        .scans
        .labels(scan)
        .unwrap()
        .expect("a stopped run keeps its labels");
    assert_eq!(set.labelled(), 1);
    assert_eq!(set.stopped, Some(platter_ai::Stopped::Cancelled));
    let shown = block_on(advisor_labels(&w.state, scan)).unwrap();
    assert_eq!((shown.labelled, shown.stopped), (1, Some(StoppedDto::Cancelled)));
}

#[test]
fn a_job_the_worker_refuses_leaves_no_claim_and_the_watcher_hears_that_it_is_over() {
    let rig = rig(true);
    let service = Arc::new(rig.service);
    let items = vec![(NodeId(1), FolderFacts::default())];
    let events = watched(&service);
    switch_service(&service, false);
    events.recv_timeout(WAIT).expect("the options changed");

    // Past the check the web view's call makes first, as when the model is switched off in between.
    let refused = service.start_labelling(1, items, |_| panic!("nothing was labelled"));

    assert_eq!(refused.unwrap_err().code(), ApiErrorCode::Invalid);
    assert!(!service.is_labelling());
    let told = events.recv_timeout(WAIT).expect("the watcher is told");
    assert_eq!(told.labelling, None);
    assert!(rig.control.calls().is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn a_new_run_replaces_the_labels_of_the_old_one() {
    let w = world(true);
    let scan = register(&w.state, &disk());

    label_and_wait(&w.state, &w.control, scan, &[REVIEW, KEEP]);
    label_and_wait(&w.state, &w.control, scan, &[KEEP, KEEP]);

    let set = w.state.scans.labels(scan).unwrap().unwrap();
    assert!(set.labels.iter().all(|label| label.verdict == ModelVerdict::Keep));
}

#[cfg(target_os = "linux")]
#[test]
fn a_run_cancelled_before_it_labelled_anything_keeps_the_labels_of_the_earlier_run() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    label_and_wait(&w.state, &w.control, scan, &[REVIEW, KEEP]);
    let earlier = w.state.scans.labels(scan).unwrap().unwrap();
    let asked = w.control.judges();

    w.control.push_stall();
    block_on(label_scan(&w.state, scan)).expect("the job is taken");
    wait_until("the model to be asked", || w.control.judges() > asked);
    w.state.advisor.cancel_labelling();
    wait_until("the job to end", || !w.state.advisor.is_labelling());

    let kept = w.state.scans.labels(scan).unwrap().unwrap();
    assert_eq!(
        kept, earlier,
        "the labels, the time and the end of the earlier run"
    );
    let shown = block_on(advisor_labels(&w.state, scan)).unwrap();
    assert_eq!((shown.labelled, shown.stopped), (2, None));
}

#[cfg(target_os = "linux")]
#[test]
fn a_run_that_fails_keeps_the_labels_of_the_earlier_run() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    label_and_wait(&w.state, &w.control, scan, &[REVIEW, KEEP]);
    let earlier = w.state.scans.labels(scan).unwrap().unwrap();

    // The model is gone from memory and cannot be loaded again: the job fails before it asks anything.
    w.state.advisor.unload().unwrap();
    w.control.fail_next_load("not enough memory");
    let _ = block_on(label_scan(&w.state, scan));
    wait_until("the job to end", || !w.state.advisor.is_labelling());

    assert_eq!(w.state.scans.labels(scan).unwrap().unwrap(), earlier);
}

#[cfg(target_os = "linux")]
#[test]
fn a_run_that_asked_about_every_folder_replaces_the_earlier_labels_even_with_none() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    label_and_wait(&w.state, &w.control, scan, &[REVIEW, KEEP]);

    label_and_wait(&w.state, &w.control, scan, &["not an answer", "nor this"]);

    let set = w.state.scans.labels(scan).unwrap().unwrap();
    assert_eq!((set.labelled(), set.stopped), (0, None));
}

#[cfg(target_os = "linux")]
#[test]
fn the_labels_of_a_scan_go_when_the_disk_changes() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    label_and_wait(&w.state, &w.control, scan, &[REVIEW, KEEP]);

    w.state.scans.disk_changed();

    let shown = block_on(advisor_labels(&w.state, scan)).unwrap();
    assert_eq!((shown.labelled, shown.labels.len()), (0, 0));
}

#[cfg(target_os = "linux")]
#[test]
fn the_labels_are_sent_in_the_contracts_shape_with_the_three_step_counts() {
    let World {
        _root,
        state,
        control,
    } = world(true);
    let scan = register(&state, &disk());
    let (_app, window) = app_over(state);

    let before = invoke(&window, "get_advisor_labels", json!({ "scan": scan })).unwrap();
    assert_eq!(before["scan"], scan);
    assert_eq!(
        (&before["labelled"], &before["candidates"]),
        (&json!(0), &json!(2))
    );
    assert_eq!(before["labels"], json!([]));
    assert_eq!(before["modelMs"], 0);
    assert!(before.get("stopped").is_none());
    assert_eq!(before["model"], json!({ "folders": 0, "allocated": 0 }));

    control.push_reply(REVIEW);
    control.push_reply(KEEP);
    assert_eq!(
        invoke(&window, "label_folders", json!({ "scan": scan })),
        Ok(Value::Null)
    );
    let state = window.state::<AppState>();
    wait_until("the labelling to end", || !state.advisor.is_labelling());

    let shown = invoke(&window, "get_advisor_labels", json!({ "scan": scan })).unwrap();
    assert_eq!(shown["scan"], scan);
    assert_eq!((&shown["labelled"], &shown["candidates"]), (&json!(2), &json!(2)));
    assert!(shown["modelMs"].is_u64());
    assert!(shown.get("stopped").is_none());
    assert_eq!(shown["rules"]["folders"], 1);
    assert_eq!(shown["rules"]["allocated"], 501 * MIB);
    assert_eq!(
        shown["rules"]["example"],
        json!({ "path": "/var/cache/pacman/pkg", "verdict": "review" })
    );
    assert_eq!(
        shown["guard"]["folders"], 2,
        "/etc/big and the unreadable /mnt/locked"
    );
    assert!(shown["guard"]["allocated"].as_u64().unwrap() >= 300 * MIB);
    assert_eq!(
        shown["guard"]["example"],
        json!({ "path": "/etc/big", "verdict": "keep" })
    );
    assert_eq!(shown["model"]["folders"], 2);
    assert_eq!(shown["model"]["allocated"], 700 * MIB);

    for label in shown["labels"].as_array().unwrap() {
        let mut keys: Vec<&str> = label.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["allocated", "node", "path", "reason", "verdict"]);
    }
    assert_eq!(shown["labels"][0]["path"], "/mnt/cache");
    assert_eq!(shown["labels"][0]["allocated"], 400 * MIB);
    assert_eq!(shown["labels"][0]["verdict"], "review");
    assert_eq!(shown["labels"][0]["reason"], "Cache the system rebuilds.");
    assert_eq!(shown["labels"][1]["path"], "/mnt/photos");
    assert_eq!(shown["labels"][1]["verdict"], "keep");
    assert_eq!(
        shown["model"]["example"],
        json!({ "path": "/mnt/cache", "verdict": "review" })
    );

    let error = invoke(&window, "get_advisor_labels", json!({ "scan": 4242 })).unwrap_err();
    assert_eq!(error["code"], "notFound");
}

/// The same commands over Tauri's IPC, on a state the caller made.
#[cfg(target_os = "linux")]
fn app_over(state: AppState) -> (App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let app = mock_builder()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            label_folders,
            cancel_labelling,
            get_advisor_labels,
            ask_advisor,
            find_folders,
            find_entries,
            crate::commands::suggestions::get_suggestions,
        ])
        .build(tauri::generate_context!())
        .expect("the app builds");
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("the main window opens");

    (app, window)
}

/// The answer as the web view gets it, to look for words that must not be in it.
fn wire(answer: &AdvisorAnswerDto) -> String {
    serde_json::to_string(answer).unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_is_answered_by_the_guard_first_and_it_says_it_is_left_alone() {
    let w = world(true);
    let scan = register(&w.state, &disk());

    for (path, part) in [
        ("/", "scanned"),
        ("/etc/big", "protected place"),
        ("/mnt/locked", "could not read"),
    ] {
        let node = node_at(&w.state, scan, path);

        let answer = block_on(ask_about(&w.state, scan, node)).unwrap();

        assert_eq!(answer.source, AnswerSourceDto::Guard, "{path}");
        assert_eq!(answer.verdict, Verdict::Keep, "{path}");
        assert!(
            answer.reason.starts_with("Left alone by Platter. "),
            "{path}: {}",
            answer.reason
        );
        assert!(answer.reason.contains(part), "{path}: {}", answer.reason);
        assert_eq!(answer.path, path);
        assert_eq!(answer.node, node);
    }
    assert_eq!(w.control.judges(), 0, "the model is not asked");

    // The size is said, except for a folder that could not be read.
    let big = block_on(ask_about(&w.state, scan, node_at(&w.state, scan, "/etc/big"))).unwrap();
    assert!(big.detail.starts_with("3"), "{}", big.detail);
    assert!(big.detail.contains(" MB."), "{}", big.detail);
    let locked = block_on(ask_about(&w.state, scan, node_at(&w.state, scan, "/mnt/locked"))).unwrap();
    assert_eq!(
        locked.detail,
        "Platter could not read it, so its size is unknown."
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_a_rule_covers_is_answered_by_the_rule_also_from_inside_it() {
    let w = world(true);
    let scan = register(&w.state, &disk());

    for path in ["/var/cache/pacman/pkg", "/var/cache/pacman/pkg/sub"] {
        let answer = block_on(ask_about(&w.state, scan, node_at(&w.state, scan, path))).unwrap();

        assert_eq!(answer.source, AnswerSourceDto::Rule, "{path}");
        assert_eq!(answer.verdict, Verdict::Review, "{path}");
        assert!(!answer.reason.is_empty());
        assert!(
            answer.detail.starts_with("Rule: ") && answer.detail.ends_with('.'),
            "{}",
            answer.detail
        );
    }

    // The rule's title and description, then the size and the share older than a year.
    let answer = block_on(ask_about(
        &w.state,
        scan,
        node_at(&w.state, scan, "/var/cache/pacman/pkg"),
    ))
    .unwrap();
    assert!(
        answer.detail.starts_with("Rule: ") && answer.detail.contains(". 5"),
        "{}",
        answer.detail
    );
    assert!(answer.detail.contains(" MB. "), "{}", answer.detail);
    assert!(answer.detail.contains("older than a year."), "{}", answer.detail);
    assert_eq!(w.control.judges(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn the_size_is_written_in_the_units_the_web_view_asks_for() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    let pacman = node_at(&w.state, scan, "/var/cache/pacman/pkg");

    let decimal = block_on(ask_about_in(&w.state, scan, pacman, SizeUnitsDto::Gb)).unwrap();
    let binary = block_on(ask_about_in(&w.state, scan, pacman, SizeUnitsDto::Gib)).unwrap();

    assert!(decimal.detail.contains(" MB"), "{}", decimal.detail);
    assert!(binary.detail.contains(" MiB"), "{}", binary.detail);
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_labelled_for_review_or_to_keep_says_so_without_the_models_reason() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    label_and_wait(&w.state, &w.control, scan, &[REVIEW, KEEP]);
    let asked = w.control.judges();

    let review = block_on(ask_about(&w.state, scan, node_at(&w.state, scan, "/mnt/cache"))).unwrap();
    let keep = block_on(ask_about(&w.state, scan, node_at(&w.state, scan, "/mnt/photos"))).unwrap();

    assert_eq!(
        (review.source, review.verdict),
        (AnswerSourceDto::Model, Verdict::Review)
    );
    assert_eq!(
        review.reason,
        "Platter has no rule for it. Deleting it is your call. The local model marked it for review."
    );
    assert_eq!(
        (keep.source, keep.verdict),
        (AnswerSourceDto::Model, Verdict::Keep)
    );
    assert_eq!(
        keep.reason,
        "Platter has no rule for it. Deleting it is your call. The local model marked it to keep."
    );
    for (answer, said) in [
        (&review, "Cache the system rebuilds."),
        (&keep, "Personal photos."),
    ] {
        assert!(!wire(answer).contains(said), "{}", wire(answer));
        assert!(answer.detail.contains(" MB."), "{}", answer.detail);
    }
    assert_eq!(w.control.judges(), asked, "the model is not asked again");
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_no_rule_covers_and_the_model_has_not_labelled_is_your_call() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    let photos = node_at(&w.state, scan, "/mnt/photos");

    let answer = block_on(ask_about(&w.state, scan, photos)).unwrap();

    assert_eq!(
        (answer.source, answer.verdict, answer.reason.as_str()),
        (
            AnswerSourceDto::Unruled,
            Verdict::Review,
            "Platter has no rule for it. Deleting it is your call."
        )
    );
    assert_eq!(answer.path, "/mnt/photos");
    assert!(!answer.detail.is_empty());
    assert_eq!(serde_json::to_value(&answer).unwrap()["source"], "unruled");
    assert_eq!(w.control.judges(), 0, "the model is not asked");
    assert_eq!(w.state.scans.labels(scan).unwrap(), None, "nothing is stored");
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_is_answered_with_the_model_off_not_downloaded_or_not_built_in() {
    let ask = |state: &AppState, scan: ScanId| {
        ["/", "/var/cache/pacman/pkg", "/mnt/photos"]
            .map(|path| block_on(ask_about(state, scan, node_at(state, scan, path))))
    };

    let off = world(true);
    let scan = register(&off.state, &disk());
    switch(&off.state, false);
    assert!(ask(&off.state, scan).iter().all(Result::is_ok));
    assert_eq!(off.control.judges(), 0);

    let missing = world(false);
    let scan = register(&missing.state, &disk());
    assert!(ask(&missing.state, scan).iter().all(Result::is_ok));

    let unavailable = AppState::with_advisor(AdvisorService::unavailable());
    let scan = register(&unavailable, &disk());
    assert!(ask(&unavailable, scan).iter().all(Result::is_ok));
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_ask_is_not_held_back_by_a_free_question_that_holds_the_model() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    let held = w.state.advisor.begin_ask().unwrap();

    let answer = block_on(ask_about(&w.state, scan, node_at(&w.state, scan, "/mnt/photos")));

    assert!(answer.is_ok());
    drop(held);
}

#[cfg(target_os = "linux")]
#[test]
fn only_a_known_folder_can_be_asked_about() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    let file = node_at(&w.state, scan, "/mnt/photos/img");

    assert_eq!(
        code_of(block_on(ask_about(&w.state, scan, file))),
        ApiErrorCode::Invalid
    );
    assert_eq!(
        code_of(block_on(ask_about(&w.state, scan, NodeId(99_999)))),
        ApiErrorCode::NotFound
    );
    assert_eq!(
        code_of(block_on(ask_about(&w.state, 4242, NodeId(1)))),
        ApiErrorCode::NotFound
    );
}

#[cfg(target_os = "linux")]
#[test]
fn an_answer_is_sent_in_the_contracts_shape() {
    let w = world(true);
    let scan = register(&w.state, &disk());
    let node = node_at(&w.state, scan, "/var/cache/pacman/pkg");
    let (_app, window) = app_over(w.state);

    let answer = invoke(&window, "ask_advisor", json!({ "scan": scan, "node": node })).unwrap();

    let mut keys: Vec<&str> = answer.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["detail", "node", "path", "reason", "source", "verdict"]);
    assert_eq!(answer["source"], "rule");
    assert_eq!(answer["verdict"], "review");
    assert_eq!(answer["path"], "/var/cache/pacman/pkg");
    assert!(answer["detail"].as_str().unwrap().contains(" MB. "));

    // `units` is optional: the size is then decimal, and `GiB` writes it in binary units.
    let binary = invoke(
        &window,
        "ask_advisor",
        json!({ "scan": scan, "node": node, "units": "GiB" }),
    )
    .unwrap();
    assert!(binary["detail"].as_str().unwrap().contains(" MiB. "));
    assert!(
        invoke(
            &window,
            "ask_advisor",
            json!({ "scan": scan, "node": node, "units": "TB" }),
        )
        .is_err()
    );
}

fn many_folders() -> FakeFs {
    let mut fs = FakeFs::new();
    for number in 1..=25u64 {
        fs.file(&format!("/d/app{number}/f"), number);
    }
    fs.file("/d/other/f", 1000).file("/d/app3.txt", 5);

    fs
}

#[test]
fn finding_folders_returns_at_most_twenty_and_at_least_one() {
    let w = world(true);
    let scan = register(&w.state, &many_folders());
    let find_with = |limit| block_on(find(&w.state, scan, "app".to_owned(), limit)).unwrap();

    assert_eq!(find_with(100).len(), 20);
    assert_eq!(find_with(20).len(), 20);
    assert_eq!(find_with(5).len(), 5);
    assert_eq!(find_with(0).len(), 1, "a limit of 0 is raised to 1");
    assert_eq!(find_with(u32::MAX).len(), 20);
}

#[test]
fn a_search_text_of_more_than_1024_bytes_is_invalid_and_1024_is_fine() {
    let w = world(true);
    let scan = register(&w.state, &many_folders());

    assert_eq!(
        code_of(block_on(find(&w.state, scan, "a".repeat(1025), 5))),
        ApiErrorCode::Invalid
    );
    // The limit counts bytes, not characters.
    assert_eq!(
        code_of(block_on(find(&w.state, scan, "é".repeat(513), 5))),
        ApiErrorCode::Invalid
    );
    assert_eq!(
        block_on(find(&w.state, scan, "a".repeat(1024), 5)).unwrap(),
        Vec::new()
    );
}

#[test]
fn finding_folders_matches_names_and_exact_paths_and_a_blank_text_finds_nothing() {
    let w = world(true);
    let scan = register(&w.state, &many_folders());

    let by_path = block_on(find(&w.state, scan, "/d/app3".to_owned(), 3)).unwrap();
    assert_eq!(by_path[0].path, "/d/app3", "the exact folder comes first");
    assert_eq!(by_path[0].allocated, 3);

    let by_name = block_on(find(&w.state, scan, "OTH".to_owned(), 5)).unwrap();
    assert_eq!(
        by_name,
        [FolderMatchDto {
            node: node_at(&w.state, scan, "/d/other"),
            path: "/d/other".to_owned(),
            allocated: 1000,
        }]
    );
    assert_eq!(
        block_on(find(&w.state, scan, "  ".to_owned(), 5)).unwrap(),
        Vec::new()
    );
    assert_eq!(
        code_of(block_on(find(&w.state, 4242, "app".to_owned(), 5))),
        ApiErrorCode::NotFound
    );
}

#[test]
fn found_folders_are_sent_in_the_contracts_shape() {
    let w = world(true);
    let scan = register(&w.state, &many_folders());
    let other = node_at(&w.state, scan, "/d/other");
    let (_app, window) = app_over(w.state);

    let found = invoke(
        &window,
        "find_folders",
        json!({ "scan": scan, "query": "other", "limit": 3 }),
    )
    .unwrap();

    assert_eq!(
        found,
        json!([{ "node": other, "path": "/d/other", "allocated": 1000 }])
    );
    let refused = invoke(
        &window,
        "find_folders",
        json!({ "scan": scan, "query": "x".repeat(1025), "limit": 3 }),
    )
    .unwrap_err();
    assert_eq!(refused["code"], "invalid");
}

#[cfg(target_os = "linux")]
#[test]
fn found_entries_are_sent_with_their_kind_and_follow_the_limits_of_found_folders() {
    let w = world(true);
    let scan = register(&w.state, &many_folders());
    let folder = node_at(&w.state, scan, "/d/app3");
    let file = node_at(&w.state, scan, "/d/app3.txt");
    let (_app, window) = app_over(w.state);

    let found = invoke(
        &window,
        "find_entries",
        json!({ "scan": scan, "query": "app3.", "limit": 5 }),
    )
    .unwrap();
    assert_eq!(
        found,
        json!([{ "node": file, "path": "/d/app3.txt", "allocated": 5, "kind": "file" }])
    );

    let both = invoke(
        &window,
        "find_entries",
        json!({ "scan": scan, "query": "/d/app3", "limit": 5 }),
    )
    .unwrap();
    assert_eq!(
        both[0],
        json!({ "node": folder, "path": "/d/app3", "allocated": 3, "kind": "dir" }),
        "the folder at the exact path comes first, as a dir"
    );

    // At most twenty and at least one. The text is at most 1024 bytes.
    let counted = |limit: u32| {
        invoke(
            &window,
            "find_entries",
            json!({ "scan": scan, "query": "app", "limit": limit }),
        )
        .unwrap()
        .as_array()
        .unwrap()
        .len()
    };
    assert_eq!(counted(100), 20);
    assert_eq!(counted(0), 1);
    let refused = invoke(
        &window,
        "find_entries",
        json!({ "scan": scan, "query": "x".repeat(1025), "limit": 3 }),
    )
    .unwrap_err();
    assert_eq!(refused["code"], "invalid");
    let unknown = invoke(
        &window,
        "find_entries",
        json!({ "scan": 4242, "query": "app", "limit": 3 }),
    )
    .unwrap_err();
    assert_eq!(unknown["code"], "notFound");
}

#[test]
fn a_search_that_a_newer_one_has_overtaken_stops_and_finds_nothing() {
    let w = world(true);
    let scan = register(&w.state, &many_folders());

    // Two searches start one after the other: the second takes the higher number.
    let older = w.state.entry_searches.fetch_add(1, Ordering::SeqCst) + 1;
    let newer = w.state.entry_searches.fetch_add(1, Ordering::SeqCst) + 1;

    let older = block_on(find_entries_numbered(&w.state, scan, "app".to_owned(), 5, older)).unwrap();
    let newer = block_on(find_entries_numbered(&w.state, scan, "app".to_owned(), 5, newer)).unwrap();
    assert!(older.is_empty(), "the overtaken search found {older:?}");
    assert_eq!(newer.len(), 5);

    // The calls of the command take their numbers in turn, and each one finds its answer when no other
    // started meanwhile.
    for _ in 0..2 {
        let found = block_on(find_entries_in(&w.state, scan, "app".to_owned(), 5)).unwrap();
        assert_eq!(found.len(), 5);
    }
    assert_eq!(w.state.entry_searches.load(Ordering::SeqCst), 4);
}

#[test]
fn a_row_and_a_scope_say_they_are_a_file_only_when_they_are() {
    use self::question::{ScanRowDto, ScopeDto};

    let row = |file| ScanRowDto {
        node: NodeId(1),
        path: "~/a".to_owned(),
        allocated: 1,
        verdict: None,
        source: None,
        note: None,
        file,
    };
    let scope = |file| ScopeDto {
        node: NodeId(1),
        path: "~/a".to_owned(),
        file,
    };

    assert_eq!(
        serde_json::to_value(row(false)).unwrap(),
        json!({ "node": 1, "path": "~/a", "allocated": 1 })
    );
    assert_eq!(
        serde_json::to_value(row(true)).unwrap(),
        json!({ "node": 1, "path": "~/a", "allocated": 1, "file": true })
    );
    assert_eq!(
        serde_json::to_value(scope(false)).unwrap(),
        json!({ "node": 1, "path": "~/a" })
    );
    assert_eq!(
        serde_json::to_value(scope(true)).unwrap(),
        json!({ "node": 1, "path": "~/a", "file": true })
    );
}

#[test]
fn asking_about_a_file_is_still_refused_as_only_a_folder_can_be_asked_about() {
    let w = world(true);
    let scan = register(&w.state, &many_folders());

    let file = node_at(&w.state, scan, "/d/app3.txt");
    let refused = block_on(ask_about(&w.state, scan, file));

    assert_eq!(code_of(refused), ApiErrorCode::Invalid);
    assert!(block_on(ask_about(&w.state, scan, node_at(&w.state, scan, "/d/app3"))).is_ok());
}

fn advisor_suggestion(suggestions: &Value) -> Option<&Value> {
    suggestions["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|suggestion| suggestion["id"] == "advisor")
}

#[cfg(target_os = "linux")]
#[test]
fn the_advisor_group_is_listed_only_with_review_labels_and_only_while_the_advisor_is_on() {
    let World {
        _root,
        state,
        control,
    } = world(true);
    let scan = register(&state, &disk());
    let cache = node_at(&state, scan, "/mnt/cache");
    let (_app, window) = app_over(state);
    let state = window.state::<AppState>();
    let suggestions = || invoke(&window, "get_suggestions", json!({ "scan": scan })).unwrap();

    let before = suggestions();
    assert!(advisor_suggestion(&before).is_none(), "no labels, no group");
    assert_eq!(before["suggestions"][0]["source"], "rule");
    assert_eq!(before["totals"]["review"], 501 * MIB);

    label_and_wait(&state, &control, scan, &[REVIEW, KEEP]);
    let listed = suggestions();
    let group = advisor_suggestion(&listed).expect("the review label is listed");
    assert_eq!(group["source"], "model");
    assert_eq!(group["verdict"], "review");
    assert_eq!(group["title"], "Labelled by the local model");
    assert_eq!(
        (&group["items"], &group["allocated"]),
        (&json!(1), &json!(400 * MIB))
    );
    assert_eq!(group["nodes"], json!([cache]), "the keep label is not listed");
    assert_eq!(listed["totals"]["review"], 901 * MIB);

    switch(&state, false);
    let off = suggestions();
    assert!(advisor_suggestion(&off).is_none(), "the advisor is switched off");
    assert_eq!(off["totals"]["review"], 501 * MIB);

    switch(&state, true);
    assert_eq!(suggestions(), listed, "and back on");

    label_and_wait(&state, &control, scan, &[KEEP, KEEP]);
    assert!(
        advisor_suggestion(&suggestions()).is_none(),
        "only keep labels: nothing to list"
    );

    label_and_wait(&state, &control, scan, &[REVIEW, KEEP]);
    assert!(advisor_suggestion(&suggestions()).is_some());
    state.scans.disk_changed();
    assert!(
        advisor_suggestion(&suggestions()).is_none(),
        "the labels went with the outdated scan"
    );
}

/// The folder of `kind` as this machine knows it, when it has one (it needs a home directory).
fn personal_folder(kind: UserDir) -> Option<String> {
    let env = platter_platform::Platform::current().environment();

    Some(env.user_dir(kind)?.to_string_lossy().into_owned())
}

#[cfg(target_os = "linux")]
#[test]
fn a_label_is_listed_only_where_no_rule_has_an_opinion() {
    let Some(documents) = personal_folder(UserDir::Documents) else {
        return;
    };
    let mut fs = disk();
    fs.file(&format!("{documents}/thesis/chapter"), 300 * MIB)
        .file("/mnt/cache/sub/deep", MIB);
    let w = world(true);
    let scan = register(&w.state, &fs);
    let tree = w.state.scans.tree(scan).unwrap();
    let label = |path: &str, verdict: ModelVerdict| ModelLabel {
        node: node_at(&w.state, scan, path),
        verdict,
        reason: "because".to_owned(),
    };
    let labels = [
        label("/mnt/cache", ModelVerdict::Review),
        label("/mnt/cache/sub", ModelVerdict::Review),
        label("/mnt/photos", ModelVerdict::Keep),
        label("/var/cache/pacman", ModelVerdict::Review),
        label("/var/cache/pacman/pkg", ModelVerdict::Review),
        label("/var/cache/pacman/pkg/sub", ModelVerdict::Review),
        label(&format!("{documents}/thesis"), ModelVerdict::Review),
        label("/", ModelVerdict::Review),
        label("/mnt/locked", ModelVerdict::Review),
    ];

    let listed = Review::new(&tree, &w.state.platform, Vec::new()).listed_labels(&labels);

    assert_eq!(
        listed.iter().map(|label| label.node).collect::<Vec<_>>(),
        [node_at(&w.state, scan, "/mnt/cache")],
        "not a keep label, not one inside another, not one a rule covers or holds a match of or a kept \
         folder holds, not the scan root, not an unreadable folder"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_scan_that_starts_inside_a_kept_folder_lists_no_label_and_the_guard_answers_every_question() {
    let Some(documents) = personal_folder(UserDir::Documents) else {
        return;
    };
    let mut fs = FakeFs::new();
    fs.file(&format!("{documents}/work/big/x"), 400 * MIB);
    let w = world(true);
    let (id, _) = w.state.scans.begin().unwrap();
    let tree = {
        let config = platter_core::ScanConfig {
            now: Some(platter_core::testing::NOW),
            ..platter_core::ScanConfig::default()
        };
        platter_core::scan(
            &fs,
            std::path::Path::new(&format!("{documents}/work")),
            &config,
            &platter_core::CancelToken::new(),
            &|_| {},
        )
        .expect("the fake scans")
    };
    w.state.scans.finish(id, tree).unwrap();
    let big = node_at(&w.state, id, "/big");
    let tree = w.state.scans.tree(id).unwrap();
    let labels = [ModelLabel {
        node: big,
        verdict: ModelVerdict::Review,
        reason: "x".to_owned(),
    }];

    assert!(
        Review::new(&tree, &w.state.platform, Vec::new())
            .listed_labels(&labels)
            .is_empty()
    );
    let answer = block_on(ask_about(&w.state, id, big)).unwrap();
    assert_eq!(answer.source, AnswerSourceDto::Guard);
    assert_eq!(answer.verdict, Verdict::Keep);
    assert!(w.control.calls().is_empty());
}

/// A disk where `link` is a link to `to`, looked at through a fake.
struct LinkProbe<'a> {
    fs: &'a FakeFs,
    link: &'static str,
    to: &'static str,
}

impl EntryProbe for LinkProbe<'_> {
    fn probe(&self, path: &[u8]) -> std::io::Result<Vec<platter_core::ProbedEntry>> {
        self.fs.probe(path)
    }

    fn resolve(&self, path: &[u8]) -> std::io::Result<Vec<u8>> {
        let leads = if path == self.link.as_bytes() {
            self.to.as_bytes()
        } else {
            path
        };

        Ok(leads.to_vec())
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_kept_through_a_link_is_no_label_candidate_and_no_label_in_it_is_listed_or_refused() {
    let mut fs = disk();
    fs.file("/mnt/cache/sub/deep", 300 * MIB);
    let state = AppState::for_test();
    let scan = register(&state, &fs);
    let tree = state.scans.tree(scan).unwrap();
    let probe = LinkProbe {
        fs: &fs,
        link: "/home/link",
        to: "/mnt/cache",
    };
    let user = vec![UserRule {
        id: 1,
        verdict: UserVerdict::Keep,
        folder: "/home/link".into(),
    }];
    let label = |path: &str| ModelLabel {
        node: node_at(&state, scan, path),
        verdict: ModelVerdict::Review,
        reason: "because".to_owned(),
    };
    let labels = [label("/mnt/cache"), label("/mnt/cache/sub"), label("/mnt/photos")];

    let through_link = Review::with_probe(&tree, &state.platform, &probe, user.clone());
    let literal = Review::with_probe(
        &tree,
        &state.platform,
        &LinkProbe {
            fs: &fs,
            link: "/home/link",
            to: "/home/link",
        },
        user.clone(),
    );

    // The folder is kept where it really is: its contents are not sent to the model.
    let candidates: Vec<NodeId> = through_link
        .label_items()
        .into_iter()
        .map(|(node, _)| node)
        .collect();
    assert!(
        candidates.contains(&node_at(&state, scan, "/mnt/photos")),
        "{candidates:?}"
    );
    assert!(
        !candidates.contains(&node_at(&state, scan, "/mnt/cache")),
        "the kept folder is no candidate: {candidates:?}"
    );
    // Without knowing where the link leads the folder is an ordinary candidate (the control).
    let ordinary: Vec<NodeId> = literal.label_items().into_iter().map(|(node, _)| node).collect();
    assert!(
        ordinary.contains(&node_at(&state, scan, "/mnt/cache")),
        "{ordinary:?}"
    );

    // Labels in it are not listed, and so the plan made from the listed ones refuses nothing the
    // suggestion did not list.
    let listed = through_link.listed_labels(&labels);
    assert_eq!(
        listed.iter().map(|label| label.node).collect::<Vec<_>>(),
        [node_at(&state, scan, "/mnt/photos")]
    );
    let rules = RuleSet::builtin().with_user(user);
    let plan = platter_cleanup::plan_labelled(
        &tree,
        &state.platform.environment_for(tree.root_path()),
        &rules,
        &listed,
        &probe,
        &platter_cleanup::CleanupOptions::default(),
    );
    assert!(plan.refused.is_empty(), "{plan:#?}");
    assert_eq!(plan.targets().count(), 1, "{plan:#?}");
}

#[test]
fn where_the_model_runs_is_sent_as_a_closed_kind_and_the_cards_name() {
    assert_eq!(
        serde_json::to_value(DeviceDto::from(&Device::Gpu {
            name: "AMD Radeon RX 7900 XTX (RADV NAVI31)".to_owned()
        }))
        .unwrap(),
        json!({ "kind": "gpu", "name": "AMD Radeon RX 7900 XTX (RADV NAVI31)" })
    );
    assert_eq!(
        serde_json::to_value(DeviceDto::from(&Device::Cpu)).unwrap(),
        json!({ "kind": "cpu" })
    );
}
