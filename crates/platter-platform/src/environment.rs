//! Facts about the machine, for code that interprets a finished scan (see `platter_core::Environment`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use platter_core::{Environment, Os};

/// The home directory, the current time and the operating system of this process.
///
/// The home directory is canonicalized when possible, because scan roots are: a home reached through a
/// symlink would otherwise never line up with the tree.
pub fn current_environment() -> Environment {
    Environment {
        home: home_directory(),
        now: unix_now(),
        os: current_os(),
    }
}

fn home_directory() -> Option<PathBuf> {
    let home = std::env::home_dir().filter(|path| path.is_absolute())?;

    Some(std::fs::canonicalize(&home).unwrap_or(home))
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

const fn current_os() -> Os {
    if cfg!(target_os = "linux") {
        Os::Linux
    } else if cfg!(target_os = "macos") {
        Os::MacOs
    } else if cfg!(target_os = "windows") {
        Os::Windows
    } else {
        Os::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_plausible_clock_and_the_compiled_os() {
        let env = current_environment();

        // After 2024-01-01: the clock was read, not defaulted.
        assert!(env.now > 1_704_067_200, "now = {}", env.now);
        assert_eq!(cfg!(target_os = "linux"), env.os == Os::Linux);
    }

    #[test]
    fn the_home_directory_is_absolute_when_known() {
        if let Some(home) = current_environment().home {
            assert!(home.is_absolute(), "{home:?}");
        }
    }
}
