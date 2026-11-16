//! Test helpers that need OS-specific code, so the other crates' tests do not have to carry any
//! (`cfg(target_os)` code lives in this crate only). Enabled by the `testing` feature.

use std::io;
use std::path::{Path, PathBuf};

/// Creates a symbolic link at `link` that points to the directory `target`.
pub fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
    }
}

/// A file name that is not valid Unicode, joined to `parent`: raw bytes on Unix, an unpaired surrogate
/// on Windows.
pub fn non_utf8_path(parent: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        parent.join(OsString::from_vec(b"caf\xe9-\xff".to_vec()))
    }
    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        parent.join(OsString::from_wide(&[0x63, 0xD800, 0x66]))
    }
}
