//! The lock on the models folder: one process at a time downloads the model, or checks it and deletes
//! it when it is wrong. Two Platter windows on the same data folder would otherwise write one partial
//! file together, or delete a model the other one has just finished.
//!
//! It is an advisory `flock` on `<folder>/.lock` (0600), taken without waiting. The lock belongs to the
//! open file, so it is released when the guard is dropped, and by the system when the process ends.

use std::fs::File;
use std::io;
use std::path::Path;

/// The name of the lock file in the models folder.
pub const LOCK_FILE: &str = ".lock";

/// The models folder is locked by this value while it lives.
#[derive(Debug)]
pub struct ModelLock {
    _file: File,
}

impl ModelLock {
    /// Takes the lock on the folder `dir`, which must exist. `Ok(None)` when another process, or another
    /// guard of this one, holds it.
    pub fn try_acquire(dir: &Path) -> io::Result<Option<Self>> {
        let file = platter_platform::open_private_file(&dir.join(LOCK_FILE))?;

        if platter_platform::lock_exclusive(&file)? {
            Ok(Some(Self { _file: file }))
        } else {
            Ok(None)
        }
    }
}
