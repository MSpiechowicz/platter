//! Looking at a path as it is now, before acting on it, on macOS (see `platter_core::EntryProbe`).
//!
//! Each folder on the way is opened with `O_NOFOLLOW | O_DIRECTORY` below the descriptor of the one before,
//! starting from `/`, and described with `fstat` of that descriptor. The last name is described with
//! `fstatat(AT_SYMLINK_NOFOLLOW)`, so a link there is seen and never followed (macOS has no `O_PATH`, and
//! nothing is opened that is not a folder). Nothing is resolved by path string: a link anywhere on the way
//! fails, a component that is not a folder other than the last fails.
//!
//! Identity is the device and inode, the link count and the creation time (`st_birthtime`). A component
//! starts a mount when its device differs from the one above it, except the step through a firmlink from
//! the system volume into the data volume (`/Users`, `/Applications`, ...), which is the same disk (see
//! [`crate::mac_pure::trash_place::starts_mount`]).
//!
//! Every look runs in its own thread and is given up after [`PROBE_TIMEOUT`]: a path on an automount or a
//! share that does not answer cannot hold cleanup. The thread is left to finish (or hang) on its own.

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::time::Duration;

use platter_core::{BirthTime, EntryKind, EntryMeta, EntryProbe, ProbedEntry};
use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, Stat as RawStat, fstat, openat, statat};
use rustix::io::Errno;

use crate::mac_pure::trash_place::{components as path_components, starts_mount};

/// How long one look at a path may take before it is given up.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Where the system volume's firmlinks lead.
const DATA_VOLUME: &CStr = c"/System/Volumes/Data";

/// How a folder is opened on the way: never a link, never anything but a folder.
pub(super) const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// What `fstat` told us about one entry.
#[derive(Debug, Clone, Copy)]
pub(super) struct Stat {
    pub meta: EntryMeta,
    pub birth: Option<BirthTime>,
    pub uid: u32,
    /// The permission bits (and setuid, setgid and sticky), without the file type.
    pub mode: u32,
}

impl Stat {
    pub fn probed(&self) -> ProbedEntry {
        ProbedEntry {
            meta: self.meta,
            mount_id: None,
            birth: self.birth,
        }
    }
}

/// Stats `name` below `dir` without following a link at the end.
pub(super) fn stat_at(dir: impl AsFd, name: &CStr) -> io::Result<Stat> {
    Ok(convert(&statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)?))
}

/// Stats what the descriptor holds.
pub(super) fn stat_fd(fd: impl AsFd) -> io::Result<Stat> {
    Ok(convert(&fstat(fd)?))
}

fn convert(raw: &RawStat) -> Stat {
    let kind = match FileType::from_raw_mode(raw.st_mode) {
        FileType::Directory => EntryKind::Dir,
        FileType::RegularFile => EntryKind::File,
        FileType::Symlink => EntryKind::Symlink,
        _ => EntryKind::Other,
    };

    // `st_birthtime` is zero where a filesystem keeps none (and then it is not a way to tell entries
    // apart).
    let birth = (raw.st_birthtime != 0 || raw.st_birthtime_nsec != 0).then(|| BirthTime {
        secs: raw.st_birthtime,
        nanos: u32::try_from(raw.st_birthtime_nsec).unwrap_or(0),
    });

    Stat {
        meta: EntryMeta {
            kind,
            allocated: u64::try_from(raw.st_blocks).unwrap_or(0).saturating_mul(512),
            apparent: u64::try_from(raw.st_size).unwrap_or(0),
            mtime: raw.st_mtime,
            // The device number as `std::fs::Metadata::dev` gives it (the scan's identity): `dev_t` is
            // a signed 32 bits, widened the same way.
            dev: raw.st_dev as u64,
            ino: raw.st_ino,
            nlink: u64::from(raw.st_nlink),
            is_mount_root: false,
        },
        birth,
        uid: raw.st_uid,
        mode: u32::from(raw.st_mode) & 0o7777,
    }
}

/// The devices that tell a firmlink from a mount: the system volume's (`/`) and the data volume's.
#[derive(Debug, Clone, Copy)]
pub(super) struct Volumes {
    root: u64,
    data: Option<u64>,
}

impl Volumes {
    pub fn read() -> io::Result<Self> {
        let root = stat_at(CWD, c"/")?.meta.dev;
        // A Mac without the data volume (before Catalina) has no firmlinks: every change is a mount.
        let data = stat_at(CWD, DATA_VOLUME).ok().map(|stat| stat.meta.dev);

        Ok(Self { root, data })
    }

    /// Whether a folder on `dev` inside one on `parent_dev` starts a mount.
    pub fn starts_mount(&self, dev: u64, parent_dev: u64) -> bool {
        starts_mount(dev, parent_dev, self.root, self.data)
    }
}

/// The names of an absolute path; `.`, `..` and empty components (`//`) are refused, a trailing `/` is
/// allowed.
pub(super) fn components(path: &[u8]) -> io::Result<Vec<&[u8]>> {
    path_components(path).ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))
}

pub(super) fn c_name(name: &[u8]) -> io::Result<CString> {
    CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// [`EntryProbe`] on macOS descriptors, with a time limit on every look.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacEntryProbe;

impl EntryProbe for MacEntryProbe {
    fn probe(&self, path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
        let path = path.to_vec();
        timed(move || walk(&path))
    }

    fn resolve(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        let path = path.to_vec();
        timed(move || {
            use std::os::unix::ffi::OsStrExt;

            let resolved = std::fs::canonicalize(std::ffi::OsStr::from_bytes(&path))?;
            Ok(resolved.into_os_string().into_encoded_bytes())
        })
    }

    fn read_link(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        let path = path.to_vec();
        timed(move || {
            use std::os::unix::ffi::OsStrExt;

            // `readlink(2)`: the link is read, never followed; `EINVAL` (not a link) is `InvalidInput`.
            let target = std::fs::read_link(std::ffi::OsStr::from_bytes(&path))?;
            Ok(target.into_os_string().into_encoded_bytes())
        })
    }
}

/// [`EntryProbe::probe`] without the time limit: one [`ProbedEntry`] per component, the entry last.
pub(super) fn walk(path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
    let parts = components(path)?;
    let volumes = Volumes::read()?;

    let mut dir: OwnedFd = openat(CWD, c"/", DIR_FLAGS, Mode::empty())?;
    let mut parent_dev = stat_fd(&dir)?.meta.dev;
    let mut found = Vec::with_capacity(parts.len());

    for (position, part) in parts.iter().enumerate() {
        let last = position + 1 == parts.len();
        let name = c_name(part)?;

        let (stat, next) = if last {
            (stat_at(&dir, &name)?, None)
        } else {
            let fd = openat(&dir, &name, DIR_FLAGS, Mode::empty()).map_err(|errno| match errno {
                // A link (`O_NOFOLLOW` with `O_DIRECTORY`) or a file on the way.
                Errno::LOOP | Errno::NOTDIR => io::Error::from(io::ErrorKind::NotADirectory),
                other => io::Error::from(other),
            })?;
            (stat_fd(&fd)?, Some(fd))
        };

        let mut entry = stat.probed();
        entry.meta.is_mount_root = volumes.starts_mount(entry.meta.dev, parent_dev);
        parent_dev = entry.meta.dev;
        found.push(entry);

        if let Some(fd) = next {
            dir = fd;
        }
    }

    Ok(found)
}

/// Runs `work` in its own thread and waits [`PROBE_TIMEOUT`] for it. The thread is left to finish (or
/// hang) on its own: a stuck system call cannot be interrupted, only stopped waiting for.
pub(super) fn timed<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    timed_for(PROBE_TIMEOUT, work)
}

fn timed_for<T: Send + 'static>(
    limit: Duration,
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("platter-probe".to_owned())
        .spawn(move || {
            let _ = send.send(work());
        })?;

    receive.recv_timeout(limit).unwrap_or_else(|_| {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the path did not answer in time (a network share or automount?)",
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt, symlink};

    use super::*;

    fn bytes(path: &std::path::Path) -> Vec<u8> {
        path.as_os_str().as_encoded_bytes().to_vec()
    }

    /// A temporary folder under its real path (`/var` is a link to `/private/var` on macOS).
    fn temp_root() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        (dir, root)
    }

    #[test]
    fn describes_every_component_down_to_the_entry() {
        let (_dir, root) = temp_root();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/f"), b"x").unwrap();

        let found = MacEntryProbe.probe(&bytes(&root.join("a/b/f"))).unwrap();

        assert_eq!(found.len(), root.components().count() - 1 + 3);
        let kinds: Vec<_> = found.iter().rev().take(3).map(|e| e.meta.kind).collect();
        assert_eq!(kinds, [EntryKind::File, EntryKind::Dir, EntryKind::Dir]);

        let meta = std::fs::symlink_metadata(root.join("a/b/f")).unwrap();
        let last = found.last().unwrap();
        assert_eq!(last.meta.ino, meta.ino());
        assert_eq!(last.meta.dev, meta.dev());
        assert_eq!(last.meta.nlink, 1);
        assert!(last.birth.is_some(), "APFS records when an entry was created");
        assert_eq!(last.mount_id, None);
    }

    #[test]
    fn a_symlink_at_the_end_is_described_and_not_followed() {
        let (_dir, root) = temp_root();
        std::fs::create_dir(root.join("real")).unwrap();
        symlink(root.join("real"), root.join("link")).unwrap();

        let found = MacEntryProbe.probe(&bytes(&root.join("link"))).unwrap();

        assert_eq!(found.last().unwrap().meta.kind, EntryKind::Symlink);
    }

    #[test]
    fn a_symlink_on_the_way_is_refused() {
        let (_dir, root) = temp_root();
        std::fs::create_dir_all(root.join("real/inner")).unwrap();
        symlink(root.join("real"), root.join("link")).unwrap();

        let error = MacEntryProbe.probe(&bytes(&root.join("link/inner"))).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotADirectory);
    }

    #[test]
    fn a_file_on_the_way_is_refused() {
        let (_dir, root) = temp_root();
        std::fs::write(root.join("file"), b"x").unwrap();

        let error = MacEntryProbe.probe(&bytes(&root.join("file/inner"))).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotADirectory);
    }

    #[test]
    fn missing_and_odd_paths_are_refused() {
        let (_dir, root) = temp_root();

        let missing = MacEntryProbe.probe(&bytes(&root.join("nope"))).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);

        for odd in [&b"relative"[..], b"/a/../b", b"/a//b", b"/a/./b"] {
            let error = MacEntryProbe.probe(odd).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{odd:?}");
        }
        assert!(MacEntryProbe.probe(b"/").unwrap().is_empty());
    }

    #[test]
    fn a_link_is_read_as_stored_and_never_followed() {
        let (_dir, root) = temp_root();
        std::fs::create_dir(root.join("real")).unwrap();
        symlink("real/../real", root.join("relative")).unwrap();
        symlink(root.join("gone"), root.join("dangling")).unwrap();

        assert_eq!(
            MacEntryProbe.read_link(&bytes(&root.join("relative"))).unwrap(),
            b"real/../real"
        );
        assert_eq!(
            MacEntryProbe.read_link(&bytes(&root.join("dangling"))).unwrap(),
            bytes(&root.join("gone"))
        );
        let plain = MacEntryProbe.read_link(&bytes(&root.join("real"))).unwrap_err();
        assert_eq!(plain.kind(), io::ErrorKind::InvalidInput);
        let missing = MacEntryProbe.read_link(&bytes(&root.join("nope"))).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn the_real_path_of_a_link_is_resolved() {
        let (_dir, root) = temp_root();
        std::fs::create_dir(root.join("real")).unwrap();
        symlink(root.join("real"), root.join("link")).unwrap();

        let resolved = MacEntryProbe.resolve(&bytes(&root.join("link"))).unwrap();

        assert_eq!(resolved, bytes(&root.join("real")));
    }

    #[test]
    fn a_look_that_takes_too_long_is_given_up() {
        let error = timed_for(Duration::from_millis(10), || {
            std::thread::sleep(Duration::from_millis(500));
            Ok(())
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn the_data_volume_is_the_same_disk_as_the_system_volume() {
        let volumes = Volumes::read().unwrap();
        let root = stat_at(CWD, c"/").unwrap().meta.dev;
        let users = stat_at(CWD, c"/Users").unwrap().meta.dev;

        assert!(!volumes.starts_mount(users, root), "/Users is a firmlink");
    }
}
