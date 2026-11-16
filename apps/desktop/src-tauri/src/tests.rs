//! The webview's access, checked through Tauri's real IPC path (mock runtime, the app's real
//! `tauri.conf.json` and `capabilities/default.json`): the capability grants nothing, yet the app's own
//! commands answer, while core and plugin commands are refused.

use serde_json::{Value, json};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewWindow, WebviewWindowBuilder};

use crate::commands;
use crate::state::AppState;

/// The commands that do not need a real window (the system ones take a concrete `AppHandle`).
fn app() -> (App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let app = mock_builder()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::volumes::list_volumes,
            commands::scan::start_scan,
            commands::scan::cancel_scan,
            commands::scan::drop_scan,
            commands::suggestions::get_suggestions,
        ])
        .build(tauri::generate_context!())
        .expect("the app builds");
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("the main window opens");

    (app, window)
}

fn invoke(window: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
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

#[test]
fn the_app_commands_work_without_any_granted_permission() {
    let (_app, window) = app();

    assert_eq!(
        invoke(&window, "list_volumes", json!({})).map(|v| v.is_array()),
        Ok(true)
    );
}

#[test]
fn cancelling_an_unknown_scan_succeeds() {
    let (_app, window) = app();

    assert_eq!(
        invoke(&window, "cancel_scan", json!({ "scan": 4242 })),
        Ok(Value::Null)
    );
}

#[test]
fn a_scan_with_a_channel_starts_and_can_be_cancelled_twice() {
    let (_app, window) = app();
    let dir = tempfile::tempdir().unwrap();

    let started = invoke(
        &window,
        "start_scan",
        json!({
            "options": { "root": dir.path(), "oneFileSystem": false },
            "onEvent": "__CHANNEL__:7",
        }),
    )
    .expect("the scan starts");
    let id = started.as_u64().expect("a scan id");

    // The scan may or may not have ended already; cancelling is fine either way, and twice.
    for _ in 0..2 {
        assert_eq!(
            invoke(&window, "cancel_scan", json!({ "scan": id })),
            Ok(Value::Null)
        );
    }
    assert_eq!(
        invoke(&window, "drop_scan", json!({ "scan": id })),
        Ok(Value::Null)
    );
}

#[test]
fn core_and_plugin_commands_are_refused() {
    let (_app, window) = app();

    for cmd in [
        "plugin:event|listen",
        "plugin:event|emit",
        "plugin:image|from_path",
        "plugin:tray|new",
        "plugin:menu|new",
        "plugin:app|version",
        "plugin:window|title",
        "plugin:dialog|open",
    ] {
        let refused = invoke(&window, cmd, json!({})).expect_err(cmd).to_string();

        assert!(
            refused.contains("not allowed"),
            "{cmd} is refused by the ACL, got {refused}"
        );
    }
}

/// Registers a finished scan of a fake filesystem and returns its id.
fn finished_scan(window: &WebviewWindow<MockRuntime>, fs: &platter_core::testing::FakeFs) -> u64 {
    let scans = &window.state::<AppState>().scans;
    let (id, _) = scans.begin().unwrap();
    scans
        .finish(id, platter_core::testing::scan_fake(fs, false))
        .unwrap();

    id
}

#[test]
fn an_unknown_or_unfinished_scan_has_no_suggestions() {
    let (_app, window) = app();
    let (running, _) = window.state::<AppState>().scans.begin().unwrap();

    for scan in [4242, running] {
        let error =
            invoke(&window, "get_suggestions", json!({ "scan": scan })).expect_err("no tree to judge");

        assert_eq!(error["code"], "notFound", "scan {scan}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_finished_scan_gets_labelled_suggestions_and_asking_again_gives_the_same_answer() {
    let (_app, window) = app();
    let mut fs = platter_core::testing::FakeFs::new();
    fs.file("/var/cache/pacman/pkg/a.zst", 5000)
        .file("/etc/hostname", 10);
    let scan = finished_scan(&window, &fs);

    let first = invoke(&window, "get_suggestions", json!({ "scan": scan })).expect("suggestions");
    let second = invoke(&window, "get_suggestions", json!({ "scan": scan })).expect("suggestions");

    assert_eq!(first, second, "the stored result is returned again");
    assert_eq!(first["totals"], json!({ "safe": 5000, "review": 0, "keep": 0 }));
    let suggestion = &first["suggestions"][0];
    assert_eq!(suggestion["id"], "pacman-cache");
    assert_eq!(suggestion["verdict"], "safe");
    assert_eq!(suggestion["path"], "/var/cache/pacman/pkg");
    assert_eq!(
        (&suggestion["allocated"], &suggestion["items"]),
        (&json!(5000), &json!(1))
    );
    assert_eq!(first["suggestions"].as_array().unwrap().len(), 1);
}

#[test]
fn suggestions_go_away_with_the_scan() {
    let (_app, window) = app();
    let scan = finished_scan(&window, &platter_core::testing::FakeFs::new());

    assert!(invoke(&window, "get_suggestions", json!({ "scan": scan })).is_ok());
    invoke(&window, "drop_scan", json!({ "scan": scan })).unwrap();

    let error = invoke(&window, "get_suggestions", json!({ "scan": scan })).expect_err("dropped");
    assert_eq!(error["code"], "notFound");
}
