//! Opening a file so that what was checked is what is used.
//!
//! A file is opened once, without following a link; every later look and the hand-over to another
//! library go through that descriptor. `/proc/self/fd/N` names the very file the descriptor holds, so a
//! library that wants a path can be given one that cannot be pointed somewhere else afterwards.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rustix::fs::OFlags;
use rustix::io::Errno;

/// Opens `path` for reading, refusing a link at the path (`O_NOFOLLOW`) and anything that is not a
/// regular file (a FIFO is refused without waiting for a writer). Only the last name is checked.
pub fn open_nofollow(path: &Path) -> io::Result<File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(code) if code == Errno::LOOP.raw_os_error() => io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is a link and Platter does not follow it", path.display()),
            ),
            _ => error,
        })?;

    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    Ok(file)
}

/// A path that names the file `file` holds open, for as long as it stays open (`/proc/self/fd/N`).
pub fn pinned_path(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_path_reads_the_same_file_even_after_the_name_is_swapped() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("model");
        std::fs::write(&path, b"original").unwrap();

        let file = open_nofollow(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"swapped").unwrap();

        assert_eq!(std::fs::read(pinned_path(&file)).unwrap(), b"original");
    }

    #[test]
    fn a_link_a_folder_and_a_missing_file_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::write(&target, b"x").unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert_eq!(
            open_nofollow(&link).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            open_nofollow(root.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            open_nofollow(&root.path().join("missing")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(open_nofollow(&target).is_ok());
    }
}
