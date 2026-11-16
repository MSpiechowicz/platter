//! Running the tools that own a cache (see `platter_core::ToolRunner`).
//!
//! A program is looked up only in the system's own program folders, and only accepted when it is a
//! regular file owned by root that no one else may change; `PATH` is never used, so a program planted in
//! a user's folder cannot be run, let alone as the administrator. It runs with an empty environment
//! (plus a fixed `PATH` and `LC_ALL=C`). Elevated commands run through `pkexec`, found the same way, which
//! shows the system's password prompt; `pkexec` answers 126 when the prompt was dismissed and 127 when
//! authorization failed.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use platter_core::{ToolCommand, ToolFailure, ToolRunner};

/// Where the system keeps its programs. Nothing else is searched.
const PROGRAM_FOLDERS: [&str; 4] = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"];
const SAFE_PATH: &str = "/usr/bin:/usr/sbin:/bin:/sbin";
/// How much of a failing tool's output is kept for the message.
const MESSAGE_LINES: usize = 3;

/// [`ToolRunner`] for Linux.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxToolRunner;

impl ToolRunner for LinuxToolRunner {
    fn run(&self, command: &ToolCommand<'_>) -> Result<(), ToolFailure> {
        let program = find_program(command.program).ok_or(ToolFailure::NotInstalled)?;

        let mut process = if command.elevated {
            let pkexec = find_program("pkexec").ok_or(ToolFailure::NoElevation)?;
            let mut process = Command::new(pkexec);
            process.arg0("pkexec").arg(&program);
            process
        } else {
            let mut process = Command::new(&program);
            // The resolved file runs under its own name, as multi-call programs (busybox) expect.
            process.arg0(command.program);
            process
        };
        process
            .args(command.args)
            .env_clear()
            .env("PATH", SAFE_PATH)
            .env("LC_ALL", "C")
            .stdin(Stdio::null());

        let output = process
            .output()
            .map_err(|error| ToolFailure::Io(error.to_string()))?;
        if output.status.success() {
            return Ok(());
        }

        let code = output.status.code();
        if command.elevated && matches!(code, Some(126 | 127)) && refused_by_pkexec(&output.stderr) {
            return Err(ToolFailure::NotAuthorized);
        }
        Err(ToolFailure::Failed {
            code,
            message: last_lines(&output.stderr, &output.stdout),
        })
    }
}

/// Whether `pkexec` itself turned the request down (dismissed or not authorized), as opposed to the tool
/// it ran ending with 126 or 127. `pkexec` says so on its own stderr line.
fn refused_by_pkexec(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr)
        .lines()
        .any(|line| line.starts_with("Error executing command as another user"))
}

/// The trusted absolute path of `name`, or `None`.
pub(crate) fn find_program(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }

    PROGRAM_FOLDERS
        .iter()
        .find_map(|folder| trusted_path(&Path::new(folder).join(name)))
}

/// A regular file (links resolved: `/bin` is often a link to `/usr/bin`) owned by root, executable, and
/// not writable by group or others, in a folder with the same properties.
/// The resolved path of `path` when it and every folder above it belong to root and no one else may
/// change them; that resolved path is what runs, so no link can be swapped in between.
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
        let fake = dir.path().join("paccache");
        std::fs::write(&fake, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(trusted_path(&fake).is_none() || rustix::process::getuid().is_root());
    }

    #[test]
    fn a_tool_that_succeeds_fails_or_is_missing_is_told_apart() {
        let runner = LinuxToolRunner;
        let plain = |program, args| ToolCommand {
            program,
            args,
            elevated: false,
        };

        assert_eq!(runner.run(&plain("true", &[])), Ok(()));
        assert_eq!(
            runner.run(&plain("no-such-program-platter", &[])),
            Err(ToolFailure::NotInstalled)
        );
        assert!(matches!(
            runner.run(&plain("sh", &["-c", "echo first >&2; echo why it failed >&2; exit 3"])),
            Err(ToolFailure::Failed { code: Some(3), message }) if message == "first\nwhy it failed"
        ));
    }

    #[test]
    fn only_pkexecs_own_message_means_the_password_was_refused() {
        assert!(refused_by_pkexec(
            b"Error executing command as another user: Request dismissed\n"
        ));
        assert!(refused_by_pkexec(
            b"Error executing command as another user: Not authorized\n\nThis incident has been reported.\n"
        ));
        assert!(!refused_by_pkexec(b"paccache: command not found\n"));
        assert!(!refused_by_pkexec(b""));
    }

    #[test]
    fn the_environment_is_not_passed_on() {
        let runner = LinuxToolRunner;
        let command = ToolCommand {
            program: "sh",
            args: &[
                "-c",
                "test -z \"$HOME\" && test \"$PATH\" = /usr/bin:/usr/sbin:/bin:/sbin",
            ],
            elevated: false,
        };

        assert_eq!(runner.run(&command), Ok(()));
    }
}
