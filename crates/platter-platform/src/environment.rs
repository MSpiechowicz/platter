//! Facts about the machine, for code that interprets a finished scan (see `platter_core::Environment`).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use platter_core::{Environment, Os, UserDir};

/// `user-dirs.dirs` is a few hundred bytes; anything much larger is not that file.
#[cfg(not(any(windows, target_os = "macos")))]
const USER_DIRS_MAX_BYTES: u64 = 64 * 1024;

/// The home directory, the user's personal folders, the current time and the operating system of this
/// process.
///
/// The home directory is canonicalized when possible, because scan roots are: a home reached through a
/// symlink would otherwise never line up with the tree.
pub fn current_environment() -> Environment {
    let home = home_directory();
    let user_dirs = home.as_deref().map(user_directories).unwrap_or_default();

    Environment {
        home,
        user_dirs,
        now: unix_now(),
        os: current_os(),
        system_root: system_root(),
    }
}

/// `env` as seen from a scan of `scan_root`: when the scan root, or a folder above it, is a second path
/// to the home or to a personal folder (a bind mount, a btrfs top level mounted elsewhere), that path
/// stands for it, so rules about the home and the personal folders apply inside the scan as well.
/// Folders are compared by identity (device and inode); `env` is returned unchanged when nothing
/// matches or nothing can be read.
pub fn environment_for_scan(env: &Environment, scan_root: &Path) -> Environment {
    let mut env = env.clone();
    let Some(home) = env.home.clone() else {
        return env;
    };

    let personal: Vec<(UserDir, PathBuf)> = UserDir::ALL
        .iter()
        .filter_map(|dir| env.user_dir(*dir).map(|path| (*dir, path)))
        .collect();
    let home_identity = identity(&home);

    for folder in scan_root.ancestors() {
        let Some(here) = identity(folder) else {
            continue;
        };

        if Some(here) == home_identity && folder != home {
            // The whole home is reachable here: personal folders inside it move along.
            env.user_dirs = personal
                .iter()
                .map(|(dir, path)| {
                    let moved = path
                        .strip_prefix(&home)
                        .map_or_else(|_| path.clone(), |inside| folder.join(inside));
                    (*dir, moved)
                })
                .collect();
            env.home = Some(folder.to_path_buf());
            return env;
        }

        if let Some((dir, _)) = personal
            .iter()
            .find(|(_, path)| path != folder && identity(path) == Some(here))
        {
            env.user_dirs.retain(|(kind, _)| kind != dir);
            env.user_dirs.push((*dir, folder.to_path_buf()));
            return env;
        }
    }

    env
}

#[cfg(unix)]
fn identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;

    std::fs::metadata(path).ok().map(|meta| (meta.dev(), meta.ino()))
}

/// The volume serial and file index the reader reads, of the folder `path` leads to.
#[cfg(windows)]
fn identity(path: &Path) -> Option<(u64, u64)> {
    let identity = crate::windows::reader::identity_following(path).ok()?;

    Some((identity.volume_serial, identity.file_index))
}

#[cfg(not(any(unix, windows)))]
fn identity(_path: &Path) -> Option<(u64, u64)> {
    None
}

/// The user's trash folder: `~/.Trash` on macOS, otherwise freedesktop (`$XDG_DATA_HOME/Trash`, by
/// default `~/.local/share/Trash`). `None` without a home directory, and always `None` on Windows (the
/// Recycle Bin is the shell's; the remover ignores `trash_dir` there).
pub fn home_trash(home: Option<&Path>) -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    if cfg!(target_os = "macos") {
        return home.map(|home| home.join(".Trash"));
    }

    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home.join(".local/share")))?;

    Some(data.join("Trash"))
}

/// [`home_trash`] of the home directory this process runs for, the way [`current_environment`] finds it.
#[cfg(target_os = "macos")]
pub(crate) fn current_home_trash() -> Option<PathBuf> {
    home_trash(home_directory().as_deref())
}

fn home_directory() -> Option<PathBuf> {
    let home = std::env::home_dir().filter(|path| path.is_absolute())?;
    let home = canonicalize(&home).unwrap_or(home);

    // A home of `/` (some services) would make every rule about the home folder cover the whole disk.
    (home.parent().is_some()).then_some(home)
}

/// `std::fs::canonicalize`; on Windows without the `\\?\` prefix (`dunce`), because every path of the app is
/// of the form `C:\a\b`.
fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    #[cfg(windows)]
    {
        dunce::canonicalize(path)
    }
    #[cfg(not(windows))]
    {
        std::fs::canonicalize(path)
    }
}

/// The personal folders of the user on Windows, as the shell reports them (a folder moved to another drive
/// or into OneDrive is found there), canonical when they exist.
#[cfg(windows)]
fn user_directories(home: &Path) -> Vec<(UserDir, PathBuf)> {
    use known_folders::{KnownFolder, get_known_folder_path};

    UserDir::ALL
        .iter()
        .filter_map(|&dir| {
            let known = match dir {
                UserDir::Desktop => KnownFolder::Desktop,
                UserDir::Documents => KnownFolder::Documents,
                UserDir::Music => KnownFolder::Music,
                UserDir::Pictures => KnownFolder::Pictures,
                UserDir::Videos => KnownFolder::Videos,
            };
            let path = get_known_folder_path(known)?;

            Some((dir, canonicalize(&path).unwrap_or(path)))
        })
        .filter(|(_, path)| is_a_personal_folder(path, home))
        .collect()
}

/// The personal folders on macOS: fixed names below the home folder, canonical when they exist. There
/// is no `user-dirs.dirs` here, and a folder that is missing is left out (the default name is assumed).
#[cfg(target_os = "macos")]
fn user_directories(home: &Path) -> Vec<(UserDir, PathBuf)> {
    UserDir::ALL
        .iter()
        .filter_map(|&dir| {
            let path = home.join(dir.default_name(Os::MacOs));
            let path = std::fs::canonicalize(&path).ok()?;

            Some((dir, path))
        })
        .filter(|(_, path)| is_a_personal_folder(path, home))
        .collect()
}

/// The personal folders the desktop configured in `user-dirs.dirs` (XDG), canonical when they exist.
#[cfg(not(any(windows, target_os = "macos")))]
fn user_directories(home: &Path) -> Vec<(UserDir, PathBuf)> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));

    // Read as text even when it is not valid UTF-8 (or the cap cut a letter): a broken byte must not
    // cost the user the protection of every personal folder.
    let Ok(bytes) = crate::read_regular_file(&config.join("user-dirs.dirs"), USER_DIRS_MAX_BYTES) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes);

    parse_user_dirs(&text, home)
        .into_iter()
        .map(|(dir, path)| (dir, std::fs::canonicalize(&path).unwrap_or(path)))
        .filter(|(_, path)| is_a_personal_folder(path, home))
        .collect()
}

/// A personal folder is a real folder of its own: `xdg-user-dirs` points a disabled one at the home
/// directory itself, and neither that nor `/` (a drive's root on Windows) may be treated as one.
fn is_a_personal_folder(path: &Path, home: &Path) -> bool {
    path.is_absolute() && path.parent().is_some() && path != home && !home.starts_with(path)
}

/// Reads `XDG_<NAME>_DIR="$HOME/..."` lines. A value is double-quoted, and either absolute or below
/// `$HOME`; everything else is ignored, as the XDG tools do.
#[cfg(not(any(windows, target_os = "macos")))]
fn parse_user_dirs(text: &str, home: &Path) -> Vec<(UserDir, PathBuf)> {
    let mut found: Vec<(UserDir, PathBuf)> = Vec::new();

    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let dir = match key.trim() {
            "XDG_DESKTOP_DIR" => UserDir::Desktop,
            "XDG_DOCUMENTS_DIR" => UserDir::Documents,
            "XDG_MUSIC_DIR" => UserDir::Music,
            "XDG_PICTURES_DIR" => UserDir::Pictures,
            "XDG_VIDEOS_DIR" => UserDir::Videos,
            _ => continue,
        };
        let Some(value) = value
            .trim()
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .map(unescape)
        else {
            continue;
        };

        let path = if let Some(rest) = value.strip_prefix("$HOME/") {
            home.join(rest)
        } else if value.starts_with('/') {
            PathBuf::from(value)
        } else {
            continue;
        };
        if !is_a_personal_folder(&path, home) {
            continue;
        }

        // A later line replaces an earlier one, as in a shell.
        found.retain(|(kind, _)| *kind != dir);
        found.push((dir, path));
    }

    found
}

/// Removes the backslash of a shell escape inside double quotes.
#[cfg(not(any(windows, target_os = "macos")))]
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();

    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }

    out
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

/// The Windows folder (`C:\Windows`) as the shell reports it; `None` on every other OS.
fn system_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        known_folders::get_known_folder_path(known_folders::KnownFolder::Windows)
            .filter(|path| path.is_absolute())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The operating system this binary was built for.
pub const fn current_os() -> Os {
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

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn localized_personal_folders_are_read_from_the_xdg_file() {
        let home = Path::new("/home/u");
        let text = "# comment\n\
            XDG_DESKTOP_DIR=\"$HOME/Pulpit\"\n\
            XDG_DOCUMENTS_DIR=\"$HOME/Dokumenty\"\n\
            XDG_DOWNLOAD_DIR=\"$HOME/Pobrane\"\n\
            XDG_PICTURES_DIR=\"/data/Obrazy\"\n\
            XDG_MUSIC_DIR=\"$HOME/Moja \\\"muzyka\\\"\"\n";

        assert_eq!(
            parse_user_dirs(text, home),
            [
                (UserDir::Desktop, PathBuf::from("/home/u/Pulpit")),
                (UserDir::Documents, PathBuf::from("/home/u/Dokumenty")),
                (UserDir::Pictures, PathBuf::from("/data/Obrazy")),
                (UserDir::Music, PathBuf::from("/home/u/Moja \"muzyka\"")),
            ]
        );
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn a_personal_folder_is_never_the_home_folder_or_something_above_it() {
        let home = Path::new("/home/u");
        let text = "XDG_DESKTOP_DIR=\"$HOME/\"\n\
            XDG_DOCUMENTS_DIR=\"/home/u\"\n\
            XDG_MUSIC_DIR=\"/\"\n\
            XDG_PICTURES_DIR=\"/home\"\n\
            XDG_VIDEOS_DIR=\"relative/Videos\"\n\
            XDG_VIDEOS_DIR=unquoted\n";

        assert_eq!(parse_user_dirs(text, home), []);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn a_later_line_replaces_an_earlier_one() {
        let home = Path::new("/home/u");
        let text = "XDG_VIDEOS_DIR=\"$HOME/A\"\nXDG_VIDEOS_DIR=\"$HOME/B\"\n";

        assert_eq!(
            parse_user_dirs(text, home),
            [(UserDir::Videos, PathBuf::from("/home/u/B"))]
        );
    }

    #[test]
    fn the_home_directory_is_absolute_when_known() {
        if let Some(home) = current_environment().home {
            assert!(home.is_absolute(), "{home:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_folder_and_the_home_are_plain_drive_paths() {
        let env = current_environment();

        let system_root = env.system_root.expect("the Windows folder is known");
        assert!(system_root.is_dir(), "{system_root:?}");
        assert!(
            !system_root.to_string_lossy().starts_with(r"\\?\"),
            "{system_root:?}"
        );
        if let Some(root) = std::env::var_os("SystemRoot") {
            assert!(
                system_root.as_os_str().eq_ignore_ascii_case(&root),
                "{system_root:?}"
            );
        }

        let home = env.home.expect("the profile folder is known");
        assert!(home.is_dir(), "{home:?}");
        assert!(!home.to_string_lossy().starts_with(r"\\?\"), "{home:?}");
        assert!(matches!(
            home.components().next(),
            Some(std::path::Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn the_personal_folders_come_from_the_shell_and_are_never_the_home_itself() {
        let env = current_environment();
        let home = env.home.clone().expect("the profile folder is known");

        assert!(!env.user_dirs.is_empty(), "a user profile has personal folders");
        for (dir, path) in &env.user_dirs {
            assert!(path.is_absolute(), "{dir:?}: {path:?}");
            assert!(!path.to_string_lossy().starts_with(r"\\?\"), "{dir:?}: {path:?}");
            assert_ne!(path, &home, "{dir:?}");
            assert_eq!(env.user_dir(*dir).as_ref(), Some(path));
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_same_folder_reached_another_way_has_the_same_identity() {
        let dir = tempfile::tempdir().unwrap();

        let once = identity(dir.path());
        let again = identity(&dir.path().join("."));

        assert!(once.is_some());
        assert_eq!(once, again);
        assert_ne!(once, identity(&dir.path().join("missing")));
    }

    #[cfg(not(windows))]
    #[test]
    fn there_is_no_windows_folder_elsewhere() {
        assert_eq!(current_environment().system_root, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_trash_of_a_mac_user_is_the_dot_trash_in_the_home() {
        assert_eq!(
            home_trash(Some(Path::new("/Users/u"))),
            Some(PathBuf::from("/Users/u/.Trash"))
        );
        assert_eq!(home_trash(None), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_trash_of_a_linux_user_is_the_freedesktop_one() {
        if std::env::var_os("XDG_DATA_HOME").is_none() {
            assert_eq!(
                home_trash(Some(Path::new("/home/u"))),
                Some(PathBuf::from("/home/u/.local/share/Trash"))
            );
        }
    }
}
