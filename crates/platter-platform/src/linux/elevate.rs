//! `platter-elevate`: the one program Platter asks `pkexec` to run as root.
//!
//! The polkit policy names this program, so one password prompt covers every cleanup command of a run.
//! That is only safe because the helper does no more than the closed list of [`ElevatedTool`]: it takes
//! from one to seven arguments, each exactly the id of a command on the list and none twice, looks each
//! command's program up the way the tool runner does (in the system's own program folders, root-owned
//! only), and runs the commands in order. It reads no file, no environment variable and no standard input.
//!
//! It runs every command as a child (with an empty environment but a fixed `PATH` and `LC_ALL=C`, and
//! the umask 022 and the folder `/`), and sends what a command writes to standard output to the helper's
//! standard error, next to what it writes there itself. After each command the helper writes one line to
//! its standard output, which carries nothing else: [`marker_line`], `platter-elevate: done <id> <code>`,
//! where the code is
//! the exit code of the command, or `signal` (it was killed), `untrusted` (the program is not installed in
//! a system folder, or is not one a user could not have planted) or `spawn` (it could not be started).
//! Whatever the outcome it goes on with the next command, and it exits 0 once it has run its list.
//!
//! Its own exit codes (never 126 or 127, which `pkexec` uses for a request it refused, and never a line
//! starting "Error executing command as another user", which the runner takes for such a refusal):
//!
//! - 64: no argument, more than seven, an argument that is not an id (or starts with `-`), or an id twice.
//!   The runner then asks `pkexec` for each command itself, as it did before there was a helper.
//! - 71: the folder `/` could not be entered.
//! - 77: the helper does not run as root.
//!
//! Every message on standard error starts with [`MESSAGE_PREFIX`], so the runner can tell the helper's
//! own 64 from the same code coming from a program.

use std::ffi::OsString;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode, Stdio};

use platter_core::ElevatedTool;

use super::tools::{SAFE_PATH, find_program};

/// The arguments are missing, are more than seven, or are not ids of the list (or one is given twice).
pub(crate) const EXIT_USAGE: u8 = 64;
/// The folder `/` could not be entered.
pub(crate) const EXIT_OS: u8 = 71;
/// The helper does not run as root.
pub(crate) const EXIT_NOT_ROOT: u8 = 77;

/// The start of every line the helper writes to its standard error.
pub(crate) const MESSAGE_PREFIX: &str = "platter-elevate: ";
/// The start of the line that tells how a command ended, on the standard output.
pub(crate) const DONE_PREFIX: &str = "platter-elevate: done ";

/// How a command of the list ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Done {
    /// It ran and ended with this exit code.
    Code(i32),
    /// It was killed by a signal.
    Signal,
    /// Its program is not installed in a system folder, or is not trusted.
    Untrusted,
    /// Its program could not be started.
    Spawn,
}

impl Done {
    fn word(self) -> String {
        match self {
            Self::Code(code) => code.to_string(),
            Self::Signal => "signal".to_owned(),
            Self::Untrusted => "untrusted".to_owned(),
            Self::Spawn => "spawn".to_owned(),
        }
    }

    pub(crate) fn from_word(word: &str) -> Option<Self> {
        match word {
            "signal" => Some(Self::Signal),
            "untrusted" => Some(Self::Untrusted),
            "spawn" => Some(Self::Spawn),
            _ => word.parse().ok().map(Self::Code),
        }
    }
}

/// The line that tells how `tool` ended, without its line break.
pub(crate) fn marker_line(tool: ElevatedTool, done: Done) -> String {
    format!("{DONE_PREFIX}{} {}", tool.id(), done.word())
}

/// The commands the arguments name, in order, or a message when they are not a list of different ids.
fn parse_request(args: impl IntoIterator<Item = OsString>) -> Result<Vec<ElevatedTool>, String> {
    let mut tools: Vec<ElevatedTool> = Vec::new();

    for arg in args {
        let Some(id) = arg.to_str() else {
            return Err("an id is not text".to_owned());
        };
        if id.starts_with('-') {
            return Err("an option is not an id of a command".to_owned());
        }
        let Some(tool) = ElevatedTool::from_id(id) else {
            return Err("not the id of a command".to_owned());
        };
        if tools.contains(&tool) {
            return Err("an id is given twice".to_owned());
        }
        if tools.len() == ElevatedTool::ALL.len() {
            return Err("more ids than there are commands".to_owned());
        }

        tools.push(tool);
    }

    if tools.is_empty() {
        return Err("expected the ids of one to seven commands".to_owned());
    }

    Ok(tools)
}

fn refuse(code: u8, message: &str) -> ExitCode {
    eprintln!("{MESSAGE_PREFIX}{message}");
    ExitCode::from(code)
}

/// Runs the commands the arguments name, one after the other, and tells how each ended.
pub fn main() -> ExitCode {
    let tools = match parse_request(std::env::args_os().skip(1)) {
        Ok(tools) => tools,
        Err(message) => return refuse(EXIT_USAGE, &message),
    };

    if !rustix::process::geteuid().is_root() {
        return refuse(EXIT_NOT_ROOT, "not running as root");
    }

    rustix::process::umask(rustix::fs::Mode::from_bits_truncate(0o022));
    if let Err(error) = std::env::set_current_dir("/") {
        return refuse(EXIT_OS, &format!("could not enter /: {error}"));
    }

    for tool in tools {
        let done = run_one(tool);

        // A closed output is not a reason to leave the rest of the list undone.
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{}", marker_line(tool, done));
        let _ = out.flush();
    }

    ExitCode::SUCCESS
}

/// Runs one command to its end. Its standard error is the helper's, and so is its standard output, so
/// that the helper's own standard output holds the marker lines only.
fn run_one(tool: ElevatedTool) -> Done {
    let Some(program) = find_program(tool.program()) else {
        return Done::Untrusted;
    };

    // The resolved file runs under its own name, as multi-call programs (busybox) expect.
    let mut process = Command::new(program);
    process
        .arg0(tool.program())
        .args(tool.args())
        .env_clear()
        .env("PATH", SAFE_PATH)
        .env("LC_ALL", "C")
        .stdin(Stdio::null());

    run_child(process, stderr_as_stdout())
}

/// A second handle on the helper's standard error, for a child's standard output. Without one (the
/// helper's standard error is closed) the output is dropped: it must not reach the marker channel.
fn stderr_as_stdout() -> Stdio {
    std::io::stderr()
        .as_fd()
        .try_clone_to_owned()
        .map_or_else(|_| Stdio::null(), Stdio::from)
}

/// Runs `process` with `stdout` as its standard output, and tells how it ended.
fn run_child(mut process: Command, stdout: Stdio) -> Done {
    let status = process.stdout(stdout).status();

    match status {
        Ok(status) => status.code().map_or(Done::Signal, Done::Code),
        Err(_) => Done::Spawn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE: &str = "PLATTER_ELEVATE_STDOUT_PROBE";
    const TOKEN: &str = "child-output-token";

    fn parse(args: &[&str]) -> Result<Vec<ElevatedTool>, String> {
        parse_request(args.iter().map(OsString::from))
    }

    #[test]
    fn each_id_names_its_command() {
        for tool in ElevatedTool::ALL {
            assert_eq!(parse(&[tool.id()]), Ok(vec![tool]));
        }
    }

    #[test]
    fn a_list_of_different_ids_keeps_its_order() {
        assert_eq!(
            parse(&["journal-vacuum", "apt-clean", "coredumps"]),
            Ok(vec![
                ElevatedTool::JournalVacuum,
                ElevatedTool::AptClean,
                ElevatedTool::Coredumps
            ])
        );

        let every: Vec<&str> = ElevatedTool::ALL.iter().map(|tool| tool.id()).collect();
        assert_eq!(parse(&every), Ok(ElevatedTool::ALL.to_vec()));
    }

    #[test]
    fn an_empty_list_a_repeat_an_unknown_id_or_an_option_is_refused() {
        for args in [
            &[][..],
            &[""],
            &["apt-clean", "apt-clean"],
            &["apt-clean", "dnf-clean", "apt-clean"],
            &["apt-clean", "--extra"],
            &["--help"],
            &["-apt-clean"],
            &["apt-clean", "-"],
            &["apt-"],
            &["apt-clean-all"],
            &["Apt-Clean"],
            &["../apt-clean"],
            &["/usr/bin/apt-get"],
            &["journal-vacuum --rotate"],
            &["journal-vacuum", "--rotate"],
            &["apt-clean", "nope"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn more_ids_than_there_are_commands_are_refused() {
        let mut args: Vec<&str> = ElevatedTool::ALL.iter().map(|tool| tool.id()).collect();
        args.push("apt-clean");

        assert!(parse(&args).is_err());
    }

    #[test]
    fn an_argument_that_is_not_text_is_refused() {
        use std::os::unix::ffi::OsStringExt;

        let mut id = b"apt-clean".to_vec();
        id.push(0xff);

        assert!(parse_request([OsString::from_vec(id)]).is_err());
    }

    #[test]
    fn a_marker_line_tells_the_command_and_how_it_ended() {
        assert_eq!(
            marker_line(ElevatedTool::AptClean, Done::Code(0)),
            "platter-elevate: done apt-clean 0"
        );
        assert_eq!(
            marker_line(ElevatedTool::Paccache2, Done::Code(1)),
            "platter-elevate: done paccache-keep-2 1"
        );
        for (done, word) in [
            (Done::Signal, "signal"),
            (Done::Untrusted, "untrusted"),
            (Done::Spawn, "spawn"),
        ] {
            assert_eq!(
                marker_line(ElevatedTool::DnfClean, done),
                format!("platter-elevate: done dnf-clean {word}")
            );
        }
    }

    #[test]
    fn a_word_is_read_back_as_what_it_was_written_from() {
        for done in [
            Done::Code(0),
            Done::Code(3),
            Done::Code(-1),
            Done::Signal,
            Done::Untrusted,
            Done::Spawn,
        ] {
            assert_eq!(Done::from_word(&done.word()), Some(done));
        }
        for word in ["", "x", "1.5", "Signal"] {
            assert_eq!(Done::from_word(word), None, "{word:?}");
        }
    }

    /// Runs inside the process the next test starts: a child that writes to its standard output, started
    /// the way the helper starts one.
    #[test]
    fn probe_for_the_output_of_a_child() {
        if std::env::var_os(PROBE).is_none() {
            return;
        }

        let mut process = Command::new("sh");
        process.args(["-c", &format!("echo {TOKEN}")]);

        assert_eq!(run_child(process, stderr_as_stdout()), Done::Code(0));
    }

    #[test]
    fn what_a_child_writes_to_standard_output_goes_to_standard_error_of_the_helper() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "linux::elevate::tests::probe_for_the_output_of_a_child",
                "--nocapture",
            ])
            .env(PROBE, "1")
            .output()
            .unwrap();

        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(TOKEN));
        assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    }

    #[test]
    fn a_child_is_told_how_it_ended_and_its_output_goes_where_it_is_told() {
        use std::io::{Read, Seek};

        let mut sink = tempfile::tempfile().unwrap();
        let mut process = Command::new("sh");
        process.args(["-c", "echo kept; exit 3"]);

        let done = run_child(process, Stdio::from(sink.try_clone().unwrap()));

        assert_eq!(done, Done::Code(3));
        let mut text = String::new();
        sink.rewind().unwrap();
        sink.read_to_string(&mut text).unwrap();
        assert_eq!(text, "kept\n");
    }

    #[test]
    fn a_child_that_is_killed_or_cannot_start_is_told_apart() {
        let mut killed = Command::new("sh");
        killed.args(["-c", "kill -9 $$"]);

        assert_eq!(run_child(killed, Stdio::null()), Done::Signal);
        assert_eq!(
            run_child(Command::new("/no/such/platter-program"), Stdio::null()),
            Done::Spawn
        );
    }
}
