//! Platter desktop: a thin Tauri IPC layer. All domain logic lives in `platter-core`; OS adapters in
//! `platter-platform`. Here are only the command wiring, the scan registry and input validation.

mod commands;
mod error;
mod last_scan;
mod state;
mod user_rules;

use std::sync::Arc;

use state::AppState;
use tauri::Manager;

/// The window's app id and class (Linux): the base name of the desktop entry the packages install
/// (`Platter.desktop`, see `src-tauri/platter.desktop`).
pub const APP_ID: &str = "Platter";

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

/// The advisor's helper process: serves the model runtime to the app that started it, over the connection it was
/// handed (a socket on Unix, the standard input and output pipes on Windows, see
/// [`platter_ai::runtime::serve`]). Returns the exit code of the process. Nothing of the app (windows, scans,
/// the web view) is started in it. Without the runtime in this build, or where the helper cannot be started,
/// it fails (2).
pub fn advisor_runtime() -> i32 {
    #[cfg(feature = "advisor")]
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
    #[cfg(not(feature = "advisor"))]
    {
        2
    }
}

pub fn run() {
    // Deep scans keep many directories open at once; raise the descriptor limit once, before any scan.
    // The reader copes with a low limit anyway, so a failure is not worth reporting.
    let _ = platter_platform::raise_fd_limit();

    // The window's app id on Wayland and its class on X11 come from the program name. The packages name
    // their desktop entry `Platter.desktop` (after the product name), so the name is set to match it.
    // Otherwise a dock that matches by file name shows a generic icon.
    #[cfg(target_os = "linux")]
    gtk::glib::set_prgname(Some(APP_ID));

    tauri::Builder::default()
        // Used from Rust only (the `pick_folder` command); the webview is granted no dialog permission.
        // The opener plugin is not registered either: `open_in_file_manager` and `open_external` call its
        // Rust functions directly.
        .plugin(tauri_plugin_dialog::init())
        // Used from Rust only (`commands::update`); the webview is granted no updater permission.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::new())
        .manage(commands::update::PendingUpdate::default())
        .setup(|app| {
            // The confirmations (a cleanup, the model download) are native dialogs shown by the backend.
            let handle = app.handle().clone();
            #[cfg(target_os = "linux")]
            let confirm = Arc::new(commands::cleanup::GtkConfirm { app: handle });
            #[cfg(not(target_os = "linux"))]
            let confirm = Arc::new(commands::cleanup::NativeConfirm { app: handle });
            app.state::<AppState>().set_confirm(confirm);
            // The day's usage of the scanned disks: now, and every hour while the app is open.
            commands::history::spawn_sampler(Arc::clone(&app.state::<AppState>().history));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::volumes::list_volumes,
            commands::scan::start_scan,
            commands::scan::cancel_scan,
            commands::scan::drop_scan,
            commands::last_scan::open_last_scan,
            commands::last_scan::forget_last_scan,
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
            commands::history::history_overview,
            commands::history::restore_history_cleanup,
            commands::system::open_in_file_manager,
            commands::system::pick_folder,
            commands::system::pick_excluded_folder,
            commands::system::open_external,
            commands::system::disk_access,
            commands::system::open_disk_access_settings,
            commands::update::check_update,
            commands::update::install_update,
            commands::update::restart_app,
        ])
        .build(context())
        .expect("error while building Platter")
        .run(|app, event| {
            // Quitting while a cleanup runs, a download or a model: see `stop_for_exit`.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                stop_for_exit(app);
            }
        });
}

/// What quitting does first: a running cleanup finishes the item in progress (a folder is never left
/// half-deleted by the process ending), a model download stops (its partial file stays for the next
/// start), a labelling job is cancelled and the model is unloaded.
pub(crate) fn stop_for_exit<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let state = app.state::<AppState>();

    state.advisor.cancel_labelling();
    state.advisor.shutdown();
    state.cleanups.stop_for_exit(std::time::Duration::from_secs(30));
}

/// What the Windows updater does right before it starts the new setup, which then ends the process without
/// going through `ExitRequested`. The setup may fail to start, and then the app goes on, so this does
/// nothing that cannot be undone: the advisor is not shut down (its worker never serves again once it is,
/// and the helper process ends with the app anyway, see `platter_platform::windows::runtime`), and a
/// running cleanup is not meant to be here (`restart_app` refuses to run the setup while one runs): one that
/// started after that check still finishes its item before the process ends.
pub(crate) fn stop_for_update<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let state = app.state::<AppState>();

    state.advisor.cancel_labelling();
    state.cleanups.stop_for_exit(std::time::Duration::from_secs(30));
}

#[cfg(test)]
mod tests;

/// The app's context (config, assets, icons). Expanded once for the whole crate: on macOS
/// `generate_context!` also embeds `Info.plist` under a fixed symbol, which may exist only once, so the
/// tests take theirs from here too.
pub(crate) fn context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}
