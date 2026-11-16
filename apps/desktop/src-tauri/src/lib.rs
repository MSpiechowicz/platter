//! Platter desktop: a thin Tauri IPC layer. All domain logic lives in `platter-core`; OS adapters in
//! `platter-platform`. Here are only the command wiring, the scan registry and input validation.

mod commands;
mod error;
mod state;

use state::AppState;

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
            commands::system::open_in_file_manager,
            commands::system::pick_folder,
            commands::system::open_external,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Platter");
}

#[cfg(test)]
mod tests;
