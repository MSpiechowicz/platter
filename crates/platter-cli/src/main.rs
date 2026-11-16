//! `platter-cli`: headless front end over `platter-core` and `platter-platform`. It only parses
//! arguments and formats output; all domain logic lives in the libraries.

mod args;
mod commands;
mod format;
mod progress;

use std::error::Error;
use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;

use args::{Cli, Command};

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return report_usage(&error),
    };

    // Deep trees keep many directories open at once; the reader copes with a low limit, but a higher
    // one avoids the slower fallback. Failing to raise it is harmless.
    let _ = platter_platform::raise_fd_limit();

    let result = match cli.command {
        Command::Volumes(args) => commands::volumes::run(&args),
        Command::Scan(args) => commands::scan::run(&args),
        Command::Suggest(args) => commands::suggest::run(&args),
        Command::ExportFixture(args) => commands::export::run(&args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        // The reader of a pipe (`| head`) went away: it has all it wanted, which is not a failure.
        Err(error) if is_broken_pipe(error.as_ref()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(
                io::stderr(),
                "platter-cli: {}",
                format::display(&error.to_string())
            );
            ExitCode::FAILURE
        }
    }
}

/// Prints what clap has to say about the command line and returns its exit code.
///
/// `--help` and `--version` print clap's own text as it is. An error message, however, quotes what the
/// user typed (`unexpected argument '...' found`), and an argument can hold terminal control
/// sequences, so the message goes through [`format::display`] line by line like every other text that
/// may contain something untrusted.
fn report_usage(error: &clap::Error) -> ExitCode {
    let code = u8::try_from(error.exit_code()).map_or(ExitCode::FAILURE, ExitCode::from);

    if !error.use_stderr() {
        let _ = error.print();
        return code;
    }

    let rendered = error.render().to_string();
    let mut stderr = io::stderr().lock();
    for line in rendered.lines() {
        let _ = writeln!(stderr, "{}", format::display(line));
    }
    code
}

fn is_broken_pipe(error: &(dyn Error + 'static)) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
}
