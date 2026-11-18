//! The check a model file must pass before it is loaded, and the only way to get a [`VerifiedModel`] (a
//! helper process that was handed a file the parent checked takes it over with [`VerifiedModel::inherited`]).
//!
//! The file is opened once, without following a link. Size and SHA-256 are checked through that
//! descriptor, and the model is loaded from the same descriptor (`/proc/self/fd/N` on Linux, `/dev/fd/N` on
//! macOS), so the name cannot be pointed at another file between the check and the load. (Someone who can write to the file itself
//! as this user can still change it in place; that is outside what this check can promise.)
//!
//! On Windows there is no name that follows a handle. The file is opened with a share mode that lets nobody
//! write, rename or delete it while it is held (and its folder is held too), the bytes are hashed through that
//! handle, and the model is loaded by the file's own path with the handle still open: the path names the
//! hashed bytes for as long as the [`VerifiedModel`] lives. See `platter_platform::windows::pinned`.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use platter_platform::Pin;

use sha2::{Digest, Sha256};

use crate::spec::ModelSpec;

const READ_CHUNK: usize = 1 << 20;

/// A model file that has been checked against its [`ModelSpec`], and is held open.
#[derive(Debug)]
pub struct VerifiedModel {
    file: File,
    /// What keeps the path naming `file` (nothing on Linux and macOS, the held folder on Windows).
    pin: Pin,
}

/// What a helper process is given to load a model that the app checked.
#[derive(Debug)]
pub struct Handover {
    /// The checked file, to be the helper's standard output (Unix).
    pub file: Option<File>,
    /// The absolute path of the checked file, to be named in the `load` request (Windows).
    pub path: Option<String>,
}

impl VerifiedModel {
    /// A path that names exactly the file that was checked, as long as this value lives. An error where no
    /// such path exists: an empty path is never handed to a loader.
    pub fn path(&self) -> Result<PathBuf, VerifyError> {
        pinned_path_of(self.pin.path(&self.file))
    }

    /// What to give a helper process that loads the model: another handle on the checked file (the same open
    /// file, not a new look at its name) where a descriptor can be handed over, and the path of the file,
    /// which this value holds so that it names the checked bytes, where it cannot (Windows).
    pub fn handover(&self) -> io::Result<Handover> {
        #[cfg(unix)]
        {
            Ok(Handover {
                file: Some(self.file.try_clone()?),
                path: None,
            })
        }
        #[cfg(not(unix))]
        {
            let path = self
                .path()
                .map_err(|error| io::Error::new(io::ErrorKind::Unsupported, error.to_string()))?;
            let text = path.to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "the model's path is not Unicode text")
            })?;

            Ok(Handover {
                file: None,
                path: Some(text.to_owned()),
            })
        }
    }

    /// The model file the helper process was given by the process that checked it. That process hashed this
    /// very open file before it handed it over, so only what it cannot know is looked at here: a regular
    /// file of exactly `bytes`. Never used for a file that was opened by name.
    #[cfg(unix)]
    pub(crate) fn inherited(file: File, bytes: u64) -> Result<Self, VerifyError> {
        let pin = Pin::inherited();
        Self::of_known_size(file, pin, bytes)
    }

    /// The model file the helper process opens by the path in its `load` request. The app that sent it holds
    /// the file open with a share mode that lets nobody change it (see the module's text), and hashed it
    /// through that handle, so this opens that very file for as long as the app holds it. What the helper cannot
    /// know is looked at here: the name is not a link, a folder or a file open for writing somewhere, and the
    /// file has exactly `bytes`. The helper's own handle keeps the same share mode, so the file stays as it is
    /// for as long as this value lives.
    #[cfg(windows)]
    pub(crate) fn opened_by_path(path: &Path, bytes: u64) -> Result<Self, VerifyError> {
        if !path.is_absolute() {
            return Err(VerifyError::NotRegular);
        }
        let (file, pin) = platter_platform::open_pinned(path).map_err(open_error)?;

        Self::of_known_size(file, pin, bytes)
    }

    fn of_known_size(file: File, pin: Pin, bytes: u64) -> Result<Self, VerifyError> {
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(VerifyError::NotRegular);
        }
        if meta.len() != bytes {
            return Err(VerifyError::WrongSize {
                expected: bytes,
                found: meta.len(),
            });
        }
        pinned_path_of(pin.path(&file))?;

        Ok(Self { file, pin })
    }
}

/// Why a file is not the model.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("The model file is missing.")]
    Missing,
    #[error("The model file is a link or not a regular file.")]
    NotRegular,
    #[error("The model file has the wrong size ({found} bytes instead of {expected}).")]
    WrongSize { expected: u64, found: u64 },
    #[error("The model file does not match its checksum.")]
    WrongHash,
    #[error("The model file cannot be checked on this system.")]
    Unsupported,
    #[error("The model file cannot be read: {0}")]
    Io(#[from] io::Error),
}

impl VerifyError {
    /// The file is there but is not the model: it should be deleted rather than tried again.
    pub fn is_corrupt(&self) -> bool {
        matches!(self, Self::NotRegular | Self::WrongSize { .. } | Self::WrongHash)
    }
}

/// Opens `path` without following a link and checks it is a regular file of the model's size with the
/// model's SHA-256.
pub fn verify(spec: &ModelSpec, path: &Path) -> Result<VerifiedModel, VerifyError> {
    let (mut file, pin) = platter_platform::open_pinned(path).map_err(open_error)?;

    let found = file.metadata()?.len();
    if found != spec.bytes {
        return Err(VerifyError::WrongSize {
            expected: spec.bytes,
            found,
        });
    }

    let never = AtomicBool::new(false);
    let digest = sha256_hex(&mut file, &never)?;
    if digest != spec.sha256 {
        return Err(VerifyError::WrongHash);
    }

    pinned_path_of(pin.path(&file))?;
    file.seek(SeekFrom::Start(0))?;
    Ok(VerifiedModel { file, pin })
}

/// The pinned path of a file, which a [`Pin`] may not have (a system where nothing can be pinned): then the
/// file cannot be loaded safely and that is an error, not an empty path.
fn pinned_path_of(path: Option<PathBuf>) -> Result<PathBuf, VerifyError> {
    path.ok_or(VerifyError::Unsupported)
}

fn open_error(error: io::Error) -> VerifyError {
    match error.kind() {
        io::ErrorKind::NotFound => VerifyError::Missing,
        io::ErrorKind::InvalidInput => VerifyError::NotRegular,
        io::ErrorKind::Unsupported => VerifyError::Unsupported,
        _ => VerifyError::Io(error),
    }
}

/// SHA-256 of everything `reader` yields, as 64 lower-case hexadecimal digits. Stops with `Interrupted`
/// when `cancel` is raised.
pub(crate) fn sha256_hex(reader: &mut impl Read, cancel: &AtomicBool) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_CHUNK];

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(hex(&hasher.finalize()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_with_no_pinned_path_is_an_error_not_an_empty_path() {
        assert!(matches!(pinned_path_of(None), Err(VerifyError::Unsupported)));
        assert_eq!(
            pinned_path_of(Some(PathBuf::from("/dev/fd/5"))).unwrap(),
            PathBuf::from("/dev/fd/5")
        );
    }
}
