//! The app's own folders (settings and data) and the files it keeps in them.
//!
//! Folders follow the XDG layout: `$XDG_CONFIG_HOME/platter` and `$XDG_DATA_HOME/platter`, by default
//! `~/.config/platter` and `~/.local/share/platter`. A relative `XDG_*` value is ignored, as the XDG
//! specification says (and as [`crate::environment`] does for the other variables it reads). Folders
//! are created with mode 0700 and files with mode 0600.
//!
//! On macOS the folders are `~/Library/Application Support/platter` (data) and
//! `~/Library/Application Support/platter/config` (settings), and `XDG_*` is ignored: it is not how a
//! Mac is set up. They are created with the same modes and held to the same rules as on Linux.
//!
//! On Windows the folders are `%APPDATA%\platter` (settings) and `%LOCALAPPDATA%\platter` (data), from the
//! known folders of the profile. They are not given a mode: a folder below the profile is closed to other
//! users by the profile's own access rules.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

const APP_DIR: &str = "platter";

/// The folder for the app's settings (`$XDG_CONFIG_HOME/platter`, on macOS
/// `~/Library/Application Support/platter/config`, on Windows `%APPDATA%\platter`), not created. `None`
/// without a home.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows_app_dir(known_folders::KnownFolder::RoamingAppData)
    }
    #[cfg(target_os = "macos")]
    {
        macos_config_dir(user_home())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        app_dir(std::env::var_os("XDG_CONFIG_HOME"), ".config")
    }
}

/// The folder for the app's data (`$XDG_DATA_HOME/platter`, on macOS
/// `~/Library/Application Support/platter`, on Windows `%LOCALAPPDATA%\platter`), not created. `None`
/// without a home.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows_app_dir(known_folders::KnownFolder::LocalAppData)
    }
    #[cfg(target_os = "macos")]
    {
        macos_data_dir(user_home())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        app_dir(std::env::var_os("XDG_DATA_HOME"), ".local/share")
    }
}

/// `platter` in the known folder `folder` of the profile.
#[cfg(windows)]
fn windows_app_dir(folder: known_folders::KnownFolder) -> Option<PathBuf> {
    known_folders::get_known_folder_path(folder)
        .filter(|path| path.is_absolute())
        .map(|base| base.join(APP_DIR))
}

/// The home folder, when it is an absolute path that is not `/`.
#[cfg_attr(windows, allow(dead_code))]
fn user_home() -> Option<PathBuf> {
    std::env::home_dir().filter(|home| home.is_absolute() && home.parent().is_some())
}

#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn app_dir(xdg: Option<OsString>, home_default: &str) -> Option<PathBuf> {
    xdg.map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| user_home().map(|home| home.join(home_default)))
        .map(|base| base.join(APP_DIR))
}

/// The data folder on macOS for the home folder `home`: `Library/Application Support/platter` in it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_data_dir(home: Option<PathBuf>) -> Option<PathBuf> {
    home.map(|home| home.join("Library/Application Support").join(APP_DIR))
}

/// The settings folder on macOS for the home folder `home`: `config` in the data folder.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_config_dir(home: Option<PathBuf>) -> Option<PathBuf> {
    macos_data_dir(home).map(|data| data.join("config"))
}

/// Creates `path` and the folders above it that are missing, each with mode 0700, and closes the
/// folder to group and others when it already existed more open. Fails when `path` is not a folder, or
/// (on Linux and macOS) when another user owns the folder it leads to (root may use any folder). There the
/// folder is opened once and checked and changed through that descriptor, so the path cannot be swapped
/// between the check and the change.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    make_dirs(path)?;

    close_dir_to_others(path)
}

/// The user id that owns what `meta` describes, where it is known.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn owner_of_meta(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;

    Some(meta.uid())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn owner_of_meta(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

/// A folder or file of another user is not ours to keep private data in: they could read or change it
/// whatever its mode is made to be. `owner` is `None` where it cannot be told. The owner must be the
/// effective user, except that root (for example under `sudo`) may use any folder it was pointed at,
/// since it could read and change it whoever owns it.
#[cfg(any(target_os = "linux", target_os = "macos"))]
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
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn owner_is_acceptable(owner: Option<u32>, euid: u32) -> bool {
    euid == 0 || owner.is_none_or(|owner| owner == euid)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
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
#[cfg(any(target_os = "linux", target_os = "macos"))]
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
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_folder_flags(root: bool) -> rustix::fs::OFlags {
    use rustix::fs::OFlags;

    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    if root { flags | OFlags::NOFOLLOW } else { flags }
}

/// Where there is no way to open a folder without following a link, the folder is checked by its path.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
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
/// file), and one that another user owns is refused (on Linux and macOS). Used for a download that continues where
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

/// See the Unix version: the same volume and file index, looked at through a fresh open that never
/// follows a link (a link at `path` is an error).
#[cfg(windows)]
pub fn is_same_file(file: &File, path: &Path) -> io::Result<bool> {
    use crate::windows::identity::{identity_of, identity_of_path};

    let again = identity_of_path(path)?;
    if again.is_reparse_point() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is a link and Platter does not follow it", path.display()),
        ));
    }
    let first = identity_of(file)?;

    Ok((first.volume_serial, first.file_index) == (again.volume_serial, again.file_index))
}

/// See the Unix version. Where file identities are not known it says `true`.
#[cfg(not(any(unix, windows)))]
pub fn is_same_file(_file: &File, _path: &Path) -> io::Result<bool> {
    Ok(true)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn no_follow(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn no_follow(_options: &mut std::fs::OpenOptions) {}

/// Closes a file that already existed more open than 0600 (the mode is set on the open file, so the
/// path cannot be swapped under it).
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn close_file_to_others(file: &File, meta: &std::fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if meta.permissions().mode() & 0o177 == 0 {
        return Ok(());
    }
    rustix::fs::fchmod(file, rustix::fs::Mode::from_raw_mode(0o600))?;

    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn close_file_to_others(_file: &File, _meta: &std::fs::Metadata) -> io::Result<()> {
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn private_mode(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn private_mode(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600);
}

/// Windows: a link at the name is opened as itself (it is then refused as not a regular file).
#[cfg(windows)]
fn private_mode(options: &mut std::fs::OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    options.custom_flags(crate::windows::identity::FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn private_mode(_options: &mut std::fs::OpenOptions) {}

/// What `O_NOFOLLOW` reports for a link at the path (`ELOOP`).
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn no_follow_refusal(error: &io::Error) -> bool {
    error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn no_follow_refusal(_error: &io::Error) -> bool {
    false
}

/// Takes an exclusive advisory lock on `file` without waiting: `Ok(true)` when it is held now, `Ok(false)`
/// when another open file description (this or another process) holds it. The lock belongs to the open
/// file description: it ends when `file` and every copy of its descriptor are closed, or the process ends.
/// Fails with `Unsupported` where locking is not available (the advisor does not run there).
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn lock_exclusive(file: &File) -> io::Result<bool> {
    use rustix::fs::{FlockOperation, flock};

    match flock(file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(true),
        Err(error) if error == rustix::io::Errno::WOULDBLOCK => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// See the Linux version. On Windows the lock is `LockFileEx` on the whole file (through `File::try_lock`),
/// held by the open handle.
#[cfg(windows)]
pub fn lock_exclusive(file: &File) -> io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

/// See the Linux version. Not available on this system.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn lock_exclusive(_file: &File) -> io::Result<bool> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// Bytes a normal user can still write on the filesystem holding `path` (a folder that exists).
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn free_space(path: &Path) -> io::Result<u64> {
    let stats = rustix::fs::statvfs(path)?;

    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}

/// Bytes the user can still write on the volume holding `path`, from the volume list of `sysinfo` (the
/// free space available to the calling user, which counts quotas).
#[cfg(windows)]
pub fn free_space(path: &Path) -> io::Result<u64> {
    let path = dunce::canonicalize(path)?;
    let disks = sysinfo::Disks::new_with_refreshed_list().map_err(io::Error::other)?;

    let mounts: Vec<(&Path, u64)> = disks
        .iter()
        .map(|disk| (disk.mount_point(), disk.available_space()))
        .collect();

    mount_holding(&path, &mounts).ok_or_else(|| io::Error::other("the volume of the path is not known"))
}

/// The value of the longest mount point that `path` is inside, compared without regard to ASCII case (as
/// Windows names are).
#[cfg_attr(not(windows), allow(dead_code))]
fn mount_holding(path: &Path, mounts: &[(&Path, u64)]) -> Option<u64> {
    let names = |path: &Path| -> Vec<String> {
        path.components()
            .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
            .collect()
    };
    let wanted = names(path);

    mounts
        .iter()
        .filter_map(|(mount, value)| {
            let own = names(mount);
            (wanted.len() >= own.len() && wanted[..own.len()] == own[..]).then_some((own.len(), *value))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, value)| value)
}

/// Bytes a normal user can still write on the filesystem holding `path`. Not known on this system.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn free_space(_path: &Path) -> io::Result<u64> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
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
            assert!(data.ends_with("platter"));
            if cfg!(target_os = "macos") {
                assert!(config.ends_with("platter/config"), "{config:?}");
            } else {
                assert!(config.ends_with("platter"), "{config:?}");
            }
        }
    }

    #[test]
    fn on_macos_the_folders_are_in_application_support_whatever_xdg_says() {
        let home = Some(PathBuf::from("/Users/me"));

        assert_eq!(
            macos_data_dir(home.clone()),
            Some(PathBuf::from("/Users/me/Library/Application Support/platter"))
        );
        assert_eq!(
            macos_config_dir(home),
            Some(PathBuf::from(
                "/Users/me/Library/Application Support/platter/config"
            ))
        );
        assert_eq!(macos_data_dir(None), None);
        assert_eq!(macos_config_dir(None), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn this_mac_keeps_its_folders_below_the_home() {
        let home = user_home().expect("the test user has a home folder");

        assert_eq!(data_dir(), macos_data_dir(Some(home.clone())));
        assert_eq!(config_dir(), macos_config_dir(Some(home)));
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

    #[test]
    fn a_private_file_is_created_0600_and_keeps_its_content() {
        use std::io::Write;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("model.partial");

        let mut first = open_private_file(&path).unwrap();
        first.write_all(b"abc").unwrap();
        drop(first);
        let second = open_private_file(&path).unwrap();

        assert_eq!(second.metadata().unwrap().len(), 3, "reopening does not truncate");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(second.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn a_folder_reached_through_a_link_above_it_is_still_used() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("real")).unwrap();
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("home")).unwrap();

        create_private_dir(&root.path().join("home/platter")).unwrap();

        assert!(root.path().join("real/platter").is_dir());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
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

    #[cfg(any(unix, windows))]
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
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

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn free_space_of_an_existing_folder_is_known_and_a_missing_one_fails() {
        let root = tempfile::tempdir().unwrap();

        assert!(free_space(root.path()).unwrap() > 0);
        assert!(free_space(&root.path().join("missing")).is_err());
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn a_lock_is_exclusive_between_open_files_and_ends_with_its_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".lock");

        let first = open_private_file(&path).unwrap();
        let second = open_private_file(&path).unwrap();

        assert!(lock_exclusive(&first).unwrap());
        assert!(!lock_exclusive(&second).unwrap(), "held by the first");
        // `flock` is by open file description, so asking again on the same file is fine; `LockFileEx` is
        // by handle and a second request on the holding handle is refused like any other.
        #[cfg(any(target_os = "linux", target_os = "macos"))]
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

    #[test]
    fn the_volume_of_a_path_is_the_longest_mount_point_that_holds_it_in_any_case() {
        let mounts: Vec<(&Path, u64)> = vec![
            (Path::new("/"), 1),
            (Path::new("/mnt"), 2),
            (Path::new("/mnt/data"), 3),
            (Path::new("/srv"), 4),
        ];

        assert_eq!(mount_holding(Path::new("/home/u"), &mounts), Some(1));
        assert_eq!(mount_holding(Path::new("/mnt/other"), &mounts), Some(2));
        assert_eq!(mount_holding(Path::new("/MNT/Data/x/y"), &mounts), Some(3));
        assert_eq!(mount_holding(Path::new("/mnt/data"), &mounts), Some(3));
        assert_eq!(mount_holding(Path::new("/mntx/data"), &mounts), Some(1));
        assert_eq!(mount_holding(Path::new("/srv"), &[]), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_folders_are_platter_in_the_roaming_and_local_app_data() {
        let config = config_dir().expect("the profile has an AppData folder");
        let data = data_dir().expect("the profile has an AppData folder");

        assert_ne!(config, data);
        for (folder, variable) in [(&config, "APPDATA"), (&data, "LOCALAPPDATA")] {
            assert!(folder.ends_with("platter"), "{folder:?}");
            assert!(folder.is_absolute(), "{folder:?}");
            assert!(!folder.to_string_lossy().starts_with(r"\\?\"), "{folder:?}");
            if let Some(base) = std::env::var_os(variable) {
                assert!(
                    folder.parent().unwrap().as_os_str().eq_ignore_ascii_case(&base),
                    "{folder:?} is not below %{variable}%"
                );
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_link_at_a_private_file_is_refused_and_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"precious").unwrap();
        let link = root.path().join("link");
        // A file symbolic link needs a privilege an ordinary account may not hold: nothing to test then.
        if std::os::windows::fs::symlink_file(&target, &link).is_err() {
            return;
        }

        let refused = open_private_file(&link).unwrap_err();

        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput, "{refused}");
        assert_eq!(std::fs::read(&target).unwrap(), b"precious");
    }

    #[cfg(windows)]
    #[test]
    fn a_missing_file_is_not_the_same_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("history.sqlite");
        let file = open_private_file(&path).unwrap();

        std::fs::remove_file(&path).unwrap();

        assert!(is_same_file(&file, &path).is_err());
    }
}
