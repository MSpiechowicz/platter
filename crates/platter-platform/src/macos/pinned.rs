//! Opening the model file so that what was checked is what is loaded, on macOS (package M3).
//!
//! A file is opened once, without following a link; every later look and the hand-over to another library go
//! through that descriptor. macOS has no `/proc/self/fd`, but `/dev/fd/N` does the work: opening it does not
//! look anything up, it duplicates descriptor `N` of the process (the descriptor's file, not whatever its old
//! name points to now). A library that wants a path and is given `/dev/fd/N` therefore loads the very file that
//! was checked, and the name it was found by can be swapped without effect. Like the Linux name it needs
//! nothing else held, so [`Pin`] holds nothing.
//!
//! The duplicate shares the original's file offset. A loader that reads from the start with absolute seeks
//! (llama.cpp does) is not disturbed by that, and the checked descriptor is rewound to 0 before it is handed
//! over.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rustix::fs::OFlags;
use rustix::io::Errno;

/// What keeps a model file's path naming the file that was checked, besides the open file itself. On macOS
/// nothing: `/dev/fd/N` follows the descriptor, so there is nothing more to hold.
#[derive(Debug)]
pub struct Pin(());

impl Pin {
    /// The pin of a file that was handed over already open (a descriptor of the helper).
    pub fn inherited() -> Self {
        Self(())
    }

    /// A path that names `file` (the file this pin was made with) for as long as it is open.
    pub fn path(&self, file: &File) -> Option<PathBuf> {
        pinned_path(file)
    }
}

/// Opens `path` for reading, refusing a link at the path (`O_NOFOLLOW`) and anything that is not a regular
/// file (a FIFO is refused without waiting for a writer: `O_NONBLOCK`). Only the last name is checked.
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

/// [`open_nofollow`], and what keeps its name pointing at it (nothing more on macOS).
pub fn open_pinned(path: &Path) -> io::Result<(File, Pin)> {
    open_nofollow(path).map(|file| (file, Pin(())))
}

/// A path that names the file `file` holds open, for as long as it stays open (`/dev/fd/N`).
pub fn pinned_path(file: &File) -> Option<PathBuf> {
    Some(PathBuf::from(format!("/dev/fd/{}", file.as_raw_fd())))
}

/// Whether a model can be held open and handed to a library by path here.
pub const fn model_runtime_supported() -> bool {
    true
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

        assert_eq!(std::fs::read(pinned_path(&file).unwrap()).unwrap(), b"original");
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

    #[test]
    fn a_pin_holds_the_open_file_and_names_it_by_descriptor() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("model");
        std::fs::write(&path, b"weights").unwrap();

        let (file, pin) = open_pinned(&path).unwrap();

        let named = pin.path(&file).unwrap();
        assert!(named.starts_with("/dev/fd/"));
        assert_eq!(std::fs::read(named).unwrap(), b"weights");
        assert!(model_runtime_supported());
    }
}
