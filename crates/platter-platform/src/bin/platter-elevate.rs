//! The root helper that `pkexec` runs for the Linux packages: see `platter_platform::linux::elevate`.

use std::process::ExitCode;

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    platter_platform::linux::elevate::main()
}

/// The helper exists on Linux only. This keeps the workspace building on the other systems.
#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!("platter-elevate: runs on Linux only");
    ExitCode::FAILURE
}
