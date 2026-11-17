//! Resolving the folder a scan starts from.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A scan root after symlinks were resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoot {
    /// The canonical absolute path.
    pub path: PathBuf,
    /// `(dev, ino)` of the directory at the time of resolving, to pass as `ScanConfig::root_identity`
    /// so the scan refuses to start when the path was swapped for another folder meanwhile. `None`
    /// where the OS reports no identity.
    pub identity: Option<(u64, u64)>,
}

/// Resolves `path` to its canonical form and remembers which directory that was.
///
/// Fails with `NotFound` when it does not exist and `NotADirectory` when it is something else.
///
/// Resolving and opening are two steps, so there is a window in which a component of the path can be
/// replaced. The identity check closes it for the scan root itself: the scan compares the directory it
/// opened with the one seen here and refuses on a mismatch (it also catches a swapped parent
/// directory, which `O_NOFOLLOW` on the last component would not). What remains is a swap that puts
/// the *same* directory (same device and inode) behind another path, which is harmless, and the
/// recorded path text: after the scan, a swapped path makes "show in file manager" point at what is
/// there now. Below the root nothing is path-based on Linux (all opens are relative to descriptors).
///
/// On Windows the path is made with `dunce`, so it is `C:\a\b` and never `\\?\C:\a\b`, and the identity is
/// the volume serial and the file index the reader reads.
pub fn resolve_root(path: &Path) -> io::Result<ResolvedRoot> {
    let canonical = canonicalize(path)?;

    let metadata = fs::metadata(&canonical)?;
    if !metadata.is_dir() {
        return Err(io::Error::from(io::ErrorKind::NotADirectory));
    }

    let identity = identity_of_directory(&canonical, &metadata);

    Ok(ResolvedRoot {
        path: canonical,
        identity: identity.filter(|&identity| identity != (0, 0)),
    })
}

#[cfg(not(windows))]
fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path)
}

#[cfg(windows)]
fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    dunce::canonicalize(path)
}

#[cfg(not(windows))]
fn identity_of_directory(_path: &Path, metadata: &fs::Metadata) -> Option<(u64, u64)> {
    let meta = crate::generic::meta::entry_meta(metadata);

    Some((meta.dev, meta.ino))
}

#[cfg(windows)]
fn identity_of_directory(path: &Path, _metadata: &fs::Metadata) -> Option<(u64, u64)> {
    let identity = crate::windows::reader::identity_following(path).ok()?;

    Some((identity.volume_serial, identity.file_index))
}
