//! Whether macOS lets Platter into every folder (Full Disk Access).
//!
//! macOS has no call that answers this. Platter tries to open a file that only an app with Full Disk Access
//! may read: the user's privacy database, `~/Library/Application Support/com.apple.TCC/TCC.db`. Opening it
//! never brings up a prompt. A refusal (`EPERM`, `EACCES`) means the access is missing. Anything else (no such
//! file, no home) says nothing, and the UI then shows no notice. Linux and Windows have no such protection, so
//! the access is always there.
//!
//! An app signed ad hoc is a new app to macOS after every update, so the access is lost again then.

use std::io;
#[cfg(target_os = "macos")]
use std::path::Path;

/// What Platter knows about its access to every folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskAccess {
    /// Every folder can be read (and on Linux and Windows, always).
    Granted,
    /// macOS refused the probe: some folders are hidden and cannot be cleaned.
    Missing,
    /// The probe could not tell.
    Unknown,
}

/// The file only an app with Full Disk Access may open, under the home folder.
pub const PROBE: &str = "Library/Application Support/com.apple.TCC/TCC.db";

/// The access, from the outcome of opening [`PROBE`] for reading.
pub fn access_of(opened: &io::Result<()>) -> DiskAccess {
    match opened {
        Ok(()) => DiskAccess::Granted,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => DiskAccess::Missing,
        Err(_) => DiskAccess::Unknown,
    }
}

/// The access of this process. On macOS it opens [`PROBE`] in the home folder and closes it at once.
pub fn current_disk_access() -> DiskAccess {
    #[cfg(target_os = "macos")]
    {
        match std::env::home_dir().filter(|home| home.is_absolute()) {
            Some(home) => access_of(&probe(&home.join(PROBE))),
            None => DiskAccess::Unknown,
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        DiskAccess::Granted
    }
}

/// Opens `path` for reading, with no link followed, and closes it.
#[cfg(target_os = "macos")]
fn probe(path: &Path) -> io::Result<()> {
    use rustix::fs::{Mode, OFlags, open};

    open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(drop)
    .map_err(io::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opened_probe_means_the_access_is_granted() {
        assert_eq!(access_of(&Ok(())), DiskAccess::Granted);
    }

    #[test]
    fn a_refused_probe_means_the_access_is_missing() {
        let refused = io::Error::from(io::ErrorKind::PermissionDenied);

        assert_eq!(access_of(&Err(refused)), DiskAccess::Missing);
    }

    /// Privacy protection answers `EPERM`, a plain permission `EACCES`: both are a refusal.
    #[cfg(unix)]
    #[test]
    fn both_refusals_of_the_system_mean_the_access_is_missing() {
        // EPERM and EACCES, the same numbers on Linux and macOS.
        for code in [1, 13] {
            assert_eq!(
                access_of(&Err(io::Error::from_raw_os_error(code))),
                DiskAccess::Missing
            );
        }
    }

    #[test]
    fn any_other_failure_says_nothing() {
        assert_eq!(
            access_of(&Err(io::Error::from(io::ErrorKind::NotFound))),
            DiskAccess::Unknown
        );
        assert_eq!(access_of(&Err(io::Error::other("odd"))), DiskAccess::Unknown);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn linux_and_windows_always_have_the_access() {
        assert_eq!(current_disk_access(), DiskAccess::Granted);
    }
}
