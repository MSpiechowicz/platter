//! Running the tools that own a cache (see `platter_core::ToolRunner`).
//!
//! A program is looked up only in the system's own program folders, and only accepted when it is a
//! regular file owned by root that no one else may change; `PATH` is never used, so a program planted in
//! a user's folder cannot be run, let alone as the administrator. It runs with an empty environment
//! (plus a fixed `PATH` and `LC_ALL=C`). Elevated commands run through `pkexec`, found the same way, which
//! shows the system's password prompt; `pkexec` answers 126 when the prompt was dismissed and 127 when
//! authorization failed.
//!
//! The deb and rpm packages carry a root helper (see [`super::elevate`]) and a polkit policy for it. A batch
//! of elevated commands of the closed list (`ElevatedTool`) runs as one `pkexec <helper> <id> <id> ...` when
//! the helper is installed where the package puts it, and is trusted by the same rules as a program: one
//! prompt for the whole batch, and the helper tells by a line per command how each one ended. A single
//! command, anything off the list, and every run without the helper (an AppImage, a source build) runs as
//! `pkexec <program> <args>`, one prompt each. A helper that does not know the ids (an older one) makes the
//! runner ask once more for each command without it.

use std::ffi::OsString;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use platter_core::ports::run_each;
use platter_core::{ElevatedTool, ToolCommand, ToolFailure, ToolRunner};

use super::elevate::{DONE_PREFIX, Done, EXIT_USAGE, MESSAGE_PREFIX};

/// Where the system keeps its programs. Nothing else is searched.
const PROGRAM_FOLDERS: [&str; 4] = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"];
pub(crate) const SAFE_PATH: &str = "/usr/bin:/usr/sbin:/bin:/sbin";
/// Where the deb and rpm packages put the root helper. The polkit policy names this same path.
pub const HELPER: &str = "/usr/lib/platter/platter-elevate";
/// How much of a failing tool's output is kept for the message.
const MESSAGE_LINES: usize = 3;
/// What a failed command of a batch of several says: the helper sends all the commands' output into the
/// same stream, so none can be told from the others.
const BATCH_MESSAGE: &str = "its own output is not kept when the commands run together";
/// What a failed command that ran alone says when it wrote nothing.
const SILENT_MESSAGE: &str = "it gave no message";

/// [`ToolRunner`] for Linux.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxToolRunner;

impl ToolRunner for LinuxToolRunner {
    fn run(&self, command: &ToolCommand<'_>) -> Result<(), ToolFailure> {
        // One command of the list is a batch of one: the helper answers by its line, not by an exit code.
        if installed_helper().is_some() && batch_tools(std::slice::from_ref(command)).is_some() {
            return self
                .run_batch(std::slice::from_ref(command))
                .pop()
                .unwrap_or(Err(ToolFailure::Io("no answer".to_owned())));
        }

        Direct.run(command)
    }

    /// Elevated commands of the closed list go through the helper in one `pkexec`, so one password
    /// prompt covers them all. Anything else runs one by one.
    fn run_batch(&self, commands: &[ToolCommand<'_>]) -> Vec<Result<(), ToolFailure>> {
        let Some(helper) = installed_helper() else {
            return run_each(self, commands);
        };
        let Some(tools) = batch_tools(commands) else {
            return run_each(self, commands);
        };

        // A program that is not installed is not asked about, so it never costs a prompt.
        let mut answers: Vec<Option<Result<(), ToolFailure>>> = vec![None; commands.len()];
        let mut asked: Vec<usize> = Vec::new();
        for (position, command) in commands.iter().enumerate() {
            if find_program(command.program).is_some() {
                asked.push(position);
            } else {
                answers[position] = Some(Err(ToolFailure::NotInstalled));
            }
        }

        if !asked.is_empty() {
            let asked_tools: Vec<ElevatedTool> = asked.iter().map(|&position| tools[position]).collect();

            for (&position, answer) in
                asked
                    .iter()
                    .zip(self.run_helper(&helper, &asked_tools, commands, &asked))
            {
                answers[position] = Some(answer);
            }
        }

        answers.into_iter().flatten().collect()
    }
}

impl LinuxToolRunner {
    /// One `pkexec` for `tools` (the commands at `asked` of `commands`), and an answer for each.
    fn run_helper(
        &self,
        helper: &Path,
        tools: &[ElevatedTool],
        commands: &[ToolCommand<'_>],
        asked: &[usize],
    ) -> Vec<Result<(), ToolFailure>> {
        let all = |failure: ToolFailure| vec![Err(failure); tools.len()];

        let Some(pkexec) = find_program("pkexec") else {
            return all(ToolFailure::NoElevation);
        };

        let output = match pkexec_output(&pkexec, &helper_argv(helper, tools)) {
            Ok(output) => output,
            Err(failure) => return all(failure),
        };

        match interpret_batch(&output, tools) {
            Batch::Answers(answers) => answers,
            Batch::Refused => all(ToolFailure::NotAuthorized),
            // An older helper: ask for each program itself, as without a helper.
            Batch::NotKnown => {
                let own: Vec<ToolCommand<'_>> = asked.iter().map(|&position| commands[position]).collect();

                run_each(&Direct, &own)
            }
        }
    }
}

/// What `pkexec` is given to run: the helper and the ids, in order.
fn helper_argv(helper: &Path, tools: &[ElevatedTool]) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec![helper.into()];
    argv.extend(tools.iter().map(|tool| OsString::from(tool.id())));

    argv
}

/// The commands of the closed list the commands are, when the helper can take all of them in one go:
/// every one elevated and on the list, none twice, and no more than the helper takes.
fn batch_tools(commands: &[ToolCommand<'_>]) -> Option<Vec<ElevatedTool>> {
    if commands.is_empty() || commands.len() > ElevatedTool::ALL.len() {
        return None;
    }

    let mut tools: Vec<ElevatedTool> = Vec::new();
    for command in commands {
        let tool = ElevatedTool::find(command.program, command.args).filter(|_| command.elevated)?;
        if tools.contains(&tool) {
            return None;
        }
        tools.push(tool);
    }

    Some(tools)
}

/// What a batch run through the helper came to.
#[derive(Debug, PartialEq, Eq)]
enum Batch {
    /// `pkexec` turned the request down (dismissed or not authorized): nothing ran.
    Refused,
    /// The helper did not know the ids (exit 64 and its own message): nothing ran.
    NotKnown,
    /// How each command ended, in the order of the request.
    Answers(Vec<Result<(), ToolFailure>>),
}

fn interpret_batch(output: &Output, tools: &[ElevatedTool]) -> Batch {
    let code = output.status.code();
    if matches!(code, Some(126 | 127)) && refused_by_pkexec(&output.stderr) {
        return Batch::Refused;
    }
    if helper_did_not_know_the_command(output) {
        return Batch::NotKnown;
    }

    let message = failure_message(&output.stderr, tools.len());
    let marked = read_markers(&output.stdout, tools);
    let answers = marked
        .into_iter()
        .map(|done| match done {
            Some(Done::Code(0)) => Ok(()),
            Some(Done::Code(code)) => Err(ToolFailure::Failed {
                code: Some(code),
                message: message.clone(),
            }),
            Some(Done::Signal) => Err(ToolFailure::Failed {
                code: None,
                message: message.clone(),
            }),
            Some(Done::Untrusted) => Err(ToolFailure::NotInstalled),
            Some(Done::Spawn) => Err(ToolFailure::Failed {
                code: None,
                message: "it could not be started".to_owned(),
            }),
            // The helper stopped before it, or its line is lost.
            None => Err(ToolFailure::Failed {
                code,
                message: "the helper gave no result for it".to_owned(),
            }),
        })
        .collect();

    Batch::Answers(answers)
}

/// What a command of a batch of `count` that ended badly says. The helper sends every command's output
/// to its standard error, which `pkexec` passes on: when the command ran alone, that is its own output.
fn failure_message(stderr: &[u8], count: usize) -> String {
    if count != 1 {
        return BATCH_MESSAGE.to_owned();
    }

    let own = last_lines(stderr, &[]);
    if own.is_empty() {
        return SILENT_MESSAGE.to_owned();
    }

    own
}

/// How each of `tools` ended, as the helper's lines on `stdout` tell. The helper's standard output holds
/// nothing but these lines (the commands' own output goes to its standard error), so a marker counts only
/// at the start of a line, and any other line is ignored.
fn read_markers(stdout: &[u8], tools: &[ElevatedTool]) -> Vec<Option<Done>> {
    let mut found: Vec<Option<Done>> = vec![None; tools.len()];
    let mut next = 0;

    for line in String::from_utf8_lossy(stdout).lines() {
        let Some(rest) = line.strip_prefix(DONE_PREFIX) else {
            continue;
        };

        // The helper answers in the order of the request, so a line can only be for a later command.
        let Some((position, word)) = tools.iter().enumerate().skip(next).find_map(|(position, tool)| {
            let word = rest.strip_prefix(tool.id())?.strip_prefix(' ')?;
            Some((position, word))
        }) else {
            continue;
        };
        let Some(done) = Done::from_word(word.trim()) else {
            continue;
        };

        found[position] = Some(done);
        next = position + 1;
    }

    found
}

/// Runs each command as itself, never through the helper: `pkexec <program> <args>` when it is elevated.
struct Direct;

impl ToolRunner for Direct {
    fn run(&self, command: &ToolCommand<'_>) -> Result<(), ToolFailure> {
        // Looked up as the user: a tool that is not installed is not asked about.
        let program = find_program(command.program).ok_or(ToolFailure::NotInstalled)?;

        let output = if command.elevated {
            let pkexec = find_program("pkexec").ok_or(ToolFailure::NoElevation)?;
            let mut argv: Vec<OsString> = vec![program.into_os_string()];
            argv.extend(command.args.iter().map(OsString::from));

            pkexec_output(&pkexec, &argv)?
        } else {
            let mut process = Command::new(&program);
            // The resolved file runs under its own name, as multi-call programs (busybox) expect.
            process.arg0(command.program).args(command.args);
            start(process)?
        };
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

/// The helper, when it is installed where the package puts it: the file at [`HELPER`] itself (no link),
/// owned by root, and in folders that only root may change.
fn installed_helper() -> Option<PathBuf> {
    installed_helper_at(Path::new(HELPER))
}

fn installed_helper_at(path: &Path) -> Option<PathBuf> {
    let resolved = trusted_path(path)?;

    (resolved == path).then_some(resolved)
}

fn pkexec_output(pkexec: &Path, argv: &[OsString]) -> Result<Output, ToolFailure> {
    let mut process = Command::new(pkexec);
    process.arg0("pkexec").args(argv);

    start(process)
}

/// Runs a process with an empty environment but for a fixed `PATH` and `LC_ALL`, and no input.
fn start(mut process: Command) -> Result<Output, ToolFailure> {
    process
        .env_clear()
        .env("PATH", SAFE_PATH)
        .env("LC_ALL", "C")
        .stdin(Stdio::null());

    process
        .output()
        .map_err(|error| ToolFailure::Io(error.to_string()))
}

/// Whether the helper itself said it does not know the id (exit 64 and its own message), as opposed to a
/// program it ran that ended with 64.
fn helper_did_not_know_the_command(output: &Output) -> bool {
    output.status.code() == Some(i32::from(EXIT_USAGE))
        && String::from_utf8_lossy(&output.stderr)
            .lines()
            .any(|line| line.starts_with(MESSAGE_PREFIX))
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
    fn a_helper_that_is_not_the_installed_file_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("platter-elevate");
        std::fs::write(&fake, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink("/bin/sh", &link).unwrap();

        // Owned by the user.
        assert!(installed_helper_at(&fake).is_none() || rustix::process::getuid().is_root());
        // A root-owned program, but only reached through a link.
        assert!(installed_helper_at(&link).is_none());
        assert!(installed_helper_at(Path::new("/no/such/platter-elevate")).is_none());
    }

    fn command_of(tool: ElevatedTool) -> ToolCommand<'static> {
        ToolCommand {
            program: tool.program(),
            args: tool.args(),
            elevated: true,
        }
    }

    #[test]
    fn the_helper_is_given_the_ids_in_order() {
        assert_eq!(
            helper_argv(
                Path::new(HELPER),
                &[ElevatedTool::JournalVacuum, ElevatedTool::AptClean]
            ),
            [
                OsString::from(HELPER),
                OsString::from("journal-vacuum"),
                OsString::from("apt-clean")
            ]
        );
    }

    #[test]
    fn only_a_list_of_different_elevated_commands_of_the_closed_list_goes_to_the_helper() {
        let every: Vec<ToolCommand<'_>> = ElevatedTool::ALL.into_iter().map(command_of).collect();
        assert_eq!(batch_tools(&every), Some(ElevatedTool::ALL.to_vec()));

        let two = [
            command_of(ElevatedTool::AptClean),
            command_of(ElevatedTool::DnfClean),
        ];
        assert_eq!(
            batch_tools(&two),
            Some(vec![ElevatedTool::AptClean, ElevatedTool::DnfClean])
        );

        assert_eq!(batch_tools(&[]), None);
        // The same command twice, which the helper refuses.
        assert_eq!(batch_tools(&[two[0], two[0]]), None);
        // Not elevated.
        let plain = ToolCommand {
            elevated: false,
            ..two[0]
        };
        assert_eq!(batch_tools(&[two[1], plain]), None);
        // Off the list.
        let off = ToolCommand {
            program: "apt-get",
            args: &["autoremove"],
            elevated: true,
        };
        assert_eq!(batch_tools(&[two[0], off]), None);
    }

    fn output(code: i32, stderr: &str) -> Output {
        output_with(code, "", stderr)
    }

    fn output_with(code: i32, stdout: &str, stderr: &str) -> Output {
        use std::os::unix::process::ExitStatusExt;

        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn only_the_helpers_own_usage_error_means_it_did_not_know_the_command() {
        assert!(helper_did_not_know_the_command(&output(
            64,
            "platter-elevate: not the id of a command\n"
        )));
        // The program the helper ran ended with 64.
        assert!(!helper_did_not_know_the_command(&output(
            64,
            "usage: paccache ...\n"
        )));
        assert!(!helper_did_not_know_the_command(&output(64, "")));
        assert!(!helper_did_not_know_the_command(&output(
            69,
            "platter-elevate: paccache is not installed in a system folder\n"
        )));
        assert!(!helper_did_not_know_the_command(&output(1, "")));
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

    const REFUSED: &str = "Error executing command as another user: Not authorized\n";

    #[test]
    fn a_refusal_by_pkexec_answers_every_command_as_not_authorized_and_nothing_ran() {
        let tools = [ElevatedTool::AptClean, ElevatedTool::JournalVacuum];

        for code in [126, 127] {
            assert_eq!(
                interpret_batch(&output(code, REFUSED), &tools),
                Batch::Refused,
                "{code}"
            );
        }
        // The same codes without pkexec's own line are the helper's or a tool's.
        assert!(matches!(
            interpret_batch(&output(127, "sh: not found\n"), &tools),
            Batch::Answers(_)
        ));
    }

    #[test]
    fn a_helper_that_does_not_know_the_ids_makes_the_runner_ask_again() {
        let tools = [ElevatedTool::AptClean, ElevatedTool::JournalVacuum];

        assert_eq!(
            interpret_batch(&output(64, "platter-elevate: not the id of a command\n"), &tools),
            Batch::NotKnown
        );
    }

    #[test]
    fn each_command_is_answered_by_its_own_line() {
        let tools = [
            ElevatedTool::AptClean,
            ElevatedTool::DnfClean,
            ElevatedTool::JournalVacuum,
            ElevatedTool::Coredumps,
            ElevatedTool::Paccache1,
        ];
        let stdout = "Reading package lists\n\
                      platter-elevate: done apt-clean 0\n\
                      platter-elevate: done dnf-clean 3\n\
                      platter-elevate: done journal-vacuum untrusted\n\
                      platter-elevate: done coredumps signal\n\
                      platter-elevate: done paccache-keep-1 spawn\n";

        let Batch::Answers(answers) = interpret_batch(&output_with(0, stdout, ""), &tools) else {
            panic!("answers were expected");
        };

        assert_eq!(answers[0], Ok(()));
        assert!(
            matches!(&answers[1], Err(ToolFailure::Failed { code: Some(3), message }) if !message.is_empty())
        );
        assert_eq!(answers[2], Err(ToolFailure::NotInstalled));
        assert!(matches!(&answers[3], Err(ToolFailure::Failed { code: None, .. })));
        assert!(matches!(&answers[4], Err(ToolFailure::Failed { code: None, .. })));
    }

    #[test]
    fn a_command_without_a_line_failed_and_the_others_keep_their_answers() {
        let tools = [
            ElevatedTool::AptClean,
            ElevatedTool::DnfClean,
            ElevatedTool::JournalVacuum,
        ];
        let stdout = "platter-elevate: done apt-clean 0\nplatter-elevate: done journal-vacuum 0\n";

        let Batch::Answers(answers) = interpret_batch(&output_with(0, stdout, ""), &tools) else {
            panic!("answers were expected");
        };

        assert_eq!(answers[0], Ok(()));
        assert!(matches!(&answers[1], Err(ToolFailure::Failed { .. })));
        assert_eq!(answers[2], Ok(()));
    }

    #[test]
    fn a_helper_that_stopped_without_lines_fails_every_command_with_its_exit_code() {
        let tools = [ElevatedTool::AptClean, ElevatedTool::DnfClean];

        let Batch::Answers(answers) =
            interpret_batch(&output(77, "platter-elevate: not running as root\n"), &tools)
        else {
            panic!("answers were expected");
        };

        assert!(
            answers
                .iter()
                .all(|answer| matches!(answer, Err(ToolFailure::Failed { code: Some(77), .. })))
        );
    }

    #[test]
    fn a_line_for_a_command_not_asked_for_or_with_a_word_that_means_nothing_is_not_an_answer() {
        let tools = [ElevatedTool::AptClean, ElevatedTool::DnfClean];
        let stdout = "platter-elevate: done apt-clean 0\n\
                      platter-elevate: done coredumps 0\n\
                      platter-elevate: done dnf-clean banana\n\
                      platter-elevate: done dnf-clean 0\n";

        assert_eq!(
            read_markers(stdout.as_bytes(), &tools),
            [Some(Done::Code(0)), Some(Done::Code(0))]
        );
    }

    #[test]
    fn a_marker_counts_only_at_the_start_of_a_line() {
        let tools = [ElevatedTool::AptClean, ElevatedTool::DnfClean];
        let stdout = [
            "done, no newlineplatter-elevate: done apt-clean 0",
            "echo platter-elevate: done dnf-clean 0",
            " platter-elevate: done dnf-clean 0",
        ]
        .join("\n");

        assert_eq!(read_markers(stdout.as_bytes(), &tools), [None, None]);
    }

    #[test]
    fn a_failed_command_that_ran_alone_keeps_what_it_wrote_and_one_of_several_does_not() {
        let alone = [ElevatedTool::AptClean];
        let stdout = "platter-elevate: done apt-clean 100\n";
        let stderr = "E: Could not get lock\nE: Unable to lock\n";

        let Batch::Answers(answers) = interpret_batch(&output_with(0, stdout, stderr), &alone) else {
            panic!("answers were expected");
        };
        assert!(
            matches!(&answers[0], Err(ToolFailure::Failed { code: Some(100), message })
                if message == "E: Could not get lock\nE: Unable to lock")
        );

        // A command that ran alone and wrote nothing does not claim its output is kept elsewhere.
        let Batch::Answers(answers) = interpret_batch(&output_with(0, stdout, ""), &alone) else {
            panic!("answers were expected");
        };
        assert!(matches!(&answers[0], Err(ToolFailure::Failed { message, .. })
            if message == "it gave no message"));

        let two = [ElevatedTool::AptClean, ElevatedTool::DnfClean];
        let stdout = "platter-elevate: done apt-clean 100\nplatter-elevate: done dnf-clean 0\n";
        let Batch::Answers(answers) = interpret_batch(&output_with(0, stdout, stderr), &two) else {
            panic!("answers were expected");
        };
        assert!(matches!(&answers[0], Err(ToolFailure::Failed { message, .. })
            if message.contains("not kept when the commands run together")));
    }

    #[test]
    fn a_line_for_an_earlier_command_is_not_taken_again() {
        let tools = [ElevatedTool::AptClean, ElevatedTool::DnfClean];
        let stdout = "platter-elevate: done dnf-clean 1\nplatter-elevate: done apt-clean 0\n";

        assert_eq!(
            read_markers(stdout.as_bytes(), &tools),
            [None, Some(Done::Code(1))]
        );
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
