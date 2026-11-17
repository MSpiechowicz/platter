//! Platter desktop: a thin Tauri IPC layer. All domain logic lives in `platter-core`; OS adapters in
//! `platter-platform`. Here are only the command wiring, the scan registry and input validation.

mod commands;
mod error;
mod state;
mod user_rules;

use std::sync::Arc;

use state::AppState;
use tauri::Manager;

/// Whether this process was started as the advisor's helper: its arguments are the program and exactly
/// `--advisor-runtime`.
pub fn is_advisor_runtime(args: impl IntoIterator<Item = std::ffi::OsString>) -> bool {
    let mut args = args.into_iter();

    args.next().is_some()
        && args
            .next()
            .is_some_and(|arg| arg == platter_platform::ADVISOR_RUNTIME_ARG)
        && args.next().is_none()
}

/// The advisor's helper process: serves the model runtime to the app that started it, over the socket it was
/// handed (see [`platter_ai::runtime::serve`]). Returns the exit code of the process. Nothing of the app
/// (windows, scans, the web view) is started in it. Without the runtime in this build, or where the helper
/// cannot be started, it fails (2).
pub fn advisor_runtime() -> i32 {
    #[cfg(all(feature = "advisor", unix))]
    {
        let Ok((socket, model)) = platter_platform::advisor_runtime_child() else {
            return 2;
        };

        platter_ai::runtime::serve(
            platter_ai::llama::LlamaEngine::new(),
            socket,
            model,
            platter_ai::MODEL.bytes,
        )
    }
    #[cfg(not(all(feature = "advisor", unix)))]
    {
        2
    }
}

pub fn run() {
    // Deep scans keep many directories open at once; raise the descriptor limit once, before any scan.
    // The reader copes with a low limit anyway, so a failure is not worth reporting.
    let _ = platter_platform::raise_fd_limit();

    tauri::Builder::default()
        // Used from Rust only (the `pick_folder` command); the webview is granted no dialog permission.
        // The opener plugin is not registered either: `open_in_file_manager` and `open_external` call its
        // Rust functions directly.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(|app| {
            // The confirmations (a cleanup, the model download) are native dialogs shown by the backend.
            let handle = app.handle().clone();
            #[cfg(target_os = "linux")]
            let confirm = Arc::new(commands::cleanup::GtkConfirm { app: handle });
            #[cfg(not(target_os = "linux"))]
            let confirm = Arc::new(commands::cleanup::NativeConfirm { app: handle });
            app.state::<AppState>().set_confirm(confirm);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::volumes::list_volumes,
            commands::scan::start_scan,
            commands::scan::cancel_scan,
            commands::scan::drop_scan,
            commands::query::get_node,
            commands::query::get_children,
            commands::query::get_lod,
            commands::query::get_path,
            commands::query::get_summary,
            commands::suggestions::get_suggestions,
            commands::advisor::advisor_status,
            commands::advisor::watch_advisor,
            commands::advisor::configure_advisor,
            commands::advisor::start_model_download,
            commands::advisor::cancel_model_download,
            commands::advisor::load_model,
            commands::advisor::unload_model,
            commands::advisor::label_folders,
            commands::advisor::cancel_labelling,
            commands::advisor::get_advisor_labels,
            commands::advisor::ask_advisor,
            commands::advisor::question::ask_scan,
            commands::advisor::clear_question_log,
            commands::advisor::answer_gpu_problem,
            commands::advisor::clear_gpu_log,
            commands::advisor::find_folders,
            commands::advisor::find_entries,
            commands::rules::list_user_rules,
            commands::rules::add_user_rule,
            commands::rules::remove_user_rule,
            commands::rules::reset_user_rules,
            commands::cleanup::plan_cleanup,
            commands::cleanup::plan_marked_cleanup,
            commands::cleanup::start_cleanup,
            commands::cleanup::cancel_cleanup,
            commands::cleanup::undo_cleanup,
            commands::system::open_in_file_manager,
            commands::system::pick_folder,
            commands::system::open_external,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Platter")
        .run(|app, event| {
            // Quitting while a cleanup runs: let it finish the item in progress (a folder is never
            // left half-deleted by the process ending), then stop. A model download stops too (its
            // partial file stays for the next start), a labelling job is cancelled and the model is
            // unloaded.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state = app.state::<AppState>();
                state.advisor.cancel_labelling();
                state.advisor.shutdown();
                state.cleanups.stop_for_exit(std::time::Duration::from_secs(30));
            }
        });
}

#[cfg(test)]
mod tests;
