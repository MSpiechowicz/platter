//! Appending lines to a private log file with a size cap, shared by the logs of the advisor (see [`super::log`]
//! and [`super::gpu_log`]).
//!
//! A file is only ever opened as a regular file and a link at its name is never followed. It is made with
//! mode 0600 in a folder of mode 0700 and closed again if someone opened it up. A line that would pass the cap
//! drops the oldest lines, and the file is then written again atomically (a temporary file, mode 0600, synced
//! and renamed over it, like the rules file).

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A file larger than this many times the cap is not read to be trimmed (something else made it): it is
/// started again.
const READ_LIMIT_IN_CAPS: u64 = 8;

/// Adds `line` (ending in a newline) to the file at `path`, making the file and its folder when they are
/// missing, and trimming the oldest lines when the file would pass `cap` bytes.
pub(super) fn append_line(path: &Path, line: &[u8], cap: usize) -> io::Result<()> {
    if line.len() > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the line is larger than the log",
        ));
    }
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the log has no folder"))?;
    platter_platform::create_private_dir(dir)?;

    // A link, a folder or any other file that is not regular is refused, before and when it is opened.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the log is not a regular file",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut file = platter_platform::open_private_file(path)?;
    close_to_others(&file)?;

    let size = file.metadata()?.len();
    if size.saturating_add(line.len() as u64) <= cap as u64 {
        return append_to(&mut file, size, line);
    }

    // Over the cap: the newest lines that fit, then the new one, written again as a whole.
    let old = if size <= READ_LIMIT_IN_CAPS * cap as u64 {
        let mut old = Vec::with_capacity(size as usize);
        file.read_to_end(&mut old)?;
        old
    } else {
        Vec::new()
    };
    drop(file);

    replace(path, dir, &trimmed(&old, line, cap))
}

/// The end of the file at `path`: its last `max` bytes, from the start of a line (so at most `max` bytes, and
/// only whole lines). Empty when there is no file. A link, a folder or any other file that is not regular is
/// refused.
#[cfg(any(feature = "advisor", test))]
pub(super) fn read_tail(path: &Path, max: usize) -> io::Result<Vec<u8>> {
    let mut file = match platter_platform::open_nofollow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let size = file.metadata()?.len();
    let from = size.saturating_sub(max as u64);
    file.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::with_capacity((size - from) as usize);
    file.take(max as u64).read_to_end(&mut bytes)?;

    if from > 0 {
        // Started in the middle of a line: on to the start of the next one.
        let start = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |at| at + 1);
        bytes.drain(..start);
    }
    Ok(bytes)
}

/// Writes `line` at the end of the file (which holds `size` bytes), after a newline when the last line was
/// cut short, so every line of the file stays a line of its own.
fn append_to(file: &mut File, size: u64, line: &[u8]) -> io::Result<()> {
    let mut piece = Vec::with_capacity(line.len() + 1);

    if size > 0 {
        let mut last = [0u8; 1];
        file.seek(SeekFrom::Start(size - 1))?;
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            piece.push(b'\n');
        }
    }
    piece.extend_from_slice(line);

    file.seek(SeekFrom::End(0))?;
    file.write_all(&piece)
}

/// What the file holds after `line` is added to `old` and the oldest lines are dropped until it is no
/// larger than `cap`. Starts at the beginning of a line.
pub(super) fn trimmed(old: &[u8], line: &[u8], cap: usize) -> Vec<u8> {
    let mut all = old.to_vec();
    if !all.is_empty() && all.last() != Some(&b'\n') {
        all.push(b'\n');
    }
    all.extend_from_slice(line);

    let excess = all.len().saturating_sub(cap);
    if excess == 0 {
        return all;
    }

    // Dropping `excess` bytes may cut a line: go on to the start of the next one.
    let from = all[excess - 1..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(all.len(), |at| excess + at);

    all.split_off(from)
}

/// Writes `bytes` to a temporary file next to `path` (mode 0600), syncs it, renames it over `path` and
/// syncs the folder.
fn replace(path: &Path, dir: &Path, bytes: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let mut name = path.as_os_str().to_owned();
    name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = PathBuf::from(name);

    let written = write_synced(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }

    sync_dir(dir)
}

/// Makes a rename in `dir` durable.
#[cfg(not(windows))]
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Windows cannot open a folder to sync it: the rename is as durable as the filesystem makes it, and the file
/// itself was synced before it.
#[cfg(windows)]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
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

/// Closes a log that was made open to group or others.
#[cfg(unix)]
fn close_to_others(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if file.metadata()?.permissions().mode() & 0o077 == 0 {
        return Ok(());
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn close_to_others(_file: &File) -> io::Result<()> {
    Ok(())
}

/// Deletes the log at `path`: nothing to do when there is none. A link, a folder or anything else that is not a
/// regular file at its name is left alone and reported (`io`).
pub(super) fn remove_regular(path: &Path, what: &str) -> crate::error::CommandResult<()> {
    use crate::error::CommandError;

    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CommandError::io(format!(
                "The {what} could not be deleted: {error}"
            ))),
        },
        Ok(_) => Err(CommandError::io(format!(
            "The {what} is not a regular file, so Platter left it alone."
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CommandError::io(format!(
            "The {what} could not be deleted: {error}"
        ))),
    }
}
