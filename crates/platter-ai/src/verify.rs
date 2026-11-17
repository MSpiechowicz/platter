//! The check a model file must pass before it is loaded, and the only way to get a [`VerifiedModel`] (a
//! helper process that was handed a file the parent checked takes it over with [`VerifiedModel::inherited`]).
//!
//! The file is opened once, without following a link. Size and SHA-256 are checked through that
//! descriptor, and the model is loaded from the same descriptor (`/proc/self/fd/N`), so the name cannot
//! be pointed at another file between the check and the load. (Someone who can write to the file itself
//! as this user can still change it in place; that is outside what this check can promise.)

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::spec::ModelSpec;

const READ_CHUNK: usize = 1 << 20;

/// A model file that has been checked against its [`ModelSpec`], and is held open.
#[derive(Debug)]
pub struct VerifiedModel {
    file: File,
}

impl VerifiedModel {
    /// A path that names exactly the file that was checked, as long as this value lives.
    pub fn path(&self) -> PathBuf {
        platter_platform::pinned_path(&self.file).unwrap_or_default()
    }

    /// Another handle on the checked file (the same open file, not a new look at its name), to hand to a
    /// helper process that loads the model.
    pub fn share(&self) -> io::Result<File> {
        self.file.try_clone()
    }

    /// The model file the helper process was given by the process that checked it. That process hashed this
    /// very open file before it handed it over, so only what it cannot know is looked at here: a regular
    /// file of exactly `bytes`. Never used for a file that was opened by name.
    pub(crate) fn inherited(file: File, bytes: u64) -> Result<Self, VerifyError> {
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
        if platter_platform::pinned_path(&file).is_none() {
            return Err(VerifyError::Unsupported);
        }

        Ok(Self { file })
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
    let mut file = platter_platform::open_nofollow(path).map_err(open_error)?;

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

    if platter_platform::pinned_path(&file).is_none() {
        return Err(VerifyError::Unsupported);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(VerifiedModel { file })
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
