pub mod clean;
pub mod export;
pub mod scan;
pub mod suggest;
pub mod volumes;

use std::error::Error;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use platter_core::dto::MAX_EXCLUDED;
use platter_core::{CancelToken, ScanConfig, Tree, UserRule, scan as run_scan};
use platter_platform::Platform;
use platter_platform::user_rules::{Loaded, load, rules_path};

use crate::format;
use crate::progress;

pub type CliResult<T> = Result<T, Box<dyn Error>>;

/// Writes `text` and a newline to stdout. A closed pipe is an error to return (`main` turns it into a
/// quiet exit), not a panic as with `println!`.
pub fn print_line(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();

    stdout.write_all(text.as_bytes())?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

/// Scans `path` with the platform's reader, showing live progress when attached to a terminal. Warnings
/// of the scan (see `ScanWarning`) are printed to stderr as lines of their own.
///
/// `max_entries` is an extra cap on the entries held in memory; the scan's memory is bounded by half of
/// what is free in any case (see [`platter_platform::suggested_memory_budget`]).
///
/// The root is resolved first and its identity pinned, as the app does, so the scan refuses to start
/// if the folder was swapped for another one in between. A path that cannot be resolved is left for
/// the scan to report.
///
/// `excluded` are folders to leave out, resolved the same way. One that cannot be resolved (it does not
/// exist, or is not a folder) is ignored with a warning on stderr.
pub fn scan_path(
    path: &Path,
    one_file_system: bool,
    max_entries: Option<u64>,
    excluded: &[PathBuf],
) -> CliResult<Tree> {
    let excluded = excluded_identities(excluded)?;

    let platform = Platform::current();
    let (path, root_identity) = match platter_platform::root::resolve_root(path) {
        Ok(resolved) => (resolved.path, resolved.identity),
        Err(_) => (path.to_path_buf(), None),
    };
    let path = path.as_path();
    let config = ScanConfig {
        one_file_system,
        max_entries,
        root_identity,
        max_memory_bytes: Some(platter_platform::suggested_memory_budget()),
        excluded,
        ..ScanConfig::default()
    };
    let show = progress::enabled();

    let result = run_scan(
        &platform.dir_source,
        path,
        &config,
        &CancelToken::new(),
        &|snapshot| {
            if show {
                progress::show(snapshot);
            }
        },
    );
    if show {
        progress::clear();
    }

    let tree = result?;
    for warning in tree.warnings() {
        let _ = writeln!(
            io::stderr(),
            "platter-cli: warning: {}",
            format::display(warning.message())
        );
    }
    Ok(tree)
}

/// The `(dev, ino)` of each folder to leave out. At most [`MAX_EXCLUDED`] are accepted, as in the app.
fn excluded_identities(excluded: &[PathBuf]) -> CliResult<Vec<(u64, u64)>> {
    if excluded.len() > MAX_EXCLUDED {
        return Err(format!(
            "at most {MAX_EXCLUDED} folders can be left out of a scan, not {}",
            excluded.len()
        )
        .into());
    }

    let mut identities = Vec::new();
    for path in excluded {
        match platter_platform::root::resolve_root(path) {
            Ok(resolved) => identities.extend(resolved.identity),
            Err(error) => {
                let _ = writeln!(
                    io::stderr(),
                    "platter-cli: warning: {} is not left out because it cannot be used: {}",
                    format::display(&path.display().to_string()),
                    format::display(&error.to_string())
                );
            }
        }
    }

    identities.sort_unstable();
    identities.dedup();
    Ok(identities)
}

/// What the user's rules file says (`$XDG_CONFIG_HOME/platter/rules.json`, the file the app writes): the
/// rules, none when there is no file, and the reason when the file cannot be used. The file is only read.
pub fn user_rules() -> Result<Vec<UserRule>, String> {
    let Some(path) = rules_path() else {
        return Ok(Vec::new());
    };

    match load(&path) {
        Loaded::Rules(file) => Ok(file.rules),
        Loaded::Broken(why) => Err(why),
    }
}
