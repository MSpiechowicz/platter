//! The limit on open descriptors on macOS (package M1).
//!
//! The soft limit starts at 256, which a scan on many workers would reach at once. It is raised to the
//! hard limit, but never above `OPEN_MAX` (10240): macOS refuses a soft limit above that with `EINVAL`
//! even when the hard limit is "unlimited".

use std::io;

use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

/// What macOS accepts as a soft `RLIMIT_NOFILE` (`OPEN_MAX`).
const OPEN_MAX: u64 = 10240;

/// Raises the soft limit on open file descriptors as far as the system allows: to the hard limit, but
/// not above 10240. Returns the soft limit now in force (`None`: unlimited), or `Err` if it could not be
/// raised.
pub fn raise_fd_limit() -> io::Result<Option<u64>> {
    let Rlimit { current, maximum } = getrlimit(Resource::Nofile);

    let Some(wanted) = wanted_soft_limit(current, maximum) else {
        return Ok(current);
    };
    setrlimit(
        Resource::Nofile,
        Rlimit {
            current: Some(wanted),
            maximum,
        },
    )?;

    Ok(Some(wanted))
}

/// The soft limit to ask for, or `None` when the current one is as high as it can go. A hard limit
/// of `None` means unlimited.
fn wanted_soft_limit(current: Option<u64>, maximum: Option<u64>) -> Option<u64> {
    let target = maximum.map_or(OPEN_MAX, |hard| hard.min(OPEN_MAX));

    match current {
        Some(soft) if soft >= target => None,
        None => None,
        Some(_) => Some(target),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_soft_limit_is_raised_to_the_hard_limit_but_not_above_open_max() {
        assert_eq!(wanted_soft_limit(Some(256), None), Some(10240));
        assert_eq!(wanted_soft_limit(Some(256), Some(u64::MAX)), Some(10240));
        assert_eq!(wanted_soft_limit(Some(256), Some(4096)), Some(4096));
        assert_eq!(wanted_soft_limit(Some(256), Some(10240)), Some(10240));
    }

    #[test]
    fn a_limit_that_is_high_enough_is_left_alone() {
        assert_eq!(wanted_soft_limit(Some(10240), None), None);
        assert_eq!(wanted_soft_limit(Some(20000), None), None);
        assert_eq!(wanted_soft_limit(Some(4096), Some(4096)), None);
        assert_eq!(
            wanted_soft_limit(None, Some(4096)),
            None,
            "unlimited stays unlimited"
        );
    }

    #[test]
    fn this_process_ends_with_at_least_the_limit_it_had() {
        let before = getrlimit(Resource::Nofile).current;

        let after = raise_fd_limit().expect("raising the soft limit");

        match (before, after) {
            (Some(before), Some(after)) => assert!(after >= before),
            (Some(_), None) => panic!("a limit does not become unlimited"),
            (None, _) => {}
        }
        assert_eq!(getrlimit(Resource::Nofile).current, after);
    }
}
