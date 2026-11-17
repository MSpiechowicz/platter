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
/// folder to group and others when it already existed more open. Fails when `path` is not a folder, or
/// (on Linux) when another user owns the folder it leads to (root may use any folder). On Linux the
/// folder is opened once and checked and changed through that descriptor, so the path cannot be swapped
/// between the check and the change.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    make_dirs(path)?;

    close_dir_to_others(path)
}

/// The user id that owns what `meta` describes, where it is known.
#[cfg(target_os = "linux")]
fn owner_of_meta(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;

    Some(meta.uid())
}

#[cfg(not(target_os = "linux"))]
fn owner_of_meta(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

/// A folder or file of another user is not ours to keep private data in: they could read or change it
/// whatever its mode is made to be. `owner` is `None` where it cannot be told. The owner must be the
/// effective user, except that root (for example under `sudo`) may use any folder it was pointed at,
/// since it could read and change it whoever owns it.
#[cfg(target_os = "linux")]
fn refuse_another_owner(path: &Path, owner: Option<u32>) -> io::Result<()> {
    if owner_is_acceptable(owner, rustix::process::geteuid().as_raw()) {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} belongs to another user", path.display()),
    ))
}

/// Whether something owned by `owner` (`None`: not known) may be used by the user `euid`.
#[cfg(target_os = "linux")]
fn owner_is_acceptable(owner: Option<u32>, euid: u32) -> bool {
    euid == 0 || owner.is_none_or(|owner| owner == euid)
}

#[cfg(not(target_os = "linux"))]
fn refuse_another_owner(_path: &Path, _owner: Option<u32>) -> io::Result<()> {
    Ok(())
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

/// Opens the folder, checks what it is and who owns it, and closes it to group and others, all on the
/// open descriptor. A link at the name is followed (a data folder kept elsewhere by a link, as dotfile
/// managers make, is the user's choice): what is checked and changed is the folder the descriptor holds,
/// so the name cannot be swapped between the check and the change. Root never follows a link at the name:
/// root accepts any owner, so a link a user made (to `/etc`, say) would have root close a system folder.
#[cfg(target_os = "linux")]
fn close_dir_to_others(path: &Path) -> io::Result<()> {
    use rustix::fs::Mode;
    use std::os::unix::fs::PermissionsExt;

    let folder = rustix::fs::open(
        path,
        open_folder_flags(rustix::process::geteuid().is_root()),
        Mode::empty(),
    )
    .map_err(|errno| {
        let error = io::Error::from(errno);
        let is_link = std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_symlink());
        if is_link && (no_follow_refusal(&error) || errno == rustix::io::Errno::NOTDIR) {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} is a link and Platter does not follow it as administrator",
                    path.display()
                ),
            )
        } else {
            error
        }
    })?;
    let folder = File::from(folder);

    let meta = folder.metadata()?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a folder", path.display()),
        ));
    }
    refuse_another_owner(path, owner_of_meta(&meta))?;

    if meta.permissions().mode() & 0o077 == 0 {
        return Ok(());
    }
    rustix::fs::fchmod(&folder, Mode::from_raw_mode(0o700))?;

    Ok(())
}

/// How the data folder is opened: a link at its name is followed, except by root.
#[cfg(target_os = "linux")]
fn open_folder_flags(root: bool) -> rustix::fs::OFlags {
    use rustix::fs::OFlags;

    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    if root { flags | OFlags::NOFOLLOW } else { flags }
}

/// Where there is no way to open a folder without following a link, the folder is checked by its path.
#[cfg(not(target_os = "linux"))]
fn close_dir_to_others(path: &Path) -> io::Result<()> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a folder", path.display()),
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        }
    }

    Ok(())
}

/// Opens `path` for reading and writing, creating it with mode 0600 when it does not exist, and keeps
/// what it holds (no truncation). A link at `path` is refused (never followed), and so is anything but a
/// regular file. A file that already existed is closed to group and others (mode 0600, set on the open
/// file), and one that another user owns is refused (on Linux). Used for a download that continues where
/// it stopped, and for files the app keeps its data in.
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

    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    refuse_another_owner(path, owner_of_meta(&meta))?;
    refuse_hard_links(path, &meta)?;
    close_file_to_others(&file, &meta)?;

    Ok(file)
}

/// A file with a second name elsewhere is not one the app made: changing its mode would change that
/// other file too (with root and `fs.protected_hardlinks` off, a system file linked into the data folder).
#[cfg(unix)]
fn refuse_hard_links(path: &Path, meta: &std::fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    if meta.nlink() <= 1 {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "{} has another name elsewhere and Platter does not use it",
            path.display()
        ),
    ))
}

#[cfg(not(unix))]
fn refuse_hard_links(_path: &Path, _meta: &std::fs::Metadata) -> io::Result<()> {
    Ok(())
}

/// Whether `path` is, right now, the very file `file` is open on (the same device and inode), looked at
/// through a fresh open that never follows a link. For a program that hands the path to a library that
/// opens it again by name: this tells whether the name still leads to the file that was checked. A link
/// at `path` or a missing file is an error. Where file identities are not known it says `true`.
#[cfg(unix)]
pub fn is_same_file(file: &File, path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let again = options.open(path)?;

    let (first, second) = (file.metadata()?, again.metadata()?);

    Ok((first.dev(), first.ino()) == (second.dev(), second.ino()))
}

/// See the Unix version.
#[cfg(not(unix))]
pub fn is_same_file(_file: &File, _path: &Path) -> io::Result<bool> {
    Ok(true)
}

#[cfg(target_os = "linux")]
fn no_follow(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
}

#[cfg(all(unix, not(target_os = "linux")))]
fn no_follow(_options: &mut std::fs::OpenOptions) {}

/// Closes a file that already existed more open than 0600 (the mode is set on the open file, so the
/// path cannot be swapped under it).
#[cfg(target_os = "linux")]
fn close_file_to_others(file: &File, meta: &std::fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if meta.permissions().mode() & 0o177 == 0 {
        return Ok(());
    }
    rustix::fs::fchmod(file, rustix::fs::Mode::from_raw_mode(0o600))?;

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn close_file_to_others(_file: &File, _meta: &std::fs::Metadata) -> io::Result<()> {
    Ok(())
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
    fn a_file_that_was_more_open_is_closed_and_keeps_its_content() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("history.sqlite");
        std::fs::write(&path, b"keep").unwrap();

        for loose in [0o644, 0o666, 0o755, 0o640] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(loose)).unwrap();

            let file = open_private_file(&path).unwrap();

            let mode = file.metadata().unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "from {loose:o}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_file_or_folder_of_another_user_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let mine = rustix::process::geteuid().as_raw();

        assert!(refuse_another_owner(root.path(), Some(mine)).is_ok());
        assert!(refuse_another_owner(root.path(), None).is_ok());
        if mine == 0 {
            // Root may use any folder, so there is no other owner to refuse.
            return;
        }
        let refused = refuse_another_owner(root.path(), Some(mine.wrapping_add(1))).unwrap_err();

        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
        assert!(refused.to_string().contains("another user"), "{refused}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_folder_kept_elsewhere_by_a_link_is_used_and_closed_to_others() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        create_private_dir(&link).unwrap();

        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "the folder the link leads to is the data folder");
        assert!(
            std::fs::symlink_metadata(&link).unwrap().is_symlink(),
            "the link stays"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn root_does_not_follow_a_link_at_the_folder_and_a_user_does() {
        use rustix::fs::OFlags;

        assert!(open_folder_flags(true).contains(OFlags::NOFOLLOW));
        assert!(!open_folder_flags(false).contains(OFlags::NOFOLLOW));
        assert!(open_folder_flags(true).contains(OFlags::DIRECTORY | OFlags::CLOEXEC));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_with_another_name_elsewhere_is_refused_and_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let elsewhere = root.path().join("elsewhere");
        std::fs::write(&elsewhere, b"not ours").unwrap();
        std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o644)).unwrap();
        let data = root.path().join("data");
        std::fs::hard_link(&elsewhere, &data).unwrap();

        let refused = open_private_file(&data).unwrap_err();

        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput, "{refused}");
        let mode = std::fs::metadata(&elsewhere).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "the other name's file is not changed");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_folder_reached_through_a_link_above_it_is_still_used() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("real")).unwrap();
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("home")).unwrap();

        create_private_dir(&root.path().join("home/platter")).unwrap();

        assert!(root.path().join("real/platter").is_dir());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_folder_of_the_user_is_accepted_and_root_may_use_any_folder() {
        assert!(owner_is_acceptable(Some(1000), 1000));
        assert!(owner_is_acceptable(None, 1000));
        assert!(
            !owner_is_acceptable(Some(1001), 1000),
            "another owner, for a normal user"
        );
        assert!(
            !owner_is_acceptable(Some(0), 1000),
            "root's own folder is not a normal user's"
        );
        assert!(
            owner_is_acceptable(Some(1000), 0),
            "under sudo, the user's folder"
        );
        assert!(owner_is_acceptable(Some(0), 0));
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

    #[cfg(unix)]
    #[test]
    fn a_name_is_the_same_file_until_another_file_takes_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("history.sqlite");
        let file = open_private_file(&path).unwrap();

        assert!(is_same_file(&file, &path).unwrap());

        // Moved away, and another file made at the name (the first is still open, so no inode is reused).
        std::fs::rename(&path, root.path().join("moved")).unwrap();
        drop(open_private_file(&path).unwrap());

        assert!(!is_same_file(&file, &path).unwrap());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_link_or_a_missing_file_is_not_the_same_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("history.sqlite");
        let file = open_private_file(&path).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();

        assert!(is_same_file(&file, &link).is_err(), "a link is never followed");

        std::fs::remove_file(&path).unwrap();

        assert!(is_same_file(&file, &path).is_err());
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
