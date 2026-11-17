//! The advisor's helper process: the model always runs in the same binary started again with
//! [`ADVISOR_RUNTIME_ARG`], so that a graphics driver that crashes or hangs takes down the helper and not
//! the app.
//!
//! The parent and the helper talk over a socket that is the helper's standard input. The model file is the
//! helper's standard output (a descriptor of a file the parent has already checked), and its standard error
//! is a pipe the parent reads. Nothing else is handed over: the environment is cleared (see
//! [`runtime_environment`]) and every other descriptor of the parent is closed on exec.
//!
//! This module is the one place that knows how the helper is started and how it takes its descriptors.
//! The Linux code is in `linux::runtime`; other Unix systems get functions that fail with `Unsupported`,
//! and systems without Unix sockets do not get the module at all (the advisor is unavailable there).

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ExitStatus};

/// The only argument of the helper process: the binary started with exactly this runs the model runtime
/// and nothing else.
pub const ADVISOR_RUNTIME_ARG: &str = "--advisor-runtime";

/// What a helper is started for. It decides the environment, nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// Asks the runtime which graphics cards there are; no model.
    Probe,
    /// Runs the model on the processor: no graphics card is shown to it.
    Cpu,
    /// Runs the model on a graphics card.
    Gpu,
}

/// A helper process that has been started.
#[derive(Debug)]
pub struct RuntimeChild {
    pub child: Child,
    /// The parent's end of the socket that is the helper's standard input.
    pub socket: UnixStream,
    /// What the helper writes to its standard error.
    pub stderr: ChildStderr,
}

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// It left with this exit code.
    Code(i32),
    /// A signal ended it (`11` is a segmentation fault).
    Signal(i32),
}

/// The program that starts the helper: this very binary.
pub fn runtime_program() -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        Ok(PathBuf::from(crate::linux::runtime::CURRENT_EXE))
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::env::current_exe()
    }
}

/// Starts `program` as the advisor's helper. `model` is the checked model file, given as the helper's
/// standard output; `None` (the probe) gives it the null device instead. Fails with `Unsupported` where
/// this cannot be done safely.
pub fn spawn_advisor_runtime(
    program: &Path,
    model: Option<File>,
    mode: RuntimeMode,
) -> io::Result<RuntimeChild> {
    #[cfg(target_os = "linux")]
    {
        crate::linux::runtime::spawn(program, model, mode)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (program, model, mode);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// In the helper: takes the descriptors the parent handed over (the socket on standard input and the model
/// file on standard output; the null device there means no model), and makes the process end with its
/// parent and refuse to be inspected. Fails when standard input is not a socket, or standard output is
/// neither a regular file nor the null device.
pub fn advisor_runtime_child() -> io::Result<(UnixStream, Option<File>)> {
    #[cfg(target_os = "linux")]
    {
        crate::linux::runtime::take_descriptors()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// How `status` ended: the exit code, or the signal that killed it. `None` when it says neither.
pub fn describe_exit(status: &ExitStatus) -> Option<Exit> {
    match (status.code(), status.signal()) {
        (Some(code), _) => Some(Exit::Code(code)),
        (None, Some(signal)) => Some(Exit::Signal(signal)),
        (None, None) => None,
    }
}

/// The environment of a helper, from the variables of the parent: nothing but the ones it needs. `HOME`
/// (where the driver keeps its caches), `XDG_CACHE_HOME` and `LD_LIBRARY_PATH` (an AppImage's libraries)
/// when they are set, and `LANG=C`. A helper for the processor also gets `GGML_VK_VISIBLE_DEVICES` empty,
/// which shows the runtime no graphics card at all (it takes every one when the variable is missing).
pub fn runtime_environment(
    vars: impl Iterator<Item = (OsString, OsString)>,
    mode: RuntimeMode,
) -> Vec<(OsString, OsString)> {
    const KEPT: [&str; 3] = ["HOME", "XDG_CACHE_HOME", "LD_LIBRARY_PATH"];

    let mut environment: Vec<(OsString, OsString)> = vars
        .filter(|(name, _)| KEPT.iter().any(|kept| name == kept))
        .collect();
    environment.push(("LANG".into(), "C".into()));
    if mode == RuntimeMode::Cpu {
        environment.push(("GGML_VK_VISIBLE_DEVICES".into(), OsString::new()));
    }

    environment
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> impl Iterator<Item = (OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn lookup(environment: &[(OsString, OsString)], name: &str) -> Option<OsString> {
        environment
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }

    #[test]
    fn the_helper_gets_only_the_variables_it_needs() {
        let environment = runtime_environment(
            vars(&[
                ("HOME", "/home/user"),
                ("PATH", "/usr/bin"),
                ("XDG_CACHE_HOME", "/cache"),
                ("LD_LIBRARY_PATH", "/app/lib"),
                ("LD_PRELOAD", "/evil.so"),
                ("VK_INSTANCE_LAYERS", "x"),
                ("LANG", "pl_PL.UTF-8"),
                ("GGML_VK_VISIBLE_DEVICES", "0"),
            ]),
            RuntimeMode::Gpu,
        );

        let mut names: Vec<_> = environment
            .iter()
            .map(|(name, _)| name.to_string_lossy())
            .collect();
        names.sort();
        assert_eq!(names, ["HOME", "LANG", "LD_LIBRARY_PATH", "XDG_CACHE_HOME"]);
        assert_eq!(
            lookup(&environment, "LANG"),
            Some("C".into()),
            "the locale is fixed"
        );
        assert_eq!(lookup(&environment, "HOME"), Some("/home/user".into()));
    }

    #[test]
    fn variables_that_are_not_set_are_not_made_up() {
        let environment = runtime_environment(vars(&[("HOME", "/home/user")]), RuntimeMode::Probe);

        assert_eq!(lookup(&environment, "XDG_CACHE_HOME"), None);
        assert_eq!(lookup(&environment, "LD_LIBRARY_PATH"), None);
    }

    #[test]
    fn only_the_helper_for_the_processor_hides_the_graphics_cards() {
        let hidden = |mode| lookup(&runtime_environment(vars(&[]), mode), "GGML_VK_VISIBLE_DEVICES");

        assert_eq!(hidden(RuntimeMode::Cpu), Some(OsString::new()), "set, and empty");
        assert_eq!(hidden(RuntimeMode::Gpu), None);
        assert_eq!(hidden(RuntimeMode::Probe), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_exit_is_a_code_or_a_signal() {
        let run = |script: &str| {
            std::process::Command::new("/bin/sh")
                .args(["-c", script])
                .status()
                .unwrap()
        };

        assert_eq!(describe_exit(&run("exit 3")), Some(Exit::Code(3)));
        assert_eq!(describe_exit(&run("kill -SEGV $$")), Some(Exit::Signal(11)));
    }
}
