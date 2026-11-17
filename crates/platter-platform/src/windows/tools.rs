//! Running the tools that own a cache on Windows (W2).
//!
//! [`WindowsToolRunner`] runs only the tools it knows by name, each at a fixed place below a known folder
//! (`System32`, `ProgramFiles`): never through `PATH`, never a program a caller names by path. The file
//! has to be a regular file (not a link) at that place. Which of them need the administrator is the
//! caller's word (`ToolCommand::elevated`), as on Linux.
//!
//! - **Elevated** (`dism`): through `runas`, which asks Windows for elevation with `ShellExecuteEx` and
//!   waits for the program to end. It gives back only the exit code, and nothing of what the program
//!   wrote. When the user declines the prompt, or the program cannot be started, `runas` reports the exit
//!   code `0xFFFFFFFF` (it drops the real error, `ERROR_CANCELLED` 1223, on the way), and that is what
//!   is `NotAuthorized` here, together with 1223 itself should a program ever end with it. The tool is
//!   trusted by where it lives (a file of `System32`), which is as far as that goes: nothing is checked
//!   of its owner or signature, and it is started with the working directory `runas` picks (`System32`).
//! - **Not elevated** (`docker`): a plain child process with a cleared environment but for what Windows
//!   programs need, no window, no input; its last output lines are kept for the message of a failure.
//!
//! An exit code of 0 is success, and so is 3010 (success, a restart is needed: `DISM` says so).

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use known_folders::{KnownFolder, get_known_folder_path};
use platter_core::{ToolCommand, ToolFailure, ToolRunner};

/// `CREATE_NO_WINDOW`: a console program started by a program with no console gets no window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `ERROR_CANCELLED`: the user declined the elevation prompt.
const ERROR_CANCELLED: i32 = 1223;
/// What `runas` reports when `ShellExecuteEx` failed (a declined prompt, a program that cannot start).
const RUNAS_FAILED: i32 = -1;
/// A restart is needed: the work is done.
const ERROR_SUCCESS_REBOOT_REQUIRED: i32 = 3010;
/// How much of a failed tool's output is kept.
const KEPT_MESSAGE_BYTES: usize = 500;

/// Runs the tools Platter drives on Windows.
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsToolRunner;

impl ToolRunner for WindowsToolRunner {
    fn run(&self, command: &ToolCommand<'_>) -> Result<(), ToolFailure> {
        let program = program_path(command.program).ok_or_else(|| {
            ToolFailure::Io(format!(
                "{} is not a tool Platter runs on Windows",
                command.program
            ))
        })?;
        if !is_regular_file(&program) {
            return Err(ToolFailure::NotInstalled);
        }

        if command.elevated {
            run_elevated(&program, command.args)
        } else {
            run_plain(&program, command.args)
        }
    }
}

/// Where the tool called `name` lives: below the system's own folders, never by `PATH`. `None` for a
/// name that is not one of the tools, or when the folder is not known.
fn program_path(name: &str) -> Option<PathBuf> {
    match name {
        "dism" | "dism.exe" => Some(get_known_folder_path(KnownFolder::System)?.join("Dism.exe")),
        "docker" | "docker.exe" => Some(
            get_known_folder_path(KnownFolder::ProgramFiles)?
                .join("Docker")
                .join("Docker")
                .join("resources")
                .join("bin")
                .join("docker.exe"),
        ),
        _ => None,
    }
}

/// Whether `path` is a regular file and not a link (nothing is followed to find out).
fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
}

fn run_elevated(program: &Path, args: &[&str]) -> Result<(), ToolFailure> {
    let status = runas::Command::new(program)
        .args(args)
        .show(false)
        .gui(true)
        .status()
        .map_err(|error| ToolFailure::Io(error.to_string()))?;

    judge(status.code(), String::new())
}

fn run_plain(program: &Path, args: &[&str]) -> Result<(), ToolFailure> {
    let output = Command::new(program)
        .args(args)
        .env_clear()
        .envs(basic_environment())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| ToolFailure::Io(error.to_string()))?;

    let mut text = output.stderr;
    if text.iter().all(u8::is_ascii_whitespace) {
        text = output.stdout;
    }

    judge(output.status.code(), last_lines(&text))
}

/// What a Windows program needs to start and to find its own files, and nothing else of the caller's
/// environment (`PATH` is the system folder alone).
fn basic_environment() -> Vec<(&'static str, std::ffi::OsString)> {
    let mut environment = Vec::new();
    for name in [
        "SystemRoot",
        "windir",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "ProgramFiles",
        "ProgramData",
    ] {
        if let Some(value) = std::env::var_os(name) {
            environment.push((name, value));
        }
    }
    if let Some(system) = get_known_folder_path(KnownFolder::System) {
        environment.push(("PATH", system.into_os_string()));
    }

    environment
}

/// What an exit code means. 0 and 3010 are success; `runas`' failure code and 1223 are a declined prompt.
fn judge(code: Option<i32>, message: String) -> Result<(), ToolFailure> {
    match code {
        Some(0) | Some(ERROR_SUCCESS_REBOOT_REQUIRED) => Ok(()),
        Some(RUNAS_FAILED) | Some(ERROR_CANCELLED) => Err(ToolFailure::NotAuthorized),
        code => Err(ToolFailure::Failed { code, message }),
    }
}

/// The last few lines of `text` (lossy), at most [`KEPT_MESSAGE_BYTES`], for a message.
fn last_lines(text: &[u8]) -> String {
    let text = String::from_utf8_lossy(text);
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(3)..].join("\n");

    let mut end = tail.len().min(KEPT_MESSAGE_BYTES);
    while !tail.is_char_boundary(end) {
        end -= 1;
    }

    tail[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_mean_what_the_module_says() {
        assert_eq!(judge(Some(0), String::new()), Ok(()));
        assert_eq!(judge(Some(3010), String::new()), Ok(()));
        assert_eq!(judge(Some(-1), String::new()), Err(ToolFailure::NotAuthorized));
        assert_eq!(judge(Some(1223), String::new()), Err(ToolFailure::NotAuthorized));
        assert_eq!(
            judge(Some(5), "denied".to_owned()),
            Err(ToolFailure::Failed {
                code: Some(5),
                message: "denied".to_owned()
            })
        );
        assert_eq!(
            judge(None, String::new()),
            Err(ToolFailure::Failed {
                code: None,
                message: String::new()
            })
        );
    }

    #[test]
    fn only_the_known_tools_have_a_place_and_none_is_found_by_path() {
        assert!(program_path("dism").is_some_and(|path| path.ends_with("Dism.exe")));
        assert!(program_path("C:\\Windows\\System32\\cmd.exe").is_none());
        assert!(program_path("..\\cmd").is_none());
        assert!(program_path("pacman").is_none());
    }

    #[test]
    fn an_unknown_program_is_refused_and_a_missing_one_is_not_installed() {
        let runner = WindowsToolRunner;

        let unknown = runner.run(&ToolCommand {
            program: "cmd",
            args: &[],
            elevated: false,
        });

        assert!(matches!(unknown, Err(ToolFailure::Io(_))));
    }

    #[test]
    fn the_kept_message_is_the_last_lines_and_is_cut_on_a_character() {
        assert_eq!(last_lines(b"a\nb\n\nc\nd\n"), "b\nc\nd");
        let long = "ż".repeat(400);

        assert!(last_lines(long.as_bytes()).len() <= KEPT_MESSAGE_BYTES);
    }
}
