//! Looking at a path as it is now, before acting on it (see `platter_core::EntryProbe`).
//!
//! Each component is opened with `O_PATH | O_NOFOLLOW` below the descriptor of the one before, starting
//! from `/`, and described with a `statx` of that descriptor. Nothing is resolved by path string, so a
//! symlink anywhere on the way is seen, never followed: a component that is not a directory, other than
//! the last, fails. `O_PATH` reads no data and needs no read permission on the entry itself.

use std::ffi::CString;
use std::io;
use std::os::fd::OwnedFd;

use platter_core::{EntryKind, EntryProbe, ProbedEntry};
use rustix::fs::{CWD, Mode, OFlags, openat};
use rustix::io::Errno;

use super::stat::{is_boundary, stat_fd};

const PATH_FLAGS: OFlags = OFlags::PATH.union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC);

/// [`EntryProbe`] on Linux descriptors.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxEntryProbe;

impl EntryProbe for LinuxEntryProbe {
    fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
        let parts = components(path)?;

        let mut dir: OwnedFd = openat(CWD, c"/", PATH_FLAGS.union(OFlags::DIRECTORY), Mode::empty())?;
        let mut parent_dev = stat_fd(&dir)?.meta.dev;
        let mut found = Vec::with_capacity(parts.len());

        for (position, part) in parts.iter().enumerate() {
            let last = position + 1 == parts.len();
            let name = CString::new(*part).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            let flags = if last {
                PATH_FLAGS
            } else {
                PATH_FLAGS.union(OFlags::DIRECTORY)
            };

            let fd = openat(&dir, &name, flags, Mode::empty()).map_err(|errno| match errno {
                // A symlink (`O_NOFOLLOW` with `O_DIRECTORY`) or a file on the way.
                Errno::LOOP | Errno::NOTDIR => io::Error::from(io::ErrorKind::NotADirectory),
                other => io::Error::from(other),
            })?;
            let stat = stat_fd(&fd)?;

            if !last && stat.meta.kind != EntryKind::Dir {
                return Err(io::Error::from(io::ErrorKind::NotADirectory));
            }

            let mut meta = stat.meta;
            meta.is_mount_root = is_boundary(stat.reported_mount_root, meta.dev, parent_dev);
            found.push(ProbedEntry {
                meta,
                mount_id: stat.mount_id,
            });

            parent_dev = meta.dev;
            dir = fd;
        }

        Ok(found)
    }

    fn resolve(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        use std::os::unix::ffi::OsStrExt;

        let resolved = std::fs::canonicalize(std::ffi::OsStr::from_bytes(path))?;
        Ok(resolved.into_os_string().into_encoded_bytes())
    }
}

/// How long one look at a path may take before it is given up.
pub const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// [`LinuxEntryProbe`] with a time limit on every look. A path on an automount or on a network share
/// that does not answer can hold a lookup in the kernel indefinitely; then the answer is `TimedOut`
/// and the caller treats the path as unknown. The thread doing the lookup is left to finish (or hang)
/// on its own: a stuck system call cannot be interrupted, only stopped waiting for.
#[derive(Debug, Clone, Copy)]
pub struct TimedEntryProbe {
    pub timeout: std::time::Duration,
}

impl Default for TimedEntryProbe {
    fn default() -> Self {
        Self {
            timeout: PROBE_TIMEOUT,
        }
    }
}

impl TimedEntryProbe {
    fn timed<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> io::Result<T> + Send + 'static,
    ) -> io::Result<T> {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("platter-probe".to_owned())
            .spawn(move || {
                let _ = send.send(work());
            })?;

        receive.recv_timeout(self.timeout).unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the path did not answer in time (a network share or automount?)",
            ))
        })
    }
}

impl EntryProbe for TimedEntryProbe {
    fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
        let path = path.to_vec();
        self.timed(move || LinuxEntryProbe.probe(&path))
    }

    fn resolve(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        let path = path.to_vec();
        self.timed(move || LinuxEntryProbe.resolve(&path))
    }
}

/// The names of an absolute path; `.`, `..` and empty components (`//`) are refused, a trailing `/` is
/// allowed.
fn components(path: &[u8]) -> io::Result<Vec<&[u8]>> {
    let Some(rest) = path.strip_prefix(b"/") else {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    };
    let rest = rest.strip_suffix(b"/").unwrap_or(rest);
    if rest.is_empty() {
        return Ok(Vec::new());
    }

    let parts: Vec<&[u8]> = rest.split(|&byte| byte == b'/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == b"." || *part == b"..")
    {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    fn bytes(path: &std::path::Path) -> Vec<u8> {
        path.as_os_str().as_encoded_bytes().to_vec()
    }

    #[test]
    fn describes_every_component_down_to_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/f"), b"x").unwrap();

        let found = LinuxEntryProbe.probe(&bytes(&root.join("a/b/f"))).unwrap();

        assert_eq!(found.len(), root.components().count() - 1 + 3);
        let kinds: Vec<_> = found.iter().rev().take(3).map(|e| e.meta.kind).collect();
        assert_eq!(kinds, [EntryKind::File, EntryKind::Dir, EntryKind::Dir]);
        let meta = std::fs::symlink_metadata(root.join("a/b/f")).unwrap();
        use std::os::unix::fs::MetadataExt;
        assert_eq!(found.last().unwrap().meta.ino, meta.ino());
    }

    #[test]
    fn a_symlink_at_the_end_is_described_and_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("real")).unwrap();
        symlink(root.join("real"), root.join("link")).unwrap();

        let found = LinuxEntryProbe.probe(&bytes(&root.join("link"))).unwrap();

        assert_eq!(found.last().unwrap().meta.kind, EntryKind::Symlink);
    }

    #[test]
    fn a_symlink_on_the_way_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("real/inner")).unwrap();
        symlink(root.join("real"), root.join("link")).unwrap();

        let error = LinuxEntryProbe
            .probe(&bytes(&root.join("link/inner")))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotADirectory);
    }

    #[test]
    fn the_timed_probe_answers_like_the_plain_one_and_gives_up_on_a_slow_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();

        // Identity and kind only: the times of `/tmp` change while other tests run.
        let shape = |found: Vec<ProbedEntry>| -> Vec<(u64, u64, EntryKind)> {
            found
                .iter()
                .map(|entry| (entry.meta.dev, entry.meta.ino, entry.meta.kind))
                .collect()
        };
        assert_eq!(
            shape(TimedEntryProbe::default().probe(&bytes(&root)).unwrap()),
            shape(LinuxEntryProbe.probe(&bytes(&root)).unwrap())
        );

        let impatient = TimedEntryProbe {
            timeout: std::time::Duration::from_millis(10),
        };
        let error = impatient
            .timed(|| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn missing_and_odd_paths_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();

        let missing = LinuxEntryProbe.probe(&bytes(&root.join("nope"))).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);

        for odd in [&b"relative"[..], b"/a/../b", b"/a//b", b"/a/./b"] {
            let error = LinuxEntryProbe.probe(odd).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{odd:?}");
        }
        assert!(LinuxEntryProbe.probe(b"/").unwrap().is_empty());
    }
}
