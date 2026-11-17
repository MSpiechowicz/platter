// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The advisor's model runs in this same binary started again (see `AdvisorService::production`): that
    // process serves the model and starts nothing of the app.
    if platter_desktop_lib::is_advisor_runtime(std::env::args_os()) {
        std::process::exit(platter_desktop_lib::advisor_runtime());
    }

    platter_desktop_lib::run();
}
