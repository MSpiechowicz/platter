//! The advisor's helper process on Windows (see `crate::runtime` for the shared part).
//!
//! The helper is this binary started again with `--advisor-runtime`. What differs from Linux:
//!
//! - The protocol runs over the helper's standard input and output, which are anonymous pipes the app made
//!   (a socket pair is not available to safe code here). Standard error is a pipe the app keeps the last lines
//!   of, as on Linux.
//! - The model is not a handle the helper inherits (the standard library cannot hand over a handle that is not
//!   a standard stream). The app pins the file (`windows::pinned`) and names its path in the `load` request.
//! - The helper is put in a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (`win32job`). The app holds
//!   the only handle to it, so when the app ends, however it ends (a crash, `taskkill /f`, a power-off of the
//!   session), Windows closes the handle and ends the helper. The same happens when the app lets go of the
//!   helper's [`SharedChild`]. The helper is assigned to the job right after it starts: a moment in which it is
//!   our own binary starting up and nothing else. Without a Job Object (the system refused one) the helper is not
//!   started: the guarantee that it dies with the app is not optional.
//! - A kill is [`Child::kill`] (`TerminateProcess`). The child is behind a lock so that the watchdog thread of a
//!   write that stalls can kill it while the owner waits for it ([`SharedChild`]; waiting looks at the process in
//!   short steps and never holds the lock while it sleeps).
//! - A helper that ends shows its exit code. A crash is an NTSTATUS (`0xC0000005` is an access violation).
//!
//! What the helper gets in its environment, from the variables of the parent (names compared without regard to
//! case, as Windows does): `SystemRoot` and `windir` only. The system's libraries and the C runtime look for the
//! Windows folder through them, and a process without `SystemRoot` fails in odd places (`Winsock`,
//! `CryptoAPI`, some loaders). Nothing else is passed: no `PATH` (nothing is looked up by it, and a folder on it
//! must not lead into the helper), no `USERPROFILE`, `APPDATA`, `LOCALAPPDATA` or `TEMP` (the helper reads and
//! writes no file of the user's: the model comes by path, there is no cache and no log), and nothing of the
//! app's own settings. It runs in the Windows folder, so that nothing relative to where the app was started
//! leads into it.
//!
//! The helper is the app's own binary and runs as the same user: this isolates crashes, not privileges.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::io::{AsHandle, AsRawHandle};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use win32job::{ExtendedLimitInfo, Job};

use crate::runtime::{ADVISOR_RUNTIME_ARG, RuntimeMode};

/// `CREATE_NO_WINDOW`: a helper started by a program that has a console (a debug build) gets no window of its
/// own.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How often [`SharedChild::wait`] looks at the process.
const WAIT_STEP: Duration = Duration::from_millis(5);

/// A helper process that has been started.
#[derive(Debug)]
pub struct RuntimeChild {
    pub child: SharedChild,
    /// The parent's end of the helper's standard input: requests go in here.
    pub stdin: ChildStdin,
    /// The helper's standard output: replies come out here.
    pub stdout: ChildStdout,
    /// What the helper writes to its standard error.
    pub stderr: ChildStderr,
}

/// The helper's process and its Job Object. It can be copied: a copy kills the process too, which is what
/// ends a write that does not go through. The process is killed when the last copy is dropped (the job is
/// closed).
#[derive(Debug, Clone)]
pub struct SharedChild {
    inner: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    child: Mutex<Child>,
    id: u32,
    /// Closed last: the process is killed when it is (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
    _job: Job,
}

impl SharedChild {
    fn new(child: Child, job: Job) -> Self {
        Self {
            inner: Arc::new(Shared {
                id: child.id(),
                child: Mutex::new(child),
                _job: job,
            }),
        }
    }

    /// The process's number.
    pub fn id(&self) -> u32 {
        self.inner.id
    }

    /// Ends the process, without waiting. It does nothing when the process is gone.
    pub fn kill(&self) {
        let _ = self.child().kill();
    }

    /// The exit status when the process has ended.
    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        self.child().try_wait()
    }

    /// Waits for the process to end. The lock is taken only for each look (a blocking wait would hold it, and
    /// a kill from another thread, the watchdog's, would then wait for the end it is meant to cause).
    pub fn wait(&self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(WAIT_STEP);
        }
    }

    fn child(&self) -> std::sync::MutexGuard<'_, Child> {
        self.inner
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A command for the helper `program`, with its environment and its folder (no argument yet).
fn helper_command(program: &Path, mode: RuntimeMode) -> Command {
    let vars: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let environment = runtime_environment(vars.iter().cloned(), mode);

    let mut command = Command::new(program);
    command.env_clear().envs(environment);
    if let Some((_, windows)) = vars.iter().find(|(name, _)| is_named(name, "SystemRoot"))
        && Path::new(windows).is_absolute()
    {
        command.current_dir(windows);
    }

    command
}

pub fn spawn(program: &Path, model: Option<File>, mode: RuntimeMode) -> io::Result<RuntimeChild> {
    if model.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "on Windows the model goes by path in the load request",
        ));
    }

    let mut command = helper_command(program, mode);
    command.arg(ADVISOR_RUNTIME_ARG);

    start(command)
}

/// Starts `command` with its three standard streams piped and no window, and puts the process in a Job Object
/// that kills it when the last handle to the job is closed.
fn start(mut command: Command) -> io::Result<RuntimeChild> {
    // Before the process exists, so a refusal leaves nothing running.
    let mut limits = ExtendedLimitInfo::new();
    limits.limit_kill_on_job_close();
    let job = Job::create_with_limit_info(&limits).map_err(io::Error::from)?;

    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn()?;

    if let Err(error) = job.assign_process(child.as_raw_handle() as isize) {
        let error = io::Error::from(error);
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::new(
            error.kind(),
            format!("the helper could not be put in a Job Object: {error}"),
        ));
    }

    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("the helper has no standard streams"));
    };

    Ok(RuntimeChild {
        child: SharedChild::new(child, job),
        stdin,
        stdout,
        stderr,
    })
}

/// In the helper: the standard input and output pipes the app made (reading end first). Refuses to run when they
/// are not pipes (the helper started by hand, at a console or with files for streams): nothing is served there.
pub fn take_handles() -> io::Result<((File, File), Option<File>)> {
    let input = std::io::stdin();
    let output = std::io::stdout();

    for (name, is_pipe) in [
        ("input", winapi_util::file::typ(&input)?.is_pipe()),
        ("output", winapi_util::file::typ(&output)?.is_pipe()),
    ] {
        if !is_pipe {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("standard {name} is not a pipe"),
            ));
        }
    }

    // Duplicates: the standard streams stay as they are (the library's buffers and locks are not in the way of
    // the protocol), and these are ours to close.
    let reader = File::from(input.as_handle().try_clone_to_owned()?);
    let writer = File::from(output.as_handle().try_clone_to_owned()?);

    Ok(((reader, writer), None))
}

/// The environment of a helper from the variables of the parent: `SystemRoot` and `windir`, see the module's
/// text for why nothing else. `mode` does not matter here (the advisor runs on the processor only).
pub fn runtime_environment(
    vars: impl Iterator<Item = (OsString, OsString)>,
    mode: RuntimeMode,
) -> Vec<(OsString, OsString)> {
    let _ = mode;
    const KEPT: [&str; 2] = ["SystemRoot", "windir"];

    vars.filter(|(name, _)| KEPT.iter().any(|kept| is_named(name, kept)))
        .collect()
}

/// Whether `name` is the variable `wanted`: Windows compares the names of variables without regard to case.
fn is_named(name: &std::ffi::OsStr, wanted: &str) -> bool {
    name.to_str()
        .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    fn system32(program: &str) -> std::path::PathBuf {
        let root = std::env::var_os("SystemRoot").expect("Windows sets SystemRoot");

        Path::new(&root).join("System32").join(program)
    }

    #[test]
    fn the_helper_gets_the_windows_folder_and_nothing_else_of_the_parents_environment() {
        let vars = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                .collect::<Vec<_>>()
                .into_iter()
        };

        let environment = runtime_environment(
            vars(&[
                ("SYSTEMROOT", "C:\\Windows"),
                ("windir", "C:\\Windows"),
                ("Path", "C:\\evil"),
                ("USERPROFILE", "C:\\Users\\u"),
                ("LOCALAPPDATA", "C:\\Users\\u\\AppData\\Local"),
                ("TEMP", "C:\\Temp"),
            ]),
            RuntimeMode::Cpu,
        );

        let mut names: Vec<String> = environment
            .iter()
            .map(|(name, _)| name.to_string_lossy().to_ascii_lowercase())
            .collect();
        names.sort();
        assert_eq!(names, ["systemroot", "windir"]);
    }

    /// Starts a copy of this test program that runs only `test`, in the helper's environment, with `marker`
    /// set so that the test knows it is the child. (The harness writes to the same standard output, so the
    /// parent looks for the lines the child is asked for.)
    fn run_test_as_helper(test: &str, marker: &str) -> RuntimeChild {
        let program = std::env::current_exe().unwrap();
        let mut command = helper_command(&program, RuntimeMode::Cpu);
        command
            .env(marker, "1")
            .args(["--exact", test])
            .args(["--nocapture", "--test-threads=1"]);

        start(command).unwrap()
    }

    #[test]
    fn the_helper_runs_with_a_clean_environment() {
        const CHILD: &str = "PLATTER_ENVIRONMENT_CHILD";
        const TEST: &str = "windows::runtime::tests::the_helper_runs_with_a_clean_environment";
        if std::env::var_os(CHILD).is_some() {
            for (name, _) in std::env::vars_os() {
                println!("ENV:{}", name.to_string_lossy().to_ascii_lowercase());
            }
            return;
        }

        let RuntimeChild {
            child,
            stdin,
            mut stdout,
            ..
        } = run_test_as_helper(TEST, CHILD);
        drop(stdin);
        let mut text = String::new();
        stdout.read_to_string(&mut text).unwrap();
        let status = child.wait().unwrap();

        let inside: Vec<&str> = text
            .lines()
            .filter_map(|line| line.find("ENV:").map(|at| &line[at + "ENV:".len()..]))
            .collect();
        let outside: Vec<String> = std::env::vars_os()
            .map(|(name, _)| name.to_string_lossy().to_ascii_lowercase())
            .collect();
        assert!(status.success(), "{status:?}: {text}");
        assert!(inside.contains(&"systemroot"), "{text}");
        assert!(outside.iter().any(|name| name == "path"), "Windows sets Path");
        for name in &outside {
            let allowed = ["systemroot", "windir"].contains(&name.as_str());
            assert!(
                allowed || !inside.contains(&name.as_str()),
                "{name} leaked: {text}"
            );
        }
    }

    #[test]
    fn standard_error_is_a_pipe_of_its_own_and_an_exit_is_its_code() {
        let mut command = helper_command(&system32("cmd.exe"), RuntimeMode::Cpu);
        command.args(["/d", "/c", "echo last words 1>&2 & exit 7"]);

        let RuntimeChild {
            child,
            stdin,
            mut stderr,
            ..
        } = start(command).unwrap();
        drop(stdin);
        let mut text = String::new();
        stderr.read_to_string(&mut text).unwrap();
        let status = child.wait().unwrap();

        assert!(text.contains("last words"), "{text:?}");
        assert_eq!(
            crate::runtime::describe_exit(&status),
            Some(crate::runtime::Exit::Code(7))
        );
    }

    #[test]
    fn a_helper_dies_when_the_last_handle_to_its_job_is_closed() {
        // A process that would run for a minute.
        let mut command = helper_command(&system32("ping.exe"), RuntimeMode::Cpu);
        command.args(["-n", "60", "127.0.0.1"]);
        let RuntimeChild {
            child,
            stdin,
            mut stdout,
            ..
        } = start(command).unwrap();
        assert!(child.try_wait().unwrap().is_none(), "it runs");

        // The end of its standard output is the end of the process.
        let (ended, gone) = mpsc::channel();
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = stdout.read_to_end(&mut sink);
            let _ = ended.send(());
        });

        drop(child);

        assert!(
            gone.recv_timeout(Duration::from_secs(20)).is_ok(),
            "the process was not ended when the job was closed"
        );
        drop(stdin);
    }

    #[test]
    fn a_copy_of_the_child_can_kill_it() {
        let mut command = helper_command(&system32("ping.exe"), RuntimeMode::Cpu);
        command.args(["-n", "60", "127.0.0.1"]);
        let RuntimeChild { child, .. } = start(command).unwrap();
        assert!(child.try_wait().unwrap().is_none(), "it runs");

        let killer = child.clone();
        std::thread::spawn(move || killer.kill()).join().unwrap();

        let began = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            assert!(began.elapsed() < Duration::from_secs(10), "it did not end");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The helper's side, run in a copy of this test program: it takes the pipes it was given as the
    /// protocol's two ends and answers what it reads, line by line. What goes in comes out unchanged, text
    /// that is not ASCII included (no code page is involved, it is bytes).
    #[test]
    fn what_goes_into_standard_input_comes_out_of_standard_output() {
        const CHILD: &str = "PLATTER_TAKE_HANDLES_CHILD";
        const TEST: &str =
            "windows::runtime::tests::what_goes_into_standard_input_comes_out_of_standard_output";
        if std::env::var_os(CHILD).is_some() {
            let ((mut reader, mut writer), model) = take_handles().expect("the pipes of the parent");
            assert!(model.is_none(), "on Windows the model comes by path");

            let mut request = String::new();
            reader.read_to_string(&mut request).unwrap();
            for line in request.lines() {
                writeln!(writer, "ECHO:{line}").unwrap();
            }
            return;
        }

        let RuntimeChild {
            child,
            mut stdin,
            mut stdout,
            ..
        } = run_test_as_helper(TEST, CHILD);
        stdin
            .write_all("{\"type\":\"gpus\"}\n{\"text\":\"żółć\"}\n".as_bytes())
            .unwrap();
        drop(stdin);
        let mut text = String::new();
        stdout.read_to_string(&mut text).unwrap();
        let status = child.wait().unwrap();

        let echoed: Vec<&str> = text
            .lines()
            .filter_map(|line| line.find("ECHO:").map(|at| &line[at + "ECHO:".len()..]))
            .collect();
        assert_eq!(echoed, ["{\"type\":\"gpus\"}", "{\"text\":\"żółć\"}"], "{text}");
        assert!(status.success(), "{status:?}: {text}");
    }

    #[test]
    fn a_wait_does_not_stop_a_kill_from_another_thread() {
        let mut command = helper_command(&system32("ping.exe"), RuntimeMode::Cpu);
        command.args(["-n", "60", "127.0.0.1"]);
        let RuntimeChild { child, .. } = start(command).unwrap();

        let killer = child.clone();
        let (killed, ended) = mpsc::channel();
        let waiter = std::thread::spawn(move || child.wait());
        std::thread::sleep(Duration::from_millis(200));
        std::thread::spawn(move || {
            killer.kill();
            let _ = killed.send(());
        });

        assert!(
            ended.recv_timeout(Duration::from_secs(10)).is_ok(),
            "the kill waited for the wait"
        );
        assert!(!waiter.join().unwrap().unwrap().success());
    }

    #[test]
    fn a_program_that_is_not_there_is_an_error_and_a_model_handle_is_refused() {
        let missing = Path::new("C:\\nonexistent\\helper.exe");
        let some_file = std::fs::File::open(system32("cmd.exe")).unwrap();

        assert_eq!(
            spawn(missing, None, RuntimeMode::Cpu).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            spawn(missing, Some(some_file), RuntimeMode::Cpu)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
