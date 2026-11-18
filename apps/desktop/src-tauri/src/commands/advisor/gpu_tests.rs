//! Graphics card problems over the service and Tauri's IPC: what reaches the status and the log, the answers the
//! user can give, and the log's deletion. The model is the fake one, in the worker's own thread: no helper
//! process starts and no graphics card is touched.

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use platter_ai::Placement;
use platter_ai::testing::{FakeEngine, FakeFetch, card, tiny_spec};
use platter_core::dto::ApiErrorCode;
use serde_json::{Value, json};

use super::tests::{Dialog, Rig, app, app_asking, content, invoke};
use super::*;

const XTX: &str = "AMD Radeon RX 7900 XTX (RADV NAVI31)";

fn options(use_gpu: bool) -> AdvisorOptionsDto {
    AdvisorOptionsDto {
        enabled: true,
        unload_after: UnloadAfterDto::FiveMinutes,
        log_questions: true,
        use_gpu,
    }
}

/// An advisor on the fake model with a discrete card, writing its graphics card problems to `log`; the model
/// file is on disk and the advisor is switched on, as the web view does at start.
fn rig_with(log: GpuLog) -> Rig {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("models");
    let spec = tiny_spec(&content());
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(spec.file), content()).unwrap();

    let mut config = WorkerConfig::new(spec, dir.clone());
    config.threads = 2;
    config.progress_every = Duration::ZERO;
    let (engine, control) = FakeEngine::new();
    control.set_gpus(vec![card(1, XTX, 23_000, false)]);
    let fetch = Arc::new(FakeFetch::new([]));
    let service =
        AdvisorService::spawn_logged(config, Box::new(engine), fetch.clone(), Arc::new(log)).unwrap();
    service.configure(options(true)).unwrap();

    Rig {
        _root: root,
        dir,
        service,
        control,
        fetch,
    }
}

/// A log in its own folder.
fn log_in(root: &tempfile::TempDir) -> (GpuLog, PathBuf) {
    let path = root.path().join("data/platter").join(gpu_log::FILE_NAME);

    (GpuLog::open(path.clone()), path)
}

/// The card fails to answer: the question is read on the processor, and a problem on the card is logged.
fn the_card_fails(rig: &Rig) {
    rig.control.fail_gpu_runs("vk::DeviceLostError");
    rig.control.set_reply("{}");

    let _ = rig.service.interpret("biggest caches in ~/Projekty");
}

fn lines(path: &PathBuf) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_failure_on_the_card_reaches_the_status_the_log_and_the_watcher() {
    let logs = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&logs);
    let rig = rig_with(log);
    let (sender, receiver) = mpsc::channel();
    rig.service.watch(Arc::new(move |status| {
        let _ = sender.send(status);
    }));

    the_card_fails(&rig);

    let status = rig.service.status();
    let problem = status.gpu_problem.clone().expect("a problem waits for an answer");
    assert_eq!(problem.what, GpuProblemWhatDto::Error);
    assert_eq!(problem.card.as_deref(), Some(XTX));
    assert!(problem.id > 0 && problem.at > 0);
    assert_eq!(status.last_gpu_problem, Some(problem.clone()));
    assert_eq!(status.problem.as_deref(), Some(platter_ai::GPU_FALLBACK));
    let incident = &lines(&path)[0];
    assert_eq!(incident["type"], "incident");
    assert_eq!(incident["device"], "gpu");
    assert_eq!(incident["id"], problem.id);

    // The status that names the problem in the text names it as pending too.
    let mut seen = Vec::new();
    while let Ok(change) = receiver.recv_timeout(Duration::from_millis(200)) {
        seen.push(change);
    }
    let first_with_text = seen
        .iter()
        .position(|change| change.problem.as_deref() == Some(platter_ai::GPU_FALLBACK))
        .expect("the change of the text is pushed");
    assert_eq!(seen[first_with_text].gpu_problem, Some(problem));
}

#[test]
fn the_wire_shape_of_a_problem_is_camel_case_and_leaves_out_an_unknown_card() {
    let problem = GpuProblemDto {
        id: 7,
        at: 5,
        card: None,
        what: GpuProblemWhatDto::Hang,
    };
    let named = GpuProblemDto {
        card: Some("a".to_owned()),
        what: GpuProblemWhatDto::Crash,
        ..problem.clone()
    };

    assert_eq!(
        serde_json::to_value(&problem).unwrap(),
        json!({ "id": 7, "at": 5, "what": "hang" })
    );
    assert_eq!(
        serde_json::to_value(&named).unwrap(),
        json!({ "id": 7, "at": 5, "card": "a", "what": "crash" })
    );
}

#[test]
fn answering_tryagain_lets_the_next_load_try_the_card_and_clears_the_prompt() {
    let logs = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&logs);
    let rig = rig_with(log);
    the_card_fails(&rig);
    let id = rig.service.status().gpu_problem.unwrap().id;

    let answered = rig
        .service
        .answer_gpu_problem(id, GpuChoiceDto::TryAgain)
        .unwrap();

    assert!(answered.gpu_problem.is_none());
    assert_eq!(
        answered.last_gpu_problem.map(|problem| problem.id),
        Some(id),
        "still the last one"
    );
    assert_eq!(lines(&path)[1]["choice"], "tryAgain");

    // The model that is loaded stays on the processor; the next load tries the card.
    rig.control.fail_gpu_runs("");
    rig.service.unload().unwrap();
    rig.service.load().unwrap();
    assert_eq!(rig.control.placement(), Placement::Gpu { index: 1 });
}

#[test]
fn answering_usecpu_keeps_the_card_out() {
    let logs = tempfile::tempdir().unwrap();
    let (log, _) = log_in(&logs);
    let rig = rig_with(log);
    the_card_fails(&rig);
    let id = rig.service.status().gpu_problem.unwrap().id;

    rig.service.answer_gpu_problem(id, GpuChoiceDto::UseCpu).unwrap();
    rig.service.unload().unwrap();
    rig.service.load().unwrap();

    assert_eq!(rig.control.placement(), Placement::Cpu);
}

#[test]
fn an_answer_to_a_problem_that_does_not_wait_is_refused() {
    let logs = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&logs);
    let rig = rig_with(log);

    let nothing = rig
        .service
        .answer_gpu_problem(1, GpuChoiceDto::UseCpu)
        .unwrap_err();
    assert_eq!(nothing.code(), ApiErrorCode::Invalid);

    the_card_fails(&rig);
    let id = rig.service.status().gpu_problem.unwrap().id;
    let wrong = rig
        .service
        .answer_gpu_problem(id + 1, GpuChoiceDto::UseCpu)
        .unwrap_err();
    assert_eq!(wrong.code(), ApiErrorCode::Invalid);
    assert!(
        rig.service.status().gpu_problem.is_some(),
        "a wrong id changed nothing"
    );

    rig.service.answer_gpu_problem(id, GpuChoiceDto::UseCpu).unwrap();
    let again = rig
        .service
        .answer_gpu_problem(id, GpuChoiceDto::TryAgain)
        .unwrap_err();
    assert_eq!(again.code(), ApiErrorCode::Invalid, "an answer is given once");
    assert_eq!(lines(&path).len(), 2, "one incident and one answer");
}

#[test]
fn settings_that_already_use_the_processor_answer_a_waiting_problem_without_asking() {
    let logs = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&logs);
    let rig = rig_with(log);
    the_card_fails(&rig);
    assert!(rig.service.status().gpu_problem.is_some());

    let status = rig.service.configure(options(false)).unwrap();

    assert!(status.gpu_problem.is_none());
    assert!(status.last_gpu_problem.is_some());
    let answer = &lines(&path)[1];
    assert_eq!(
        (answer["type"].as_str(), answer["choice"].as_str()),
        (Some("answer"), Some("useCpu"))
    );
}

#[test]
fn a_problem_nobody_answered_blocks_the_card_at_the_next_start_and_asks_again() {
    let logs = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&logs);
    {
        let first = rig_with(log);
        the_card_fails(&first);
    }

    let second = rig_with(GpuLog::open(path));

    assert!(second.service.status().gpu_problem.is_some(), "asked again");
    let status = second.service.load().unwrap();
    assert_eq!(status.device.map(|device| device.kind), Some(DeviceKindDto::Cpu));
    assert_eq!(second.control.placement(), Placement::Cpu);
    assert_eq!(second.control.calls(), ["load 2"], "the card was not even tried");
}

#[test]
fn clearing_the_log_forgets_the_last_problem_and_keeps_the_one_that_waits() {
    let logs = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&logs);
    let rig = rig_with(log);
    assert!(rig.service.clear_gpu_log().is_ok(), "no log yet");
    the_card_fails(&rig);
    let id = rig.service.status().gpu_problem.unwrap().id;

    rig.service.clear_gpu_log().unwrap();

    assert!(!path.exists());
    let status = rig.service.status();
    assert!(status.last_gpu_problem.is_none());
    assert_eq!(status.gpu_problem.map(|problem| problem.id), Some(id));

    // Answering still works, and writes the file again.
    rig.service
        .answer_gpu_problem(id, GpuChoiceDto::TryAgain)
        .unwrap();
    assert_eq!(lines(&path).len(), 1);
}

#[test]
fn the_commands_answer_over_ipc_with_the_contracts_names() {
    let logs = tempfile::tempdir().unwrap();
    let (log, _) = log_in(&logs);
    let rig = rig_with(log);
    the_card_fails(&rig);
    let id = rig.service.status().gpu_problem.unwrap().id;
    let (_app, window) = app(rig.service);

    let status = invoke(&window, "advisor_status", json!({})).unwrap();
    assert_eq!(status["gpuProblem"]["id"], id);
    assert_eq!(status["gpuProblem"]["what"], "error");
    assert_eq!(status["gpuProblem"]["card"], XTX);
    assert_eq!(status["lastGpuProblem"], status["gpuProblem"]);

    // What is not the contract is refused, and changes nothing.
    for refused in [
        json!({ "id": id, "choice": "never" }),
        json!({ "id": id }),
        json!({ "choice": "useCpu" }),
        json!({ "id": -1, "choice": "useCpu" }),
        json!({ "id": "x", "choice": "useCpu" }),
        json!({ "id": id, "choice": "UseCpu" }),
    ] {
        assert!(
            invoke(&window, "answer_gpu_problem", refused.clone()).is_err(),
            "{refused}"
        );
    }
    let stale = invoke(
        &window,
        "answer_gpu_problem",
        json!({ "id": id + 5, "choice": "useCpu" }),
    )
    .unwrap_err();
    assert_eq!(stale["code"], "invalid");
    assert!(invoke(&window, "advisor_status", json!({})).unwrap()["gpuProblem"].is_object());

    let answered = invoke(
        &window,
        "answer_gpu_problem",
        json!({ "id": id, "choice": "tryAgain" }),
    )
    .unwrap();
    assert!(answered.get("gpuProblem").is_none());
    assert_eq!(answered["lastGpuProblem"]["id"], id);

    assert_eq!(invoke(&window, "clear_gpu_log", json!({})), Ok(Value::Null));
    let cleared = invoke(&window, "advisor_status", json!({})).unwrap();
    assert!(cleared.get("lastGpuProblem").is_none());
    assert_eq!(
        invoke(&window, "clear_gpu_log", json!({})),
        Ok(Value::Null),
        "nothing to delete is fine"
    );
}

#[test]
fn an_advisor_without_a_runtime_has_no_graphics_card_problem_and_refuses_an_answer() {
    let service = AdvisorService::unavailable();

    let status = service.status();

    assert!(status.gpu_problem.is_none() && status.last_gpu_problem.is_none());
    assert_eq!(
        service
            .answer_gpu_problem(1, GpuChoiceDto::UseCpu)
            .unwrap_err()
            .code(),
        ApiErrorCode::Invalid
    );
    assert!(service.clear_gpu_log().is_ok());
}

#[test]
fn the_advisor_the_tests_of_the_app_use_has_no_runtime_and_reads_no_log() {
    let service = AdvisorService::for_test();

    let status = service.status();

    assert!(status.gpu_problem.is_none() && status.last_gpu_problem.is_none());
}

#[test]
fn an_unreadable_log_file_does_not_stop_the_advisor() {
    let logs = tempfile::tempdir().unwrap();
    let path = logs.path().join(gpu_log::FILE_NAME);
    fs::create_dir(&path).unwrap();

    let rig = rig_with(GpuLog::open(path));

    assert!(rig.service.status().gpu_problem.is_none());
    assert_eq!(rig.service.load().unwrap().state, ModelStateDto::Loaded);
}

#[test]
fn clearing_the_log_asks_first_and_a_no_keeps_the_last_problem() {
    let logs = tempfile::tempdir().unwrap();
    let (log, _) = log_in(&logs);
    let rig = rig_with(log);
    the_card_fails(&rig);
    let id = rig.service.status().gpu_problem.unwrap().id;
    rig.service
        .answer_gpu_problem(id, GpuChoiceDto::TryAgain)
        .unwrap();
    let dialog = Dialog::answering(false);
    let (_app, window) = app_asking(rig.service, dialog.clone());

    let declined = invoke(&window, "clear_gpu_log", json!({})).unwrap_err();
    assert_eq!(declined["code"], "cancelled");
    assert_eq!(dialog.asked()[0].title, "Clear the graphics card problem log?");
    let status = invoke(&window, "advisor_status", json!({})).unwrap();
    assert_eq!(status["lastGpuProblem"]["id"], id, "the log is kept");
}
