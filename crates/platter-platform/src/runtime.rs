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
//! The Linux code is in `linux::runtime` and the macOS code in `macos::runtime`; other Unix systems get functions that fail with `Unsupported`.
//!
//! On Windows (`windows::runtime`) the protocol runs over the helper's standard input and output pipes
//! instead of a socket, the model is not handed over as a descriptor (the helper opens it by the path in the
//! `load` request, see `windows::pinned`), and the helper is put in a Job Object that kills it when the app
//! ends. Systems that are neither Unix nor Windows do not get the module at all (the advisor is unavailable
//! there).

#[cfg(unix)]
use std::ffi::OsString;
use std::fs::File;
use std::io;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
#[cfg(unix)]
use std::process::{Child, ChildStderr};

pub use crate::ADVISOR_RUNTIME_ARG;

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
#[cfg(unix)]
#[derive(Debug)]
pub struct RuntimeChild {
    pub child: Child,
    /// The parent's end of the socket that is the helper's standard input.
    pub socket: UnixStream,
    /// What the helper writes to its standard error.
    pub stderr: ChildStderr,
}

#[cfg(windows)]
pub use crate::windows::runtime::{RuntimeChild, SharedChild};

/// The helper's end of the connection to the app: the socket it was handed as standard input (Unix), or its
/// standard input and output (Windows; the reading end first).
#[cfg(unix)]
pub type HelperEnd = UnixStream;

/// The helper's end of the connection to the app: the socket it was handed as standard input (Unix), or its
/// standard input and output (Windows; the reading end first).
#[cfg(windows)]
pub type HelperEnd = (File, File);

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// It left with this exit code. On Windows this is the exit code as the system reports it, read as a
    /// signed number: a crash is an NTSTATUS such as `0xC0000005` (an access violation), which shows as
    /// negative here (see [`exit_text`]).
    Code(i32),
    /// A signal ended it (`11` is a segmentation fault). Unix only.
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

/// Starts `program` as the advisor's helper. On Unix `model` is the checked model file, given as the
/// helper's standard output (`None`, the probe, gives it the null device instead). On Windows the model goes
/// by path in the `load` request and `model` must be `None`. Fails with `Unsupported` where this cannot be
/// done safely.
pub fn spawn_advisor_runtime(
    program: &Path,
    model: Option<File>,
    mode: RuntimeMode,
) -> io::Result<RuntimeChild> {
    #[cfg(target_os = "linux")]
    {
        crate::linux::runtime::spawn(program, model, mode)
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::runtime::spawn(program, model, mode)
    }
    #[cfg(windows)]
    {
        crate::windows::runtime::spawn(program, model, mode)
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        let _ = (program, model, mode);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// In the helper: takes the connection the parent handed over (the socket on standard input on Unix, the
/// standard input and output pipes on Windows) and, on Unix, the model file on standard output (the null
/// device there means no model; on Windows there is none, the model comes by path). Makes the process end
/// with its parent (Unix; on Windows the parent's Job Object does that) and refuse to be inspected. Fails
/// when the connection is not what the parent hands over (run by hand, for one).
pub fn advisor_runtime_child() -> io::Result<(HelperEnd, Option<File>)> {
    #[cfg(target_os = "linux")]
    {
        crate::linux::runtime::take_descriptors()
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::runtime::take_descriptors()
    }
    #[cfg(windows)]
    {
        crate::windows::runtime::take_handles()
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// How `status` ended: the exit code, or the signal that killed it. `None` when it says neither.
pub fn describe_exit(status: &ExitStatus) -> Option<Exit> {
    #[cfg(unix)]
    {
        match (status.code(), status.signal()) {
            (Some(code), _) => Some(Exit::Code(code)),
            (None, Some(signal)) => Some(Exit::Signal(signal)),
            (None, None) => None,
        }
    }
    // A process on Windows always ends with an exit code (a crash is an NTSTATUS in it).
    #[cfg(not(unix))]
    {
        status.code().map(Exit::Code)
    }
}

/// What an exit means in words, where the system's own code is not self-explanatory: a Windows exit code is
/// shown in hexadecimal with the name of the failure (`0xC0000005 (access violation)`). `None` for an exit that
/// needs no words (a Unix exit code or signal, or a Windows exit code of 0 to 255).
pub fn exit_text(exit: Exit) -> Option<String> {
    #[cfg(windows)]
    {
        match exit {
            Exit::Code(code) => windows_exit_text(code),
            Exit::Signal(_) => None,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = exit;
        None
    }
}

/// The text for a Windows exit code: hexadecimal, with the name where it is a failure this program knows.
/// `None` for the small codes a program returns itself.
pub fn windows_exit_text(code: i32) -> Option<String> {
    let unsigned = code as u32;
    if unsigned < 256 {
        return None;
    }

    let name = match unsigned {
        0xC000_0005 => Some("access violation"),
        0xC000_00FD => Some("stack overflow"),
        0xC000_0374 => Some("heap corruption"),
        0xC000_0409 => Some("stack buffer overrun or fast fail (the program aborted)"),
        0xC000_013A => Some("interrupted"),
        0xC000_0142 => Some("a library could not start"),
        0xC000_0135 | 0xC000_0139 | 0xC000_007B => Some("a library is missing or does not load"),
        0xE06D_7363 => Some("an uncaught C++ exception"),
        _ => None,
    };

    Some(match name {
        Some(name) => format!("0x{unsigned:08X} ({name})"),
        None => format!("0x{unsigned:08X}"),
    })
}

/// The environment of a helper, from the variables of the parent: nothing but the ones it needs.
///
/// On Linux: `HOME` (where the driver keeps its caches) and `XDG_CACHE_HOME` when they are set, `LANG=C`, and
/// `VK_LOADER_LAYERS_DISABLE=~implicit~`, which keeps the Vulkan loader from starting the implicit layers that
/// other programs install (overlays, capture tools) in the helper. `LD_LIBRARY_PATH` (an AppImage's libraries)
/// is kept only when `APPIMAGE` is set and not empty, and then only its absolute, non-empty components: a
/// relative one would lead to whatever folder the helper runs in. A helper for the processor also gets
/// `GGML_VK_VISIBLE_DEVICES` empty, which shows the runtime no graphics card at all (it takes every one when
/// the variable is missing).
///
/// On macOS: `HOME` and `TMPDIR` when they are set (the user's own folders: where the system keeps the
/// compiled shaders of the Metal runtime) and `LANG=C`. There is no Vulkan and no loader path there, and no
/// `DYLD_*` variable ever reaches the helper (the environment is cleared). A helper for the processor is kept
/// from the graphics card by the load itself (no device is given to the model), not by a variable.
#[cfg(unix)]
pub fn runtime_environment(
    vars: impl Iterator<Item = (OsString, OsString)>,
    mode: RuntimeMode,
) -> Vec<(OsString, OsString)> {
    environment_for(cfg!(target_os = "macos"), vars, mode)
}

/// [`runtime_environment`] for the rules of one system (`apple`: macOS's), so that both are tested on Linux.
#[cfg(unix)]
fn environment_for(
    apple: bool,
    vars: impl Iterator<Item = (OsString, OsString)>,
    mode: RuntimeMode,
) -> Vec<(OsString, OsString)> {
    let kept: &[&str] = if apple {
        &["HOME", "TMPDIR"]
    } else {
        &["HOME", "XDG_CACHE_HOME"]
    };

    let vars: Vec<(OsString, OsString)> = vars.collect();
    let mut environment: Vec<(OsString, OsString)> = vars
        .iter()
        .filter(|(name, _)| kept.iter().any(|kept| name == kept))
        .cloned()
        .collect();
    environment.push(("LANG".into(), "C".into()));
    if apple {
        return environment;
    }

    let in_appimage = vars
        .iter()
        .any(|(name, value)| name == "APPIMAGE" && !value.is_empty());
    if in_appimage {
        let library_path = vars
            .iter()
            .find(|(name, _)| name == "LD_LIBRARY_PATH")
            .and_then(|(_, value)| absolute_components(value));
        if let Some(library_path) = library_path {
            environment.push(("LD_LIBRARY_PATH".into(), library_path));
        }
    }

    environment.push(("VK_LOADER_LAYERS_DISABLE".into(), "~implicit~".into()));
    if mode == RuntimeMode::Cpu {
        environment.push(("GGML_VK_VISIBLE_DEVICES".into(), OsString::new()));
    }

    environment
}

#[cfg(windows)]
pub use crate::windows::runtime::runtime_environment;

/// The components of a colon separated path list that are absolute, joined again. `None` when none is left.
#[cfg(unix)]
fn absolute_components(list: &std::ffi::OsStr) -> Option<OsString> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let kept: Vec<&[u8]> = list
        .as_bytes()
        .split(|byte| *byte == b':')
        .filter(|component| component.first() == Some(&b'/'))
        .collect();

    (!kept.is_empty()).then(|| OsString::from_vec(kept.join(&b':')))
}

/// Whether a helper whose parent had the process id `at_start` when it began (`now`: the id of its parent
/// now) has lost it. Without a death signal (macOS has none) the helper polls this. A parent that is `init`
/// (id 1, or 0) already means the app was gone before the helper looked: it was adopted at once. An id that
/// differs later means the app ended and the helper was adopted by `init`.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn parent_is_gone(at_start: u32, now: u32) -> bool {
    at_start <= 1 || now != at_start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_parent_is_gone_when_it_was_init_at_the_start_or_the_id_changes() {
        assert!(!parent_is_gone(4242, 4242), "the same parent is not gone");
        assert!(parent_is_gone(4242, 1), "adopted by init");
        assert!(parent_is_gone(4242, 7), "adopted by another reaper");
        assert!(parent_is_gone(1, 1), "the app was already gone at the start");
        assert!(parent_is_gone(0, 0));
    }

    #[test]
    fn a_windows_exit_code_is_told_in_hexadecimal_with_its_name_when_it_is_a_known_failure() {
        assert_eq!(
            windows_exit_text(0xC000_0005_u32 as i32).as_deref(),
            Some("0xC0000005 (access violation)")
        );
        assert_eq!(
            windows_exit_text(0xC000_0409_u32 as i32).as_deref(),
            Some("0xC0000409 (stack buffer overrun or fast fail (the program aborted))")
        );
        assert_eq!(
            windows_exit_text(0xDEAD_BEEF_u32 as i32).as_deref(),
            Some("0xDEADBEEF"),
            "an unknown status is still shown in hexadecimal"
        );
    }

    #[test]
    fn the_small_exit_codes_a_program_returns_itself_need_no_words() {
        for code in [0, 1, 2, 255] {
            assert_eq!(windows_exit_text(code), None, "{code}");
        }
    }

    #[cfg(unix)]
    mod environment {
        use super::super::*;

        /// The Linux rules (the macOS ones are tested through `environment_for` directly).
        fn runtime_environment(
            vars: impl Iterator<Item = (OsString, OsString)>,
            mode: RuntimeMode,
        ) -> Vec<(OsString, OsString)> {
            environment_for(false, vars, mode)
        }

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
                    ("APPIMAGE", "/opt/Platter.AppImage"),
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
            assert_eq!(
                names,
                [
                    "HOME",
                    "LANG",
                    "LD_LIBRARY_PATH",
                    "VK_LOADER_LAYERS_DISABLE",
                    "XDG_CACHE_HOME"
                ]
            );
            assert_eq!(
                lookup(&environment, "LANG"),
                Some("C".into()),
                "the locale is fixed"
            );
            assert_eq!(lookup(&environment, "HOME"), Some("/home/user".into()));
        }

        #[test]
        fn the_implicit_vulkan_layers_are_always_disabled() {
            for mode in [RuntimeMode::Probe, RuntimeMode::Cpu, RuntimeMode::Gpu] {
                let environment = runtime_environment(vars(&[("VK_LOADER_LAYERS_DISABLE", "")]), mode);

                assert_eq!(
                    lookup(&environment, "VK_LOADER_LAYERS_DISABLE"),
                    Some("~implicit~".into())
                );
                assert_eq!(
                    environment
                        .iter()
                        .filter(|(name, _)| name == "VK_LOADER_LAYERS_DISABLE")
                        .count(),
                    1
                );
            }
        }

        #[test]
        fn the_library_path_is_kept_only_in_an_appimage_and_only_its_absolute_components() {
            let path = |pairs: &[(&str, &str)]| {
                lookup(
                    &runtime_environment(vars(pairs), RuntimeMode::Gpu),
                    "LD_LIBRARY_PATH",
                )
            };

            assert_eq!(
                path(&[("LD_LIBRARY_PATH", "/app/lib")]),
                None,
                "not in an AppImage"
            );
            assert_eq!(
                path(&[("APPIMAGE", ""), ("LD_LIBRARY_PATH", "/app/lib")]),
                None,
                "an empty APPIMAGE is none"
            );
            assert_eq!(
                path(&[("APPIMAGE", "/x"), ("LD_LIBRARY_PATH", "/app/lib")]),
                Some("/app/lib".into())
            );
            assert_eq!(
                path(&[
                    ("APPIMAGE", "/x"),
                    ("LD_LIBRARY_PATH", ":lib:/app/lib::./here:/usr/lib:")
                ]),
                Some("/app/lib:/usr/lib".into()),
                "empty and relative components are dropped"
            );
            assert_eq!(
                path(&[("APPIMAGE", "/x"), ("LD_LIBRARY_PATH", ":rel:")]),
                None,
                "nothing absolute is nothing"
            );
            assert_eq!(path(&[("APPIMAGE", "/x")]), None, "not made up");
        }

        #[test]
        fn variables_that_are_not_set_are_not_made_up() {
            let environment = runtime_environment(vars(&[("HOME", "/home/user")]), RuntimeMode::Probe);

            assert_eq!(lookup(&environment, "XDG_CACHE_HOME"), None);
            assert_eq!(lookup(&environment, "LD_LIBRARY_PATH"), None);
            assert_eq!(lookup(&environment, "APPIMAGE"), None);
        }

        #[test]
        fn only_the_helper_for_the_processor_hides_the_graphics_cards() {
            let hidden = |mode| lookup(&runtime_environment(vars(&[]), mode), "GGML_VK_VISIBLE_DEVICES");

            assert_eq!(hidden(RuntimeMode::Cpu), Some(OsString::new()), "set, and empty");
            assert_eq!(hidden(RuntimeMode::Gpu), None);
            assert_eq!(hidden(RuntimeMode::Probe), None);
        }

        #[test]
        fn on_macos_the_helper_gets_home_tmpdir_and_a_fixed_locale_and_nothing_else() {
            let environment = environment_for(
                true,
                vars(&[
                    ("HOME", "/Users/user"),
                    ("TMPDIR", "/var/folders/xx/T/"),
                    ("PATH", "/usr/bin"),
                    ("XDG_CACHE_HOME", "/cache"),
                    ("APPIMAGE", "/opt/Platter.AppImage"),
                    ("LD_LIBRARY_PATH", "/app/lib"),
                    ("DYLD_INSERT_LIBRARIES", "/evil.dylib"),
                    ("DYLD_LIBRARY_PATH", "/evil"),
                    ("LANG", "pl_PL.UTF-8"),
                ]),
                RuntimeMode::Gpu,
            );

            let mut names: Vec<_> = environment
                .iter()
                .map(|(name, _)| name.to_string_lossy())
                .collect();
            names.sort();
            assert_eq!(names, ["HOME", "LANG", "TMPDIR"]);
            assert_eq!(lookup(&environment, "LANG"), Some("C".into()));
            assert_eq!(lookup(&environment, "TMPDIR"), Some("/var/folders/xx/T/".into()));
        }

        #[test]
        fn on_macos_no_mode_adds_a_graphics_variable_and_unset_variables_are_not_made_up() {
            for mode in [RuntimeMode::Probe, RuntimeMode::Cpu, RuntimeMode::Gpu] {
                let environment = environment_for(true, vars(&[("HOME", "/Users/user")]), mode);

                let mut names: Vec<_> = environment
                    .iter()
                    .map(|(name, _)| name.to_string_lossy())
                    .collect();
                names.sort();
                assert_eq!(names, ["HOME", "LANG"], "{mode:?}");
            }
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
}
