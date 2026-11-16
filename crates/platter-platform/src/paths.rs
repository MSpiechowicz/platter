//! Turning a node of a scanned tree back into a path the OS accepts.

use std::path::PathBuf;

use platter_core::{NodeId, Tree};

/// The exact filesystem path of `id`, even when a name on the way is not valid UTF-8.
///
/// On Unix this is lossless: names are the raw bytes the kernel returned and
/// [`Tree::path_bytes`] joins them unchanged.
///
/// On other platforms the stored bytes are the standard library's internal encoding, which cannot be
/// turned back into an `OsString` without `unsafe`. There this falls back to [`Tree::path_of`], which
/// replaces invalid UTF-8 with U+FFFD, so a path containing such a name may not resolve.
pub fn node_path(tree: &Tree, id: NodeId) -> PathBuf {
    #[cfg(unix)]
    {
        path_from_bytes(tree.path_bytes(id))
    }
    #[cfg(not(unix))]
    {
        tree.path_of(id)
    }
}

/// Unix path from raw bytes, with no conversion.
#[cfg(unix)]
pub fn path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    PathBuf::from(OsString::from_vec(bytes))
}
