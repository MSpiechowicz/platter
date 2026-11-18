//! Running the tools that own a cache on macOS (package M2a).
//!
//! No tool is elevated on macOS: a command with `elevated` set is refused (`NoElevation`) before anything
//! is looked up or started. An unelevated one runs only from a root-owned program folder (`/usr/bin`,
//! `/bin`, `/usr/sbin`, `/sbin`), and only as a regular file owned by root that no one else may change, in
//! folders with the same properties. `PATH` is never used, so a program planted in a user's folder cannot
//! run. It runs with an empty environment (plus a fixed `PATH` and `LC_ALL=C`) and no input; the last lines
//! of what a failing tool wrote are kept for the message.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use platter_core::{ToolCommand, ToolFailure, ToolRunner};

/// Where the system keeps its programs. Nothing else is searched.
const PROGRAM_FOLDERS: [&str; 4] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];
const SAFE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
/// How much of a failing tool's output is kept for the message.
const MESSAGE_LINES: usize = 3;

/// Runs the tools Platter drives on macOS.
#[derive(Debug, Clone, Copy, Default)]
pub struct MacToolRunner;

impl ToolRunner for MacToolRunner {
    fn run(&self, command: &ToolCommand<'_>) -> Result<(), ToolFailure> {
        if command.elevated {
            return Err(ToolFailure::NoElevation);
        }

        let program = find_program(command.program).ok_or(ToolFailure::NotInstalled)?;
        let output = Command::new(&program)
            // The resolved file runs under its own name, as multi-call programs expect.
            .arg0(command.program)
            .args(command.args)
            .env_clear()
            .env("PATH", SAFE_PATH)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| ToolFailure::Io(error.to_string()))?;
        if output.status.success() {
            return Ok(());
        }

        Err(ToolFailure::Failed {
            code: output.status.code(),
            message: last_lines(&output.stderr, &output.stdout),
        })
    }
}

/// The trusted absolute path of `name`, or `None`.
fn find_program(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }

    PROGRAM_FOLDERS
        .iter()
        .find_map(|folder| trusted_path(&Path::new(folder).join(name)))
}

/// The resolved path of `path` when it is a regular file owned by root, executable and not writable by
/// group or others, and it and every folder above it belong to root and no one else may change them. The
/// resolved path is what runs, so no link can be swapped in between.
fn trusted_path(path: &Path) -> Option<PathBuf> {
    let resolved = std::fs::canonicalize(path).ok()?;
    let meta = std::fs::metadata(&resolved).ok()?;
    let mode = meta.permissions().mode();
    if !(meta.is_file() && meta.uid() == 0 && mode & 0o022 == 0 && mode & 0o111 != 0) {
        return None;
    }

    for folder in resolved.ancestors().skip(1) {
        let meta = std::fs::metadata(folder).ok()?;
        if meta.uid() != 0 || meta.permissions().mode() & 0o022 != 0 {
            return None;
        }
    }

    Some(resolved)
}

fn last_lines(stderr: &[u8], stdout: &[u8]) -> String {
    let text = if stderr.iter().any(|byte| !byte.is_ascii_whitespace()) {
        stderr
    } else {
        stdout
    };
    let text = String::from_utf8_lossy(text);
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();

    lines[lines.len().saturating_sub(MESSAGE_LINES)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain<'a>(program: &'a str, args: &'a [&'a str]) -> ToolCommand<'a> {
        ToolCommand {
            program,
            args,
            elevated: false,
        }
    }

    #[test]
    fn only_system_programs_are_found() {
        assert!(find_program("sh").is_some_and(|path| path.is_absolute()));
        assert!(find_program("no-such-program-platter").is_none());
        assert!(find_program("../bin/sh").is_none());
        assert!(find_program("/bin/sh").is_none());
        assert!(find_program("").is_none());
    }

    #[test]
    fn a_program_in_a_user_folder_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("tool");
        std::fs::write(&fake, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(trusted_path(&fake).is_none() || rustix::process::getuid().is_root());
    }

    #[test]
    fn an_elevated_command_is_refused_before_anything_runs() {
        let command = ToolCommand {
            program: "true",
            args: &[],
            elevated: true,
        };

        assert_eq!(MacToolRunner.run(&command), Err(ToolFailure::NoElevation));
    }

    #[test]
    fn a_tool_that_succeeds_fails_or_is_missing_is_told_apart() {
        assert_eq!(MacToolRunner.run(&plain("true", &[])), Ok(()));
        assert_eq!(
            MacToolRunner.run(&plain("no-such-program-platter", &[])),
            Err(ToolFailure::NotInstalled)
        );
        assert!(matches!(
            MacToolRunner.run(&plain("sh", &["-c", "echo first >&2; echo why it failed >&2; exit 3"])),
            Err(ToolFailure::Failed { code: Some(3), message }) if message == "first\nwhy it failed"
        ));
    }

    #[test]
    fn the_environment_is_not_passed_on() {
        let command = plain(
            "sh",
            &[
                "-c",
                "test -z \"$HOME\" && test \"$PATH\" = /usr/bin:/bin:/usr/sbin:/sbin && test \"$LC_ALL\" = C",
            ],
        );

        assert_eq!(MacToolRunner.run(&command), Ok(()));
    }
}
