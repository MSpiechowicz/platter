//! The user's rules file: `$XDG_CONFIG_HOME/platter/rules.json` (see docs/architecture.md, "Files on disk").
//!
//! The format and its limits are checked by `platter-core`; this module only moves the bytes. Reading is
//! cautious (a regular file, never a followed link, at most 256 KiB) and a file that fails any check is
//! reported as broken and left exactly as it is: it is never overwritten, only moved aside on request. Writing
//! goes through a temporary file in the same folder (mode 0600, folder 0700) that is synced, renamed into
//! place, and the folder synced.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use platter_core::{MAX_RULES_FILE_BYTES, UserRulesFile, parse_user_rules, render_user_rules};

use crate::app_dirs::{config_dir, create_private_dir};

const FILE_NAME: &str = "rules.json";
const BROKEN_SUFFIX: &str = ".broken";

/// What the rules file holds right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    /// The rules; a file that does not exist is an empty list.
    Rules(UserRulesFile),
    /// The file is there but cannot be trusted. The text says why, for people to read.
    Broken(String),
}

/// Where the rules file is (`$XDG_CONFIG_HOME/platter/rules.json`), not created. `None` without a home.
pub fn rules_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(FILE_NAME))
}

/// Reads the rules file at `path`. Nothing is changed, whatever the file holds.
pub fn load(path: &Path) -> Loaded {
    let bytes = match read_capped(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Loaded::Rules(UserRulesFile::default()),
        Err(error) => return Loaded::Broken(format!("The rules file cannot be read: {error}")),
    };

    match parse_user_rules(&bytes) {
        Ok(file) => Loaded::Rules(file),
        Err(error) => Loaded::Broken(error.to_string()),
    }
}

/// The bytes of the regular file at `path`, `None` when there is no such file. A link at the name, a
/// folder, a FIFO or a file over the size cap is an error.
fn read_capped(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = match open_regular(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    // One byte more than the cap, so a file that is too large is noticed and not cut.
    let mut bytes = Vec::new();
    file.take(MAX_RULES_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RULES_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("it is larger than {} KiB", MAX_RULES_FILE_BYTES / 1024),
        ));
    }

    Ok(Some(bytes))
}

#[cfg(target_os = "linux")]
fn open_regular(path: &Path) -> io::Result<File> {
    crate::open_nofollow(path)
}

#[cfg(not(target_os = "linux"))]
fn open_regular(path: &Path) -> io::Result<File> {
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "it is a link"));
    }

    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "it is not a regular file",
        ));
    }
    Ok(file)
}

/// Writes `file` to `path` atomically: a temporary file next to it (mode 0600, in a 0700 folder that is
/// created when missing), synced, renamed over `path`, and the folder synced. Refuses, and changes
/// nothing, when what is at `path` is broken: such a file is never overwritten. Also refuses a file that
/// renders larger than 256 KiB, because the next read would call it broken.
///
/// "Not broken" is looked at before anything is created and again right before the rename, which is as
/// late as a rename over a name allows. Between that last look and the rename another process can still
/// replace the file (a lost update of its change) or break it (the broken bytes are then replaced). The
/// app is protected against the consequences, not the race: it reads the file on every use, so a change
/// it did not make moves the generation its plans are made from, and those plans are refused.
pub fn save(path: &Path, file: &UserRulesFile) -> io::Result<()> {
    let bytes = render_user_rules(file);
    if bytes.len() > MAX_RULES_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "the rules would be larger than {} KiB, which Platter does not read",
                MAX_RULES_FILE_BYTES / 1024
            ),
        ));
    }
    refuse_broken(path)?;

    let dir = parent_of(path)?;
    create_private_dir(dir)?;

    let temporary = temporary_name(path);
    let written = write_synced(&temporary, &bytes)
        .and_then(|()| refuse_broken(path))
        .and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }

    sync_dir(dir)
}

fn refuse_broken(path: &Path) -> io::Result<()> {
    match load(path) {
        Loaded::Broken(why) => Err(io::Error::new(io::ErrorKind::InvalidData, why)),
        Loaded::Rules(_) => Ok(()),
    }
}

/// Moves a broken rules file to `rules.json.broken` next to it (`.broken.1`, `.broken.2`, ... when that
/// name is taken, so an earlier one is never lost) and returns where it went. A missing file is an error.
/// The name is taken by the rename itself (it fails when the name exists), so an earlier file cannot be
/// replaced even by one that appears at the same moment.
pub fn move_aside(path: &Path) -> io::Result<PathBuf> {
    let dir = parent_of(path)?;
    // The file itself is never opened, so a link or a folder at the name is moved as it is.
    std::fs::symlink_metadata(path)?;

    for attempt in 0..MAX_BROKEN_FILES {
        let target = broken_name(path, attempt);

        match rename_without_replacing(path, &target) {
            Ok(()) => {
                sync_dir(dir)?;
                return Ok(target);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::other("too many old rules files"))
}

/// How many `.broken` files are kept before moving another aside is refused.
const MAX_BROKEN_FILES: u32 = 1000;

/// Renames `from` to `to`, failing with `AlreadyExists` when `to` is there (`renameat2` with
/// `RENAME_NOREPLACE`). On a filesystem that does not know the flag the name is checked first, which can
/// lose against a file created in between.
#[cfg(target_os = "linux")]
fn rename_without_replacing(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    use rustix::io::Errno;

    match renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(Errno::INVAL | Errno::NOSYS | Errno::OPNOTSUPP) => rename_after_checking(from, to),
        Err(error) => Err(io::Error::from(error)),
    }
}

#[cfg(not(target_os = "linux"))]
fn rename_without_replacing(from: &Path, to: &Path) -> io::Result<()> {
    rename_after_checking(from, to)
}

fn rename_after_checking(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(to) {
        Ok(_) => Err(io::Error::from(io::ErrorKind::AlreadyExists)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => std::fs::rename(from, to),
        Err(error) => Err(error),
    }
}

fn broken_name(path: &Path, attempt: u32) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(BROKEN_SUFFIX);
    if attempt > 0 {
        name.push(format!(".{attempt}"));
    }

    PathBuf::from(name)
}

fn parent_of(path: &Path) -> io::Result<&Path> {
    path.parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the rules file has no folder"))
}

/// A name for the temporary file that no other writer (this process or another window) uses.
fn temporary_name(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let mut name = path.as_os_str().to_owned();
    name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    PathBuf::from(name)
}

/// Creates `path` (it must not exist, and a link there is never followed), writes `bytes` and syncs.
fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Makes a rename or a new name in `dir` durable.
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(test)]
mod tests;
