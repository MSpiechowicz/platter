//! Linux side of the advisor's helper process (see `crate::runtime`).
//!
//! The parent starts the helper with `Command`, handing over three descriptors by `Stdio`: a socket as
//! standard input, the model file (or the null device) as standard output, and a pipe as standard error.
//! No `pre_exec` and no `unsafe`: standard descriptors are the only thing handed over, and the socket's
//! parent end and every other descriptor are close-on-exec (the standard library opens them so).

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};

use rustix::process::{DumpableBehavior, Signal, set_dumpable_behavior, set_parent_process_death_signal};

use crate::runtime::{ADVISOR_RUNTIME_ARG, RuntimeChild, RuntimeMode, runtime_environment};

/// This process's own binary, as the kernel names it (it works for a binary that was replaced on disk by an
/// update while it runs).
pub const CURRENT_EXE: &str = "/proc/self/exe";

const NULL_DEVICE: &str = "/dev/null";

pub fn spawn(program: &Path, model: Option<File>, mode: RuntimeMode) -> io::Result<RuntimeChild> {
    let (socket, helper_end) = UnixStream::pair()?;
    let standard_output = match model {
        Some(file) => Stdio::from(file),
        None => Stdio::from(File::open(NULL_DEVICE)?),
    };

    let mut command = Command::new(program);
    command
        .arg(ADVISOR_RUNTIME_ARG)
        .env_clear()
        .envs(runtime_environment(std::env::vars_os(), mode))
        .stdin(Stdio::from(OwnedFd::from(helper_end)))
        .stdout(standard_output)
        .stderr(Stdio::piped());

    let spawned = command.spawn();
    // The command holds the helper's end of the socket and the model: let go of them now, or the parent
    // would never see the helper's death as the end of the socket.
    drop(command);

    let mut child = spawned?;
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("the helper has no standard error"));
    };

    Ok(RuntimeChild {
        child,
        socket,
        stderr,
    })
}

pub fn take_descriptors() -> io::Result<(UnixStream, Option<File>)> {
    // Duplicates: the standard streams stay as they are, and these are ours to close.
    let socket = UnixStream::from(std::io::stdin().as_fd().try_clone_to_owned()?);
    socket
        .local_addr()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "standard input is not a socket"))?;

    let model = File::from(std::io::stdout().as_fd().try_clone_to_owned()?);
    let kind = model.metadata()?;
    let model = if kind.is_file() {
        Some(model)
    } else if is_null_device(&kind) {
        None
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "standard output is neither a model file nor the null device",
        ));
    };

    // The helper ends with its parent, and nobody can read its memory or make it dump core.
    set_parent_process_death_signal(Some(Signal::KILL)).map_err(io::Error::from)?;
    set_dumpable_behavior(DumpableBehavior::NotDumpable).map_err(io::Error::from)?;

    Ok((socket, model))
}

fn is_null_device(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_char_device()
        && std::fs::metadata(NULL_DEVICE).is_ok_and(|null| null.rdev() == meta.rdev())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A helper that is a shell script (it ignores the argument it is given).
    fn script_program(dir: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = dir.join("helper.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn the_helper_gets_the_socket_the_model_and_a_pipe_and_a_clean_environment() {
        let dir = tempfile::tempdir().unwrap();
        let model_path = dir.path().join("model");
        std::fs::write(&model_path, b"weights").unwrap();
        let program = script_program(
            dir.path(),
            "read line <&0\necho \"got $line\" >&0\ncat <&1 >&2\nenv >&2\nexit 7",
        );

        let child = spawn(&program, Some(File::open(&model_path).unwrap()), RuntimeMode::Cpu).unwrap();
        let RuntimeChild {
            mut child,
            mut socket,
            mut stderr,
        } = child;
        socket.write_all(b"hello\n").unwrap();
        let mut answer = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(&socket), &mut answer).unwrap();
        let mut diagnostics = String::new();
        stderr.read_to_string(&mut diagnostics).unwrap();
        let status = child.wait().unwrap();

        assert_eq!(answer, "got hello\n", "standard input is the socket");
        assert!(
            diagnostics.starts_with("weights"),
            "standard output is the model file"
        );
        assert!(diagnostics.contains("LANG=C") && diagnostics.contains("GGML_VK_VISIBLE_DEVICES=\n"));
        assert!(
            !diagnostics.lines().any(|line| line.starts_with("PATH=")),
            "the environment is cleared: {diagnostics}"
        );
        assert_eq!(
            crate::runtime::describe_exit(&status),
            Some(crate::runtime::Exit::Code(7))
        );
    }

    #[test]
    fn the_end_of_a_helper_is_the_end_of_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let program = script_program(dir.path(), "exit 0");

        let RuntimeChild {
            mut child,
            mut socket,
            ..
        } = spawn(&program, None, RuntimeMode::Probe).unwrap();
        let mut rest = Vec::new();
        socket.read_to_end(&mut rest).unwrap();

        assert!(rest.is_empty(), "the parent sees the end, not a hang");
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn a_program_that_is_not_there_is_an_error() {
        let error = spawn(Path::new("/nonexistent/helper"), None, RuntimeMode::Probe).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn the_null_device_is_recognised_and_a_regular_file_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();

        assert!(is_null_device(&std::fs::metadata(NULL_DEVICE).unwrap()));
        assert!(!is_null_device(&std::fs::metadata(&file).unwrap()));
        assert!(!is_null_device(&std::fs::metadata("/dev/zero").unwrap()));
    }
}
