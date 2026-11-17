//! The app's own folders (settings and data) and the files it keeps in them.
//!
//! Folders follow the XDG layout: `$XDG_CONFIG_HOME/platter` and `$XDG_DATA_HOME/platter`, by default
//! `~/.config/platter` and `~/.local/share/platter`. A relative `XDG_*` value is ignored, as the XDG
//! specification says (and as [`crate::environment`] does for the other variables it reads). Folders
//! are created with mode 0700 and files with mode 0600.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

const APP_DIR: &str = "platter";

/// The folder for the app's settings (`$XDG_CONFIG_HOME/platter`), not created. `None` without a home.
pub fn config_dir() -> Option<PathBuf> {
    app_dir(std::env::var_os("XDG_CONFIG_HOME"), ".config")
}

/// The folder for the app's data (`$XDG_DATA_HOME/platter`), not created. `None` without a home.
pub fn data_dir() -> Option<PathBuf> {
    app_dir(std::env::var_os("XDG_DATA_HOME"), ".local/share")
}

fn app_dir(xdg: Option<OsString>, home_default: &str) -> Option<PathBuf> {
    let home = || std::env::home_dir().filter(|home| home.is_absolute() && home.parent().is_some());

    xdg.map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home().map(|home| home.join(home_default)))
        .map(|base| base.join(APP_DIR))
}

/// Creates `path` and the folders above it that are missing, each with mode 0700, and closes the
/// folder to group and others when it already existed more open. Fails when `path` is not a folder.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    make_dirs(path)?;

    if !std::fs::metadata(path)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a folder", path.display()),
        ));
    }
    close_to_others(path)
}

#[cfg(unix)]
fn make_dirs(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

#[cfg(not(unix))]
fn make_dirs(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)
}

#[cfg(unix)]
fn close_to_others(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o077 == 0 {
        return Ok(());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn close_to_others(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Opens `path` for reading and writing, creating it with mode 0600 when it does not exist, and keeps
/// what it holds (no truncation). A link at `path` is refused (never followed), and so is anything but a
/// regular file. Used for a download that continues where it stopped.
pub fn open_private_file(path: &Path) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    private_mode(&mut options);

    let file = options.open(path).map_err(|error| {
        if no_follow_refusal(&error) {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is a link and Platter does not follow it", path.display()),
            )
        } else {
            error
        }
    })?;

    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    Ok(file)
}

#[cfg(target_os = "linux")]
fn private_mode(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
}

#[cfg(all(unix, not(target_os = "linux")))]
fn private_mode(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600);
}

#[cfg(not(unix))]
fn private_mode(_options: &mut std::fs::OpenOptions) {}

/// What `O_NOFOLLOW` reports for a link at the path (`ELOOP`).
#[cfg(target_os = "linux")]
fn no_follow_refusal(error: &io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())
}

#[cfg(not(target_os = "linux"))]
fn no_follow_refusal(_error: &io::Error) -> bool {
    false
}

/// Takes an exclusive advisory lock on `file` without waiting: `Ok(true)` when it is held now, `Ok(false)`
/// when another open file description (this or another process) holds it. The lock belongs to the open
/// file description: it ends when `file` and every copy of its descriptor are closed, or the process ends.
/// Fails with `Unsupported` where locking is not available (the advisor does not run there).
#[cfg(target_os = "linux")]
pub fn lock_exclusive(file: &File) -> io::Result<bool> {
    use rustix::fs::{FlockOperation, flock};

    match flock(file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(true),
        Err(error) if error == rustix::io::Errno::WOULDBLOCK => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// See the Linux version. Not available on this system.
#[cfg(not(target_os = "linux"))]
pub fn lock_exclusive(_file: &File) -> io::Result<bool> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// Bytes a normal user can still write on the filesystem holding `path` (a folder that exists).
#[cfg(target_os = "linux")]
pub fn free_space(path: &Path) -> io::Result<u64> {
    let stats = rustix::fs::statvfs(path)?;

    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}

/// Bytes a normal user can still write on the filesystem holding `path`. Not known on this system.
#[cfg(not(target_os = "linux"))]
pub fn free_space(_path: &Path) -> io::Result<u64> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_xdg_value_wins_and_a_relative_one_is_ignored() {
        let absolute = app_dir(Some("/srv/cfg".into()), ".config");
        let relative = app_dir(Some("cfg".into()), ".config");
        let empty = app_dir(Some("".into()), ".config");
        let unset = app_dir(None, ".config");

        assert_eq!(absolute, Some(PathBuf::from("/srv/cfg/platter")));
        assert_eq!(relative, unset, "a relative value falls back to the home default");
        assert_eq!(empty, unset);
        if let Some(path) = unset {
            assert!(path.is_absolute());
            assert!(path.ends_with(".config/platter"), "{path:?}");
        }
    }

    #[test]
    fn config_and_data_are_different_places_below_the_app_name() {
        if let (Some(config), Some(data)) = (config_dir(), data_dir()) {
            assert_ne!(config, data);
            assert!(config.ends_with("platter") && data.ends_with("platter"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn folders_are_created_closed_to_others_and_an_open_one_is_closed() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let deep = root.path().join("a/b/c");

        create_private_dir(&deep).unwrap();
        create_private_dir(&deep).unwrap();

        for path in [root.path().join("a"), root.path().join("a/b"), deep.clone()] {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{path:?}");
        }

        std::fs::set_permissions(&deep, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir(&deep).unwrap();
        let mode = std::fs::metadata(&deep).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn a_file_where_a_folder_should_be_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, b"x").unwrap();

        let error = create_private_dir(&file).unwrap_err();

        assert!(matches!(
            error.kind(),
            io::ErrorKind::NotADirectory | io::ErrorKind::AlreadyExists
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_private_file_is_created_0600_and_keeps_its_content() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("model.partial");

        let mut first = open_private_file(&path).unwrap();
        first.write_all(b"abc").unwrap();
        drop(first);
        let second = open_private_file(&path).unwrap();

        assert_eq!(second.metadata().unwrap().len(), 3, "reopening does not truncate");
        assert_eq!(second.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_link_and_a_folder_are_refused_as_private_files() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"precious").unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let through_link = open_private_file(&link).unwrap_err();
        let folder = open_private_file(root.path()).unwrap_err();

        assert_eq!(through_link.kind(), io::ErrorKind::InvalidInput);
        assert!(through_link.to_string().contains("link"), "{through_link}");
        assert_eq!(std::fs::read(&target).unwrap(), b"precious");
        assert!(folder.kind() == io::ErrorKind::InvalidInput || folder.raw_os_error().is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn free_space_of_an_existing_folder_is_known_and_a_missing_one_fails() {
        let root = tempfile::tempdir().unwrap();

        assert!(free_space(root.path()).unwrap() > 0);
        assert!(free_space(&root.path().join("missing")).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_lock_is_exclusive_between_open_files_and_ends_with_its_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".lock");

        let first = open_private_file(&path).unwrap();
        let second = open_private_file(&path).unwrap();

        assert!(lock_exclusive(&first).unwrap());
        assert!(!lock_exclusive(&second).unwrap(), "held by the first");
        assert!(
            lock_exclusive(&first).unwrap(),
            "asking again on the same file is fine"
        );

        drop(first);

        assert!(
            lock_exclusive(&second).unwrap(),
            "free once the first file is closed"
        );
    }
}
