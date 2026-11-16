//! Opening a directory so that it is the directory that was listed.
//!
//! The walker lists a directory, remembers what it saw of each subdirectory (device, inode, mount id)
//! and opens it later, possibly much later and possibly from the scan root downwards. Between the
//! listing and the open anything can happen to the name: another directory can be moved in, or a
//! filesystem mounted over it. Reading whatever is behind the name then would count foreign data under
//! the wrong node, and a mount appearing on the way would be entered without ever having been
//! classified. Two measures close that:
//!
//! - **`openat2` with `RESOLVE_NO_XDEV`** for a directory that was not a mount root when listed: the
//!   kernel refuses the open (`EXDEV`) if the name now leads into another mount. It also passes
//!   `RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, on top of `O_NOFOLLOW`. This is atomic, no race
//!   window remains for the mount case. A directory that *was* an accepted mount root is opened
//!   without `NO_XDEV` (the open has to cross into the mount). Kernels before 5.6 answer `ENOSYS`, and
//!   some sandboxes refuse the syscall (`EPERM`, `EACCES`): whether `openat2` can be used is decided
//!   once per process by a probe that touches no scanned data (see [`openat2_usable`]), and where it
//!   cannot the open is a plain `openat`. An error from `openat2` on a real directory never changes that
//!   decision: it is that directory's error, counted by the scan.
//! - **A `statx` of the opened descriptor** (`AT_EMPTY_PATH | AT_NO_AUTOMOUNT`) compared with the
//!   listing: device, inode and mount id must all be equal, else the directory is not read and the
//!   caller counts one error. This is what catches a plain swap for another directory (no syscall flag
//!   can), and it runs for every open, whichever way it was done. The same check runs on every
//!   component when a handle is reopened from the scan root.
//!
//! What stays possible is a swap that puts back the very same directory (same device, inode and mount),
//! which is not a swap.

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use rustix::fs::{CWD, Mode, OFlags, ResolveFlags, openat, openat2};
use rustix::io::Errno;

use super::stat::{Stat, stat_fd_identity};

/// How often a directory open that hits the descriptor limit is retried (with growing sleeps).
const EXHAUSTED_RETRIES: u32 = 40;

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// The flags of every `openat2` of the reader, probe included.
const NO_SYMLINKS: ResolveFlags = ResolveFlags::NO_SYMLINKS.union(ResolveFlags::NO_MAGICLINKS);

/// Whether `openat2` can be used by this process, decided at the first open and kept for good.
static OPENAT2_USABLE: OnceLock<bool> = OnceLock::new();

/// What the listing saw of a directory: the directory that has to be behind its name when it is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Identity {
    pub dev: u64,
    pub ino: u64,
    /// `None` on kernels that report no mount id (before 5.8): only device and inode are compared.
    pub mount_id: Option<u64>,
    /// The directory is a mount root (an accepted one: nothing else gets a handle). Opening it crosses
    /// into another mount, which is allowed only then.
    pub mount_root: bool,
}

impl Identity {
    pub fn new(stat: &Stat, mount_root: bool) -> Self {
        Self {
            dev: stat.meta.dev,
            ino: stat.meta.ino,
            mount_id: stat.mount_id,
            mount_root,
        }
    }

    /// Whether `now`, the statx of an opened descriptor, is the directory this identity describes.
    pub fn matches(&self, now: &Stat) -> bool {
        let same_mount = match (self.mount_id, now.mount_id) {
            (Some(listed), Some(opened)) => listed == opened,
            _ => true,
        };

        self.dev == now.meta.dev && self.ino == now.meta.ino && same_mount
    }
}

/// The names from the scan root down to a directory, as a linked list shared with its siblings'
/// ancestors, each with the identity it was listed with. It lets a handle be reopened from the root when
/// its parent descriptor is gone, checking every component on the way.
#[derive(Debug)]
pub(super) struct Chain {
    pub name: CString,
    pub identity: Identity,
    pub parent: Option<Arc<Chain>>,
}

/// Opens the directory `chain` names and checks it against the listing.
///
/// With `parent` (the descriptor of the directory it was listed in) that is one `openat`; without, the
/// chain is walked down from `root`, one component at a time. Retries while the process is out of file
/// descriptors. Returns the descriptor with its verified identity (its device is what the entries of
/// the directory are compared with to spot mount boundaries on old kernels).
pub(super) fn open_child(
    root: &OwnedFd,
    chain: &Chain,
    parent: Option<&OwnedFd>,
) -> io::Result<(OwnedFd, Stat)> {
    retry_exhausted(|| match parent {
        Some(parent) => open_checked(parent, &chain.name, &chain.identity),
        None => open_from_root(root, chain),
    })
}

/// Opens the directory `chain` names by descending from `root` one component at a time, checking each.
fn open_from_root(root: &OwnedFd, chain: &Chain) -> io::Result<(OwnedFd, Stat)> {
    let mut links = Vec::new();
    let mut link = Some(chain);
    while let Some(current) = link {
        links.push(current);
        link = current.parent.as_deref();
    }

    let mut current: Option<(OwnedFd, Stat)> = None;
    for link in links.into_iter().rev() {
        let parent = current.as_ref().map_or(root.as_fd(), |(fd, _)| fd.as_fd());

        let opened = open_checked(parent, &link.name, &link.identity)?;
        current = Some(opened);
    }

    current.ok_or_else(|| io::Error::other("a directory chain names at least one directory"))
}

/// One `openat` of `name` below `parent`, then the check against what the listing saw.
fn open_checked(parent: impl AsFd, name: &CStr, listed: &Identity) -> io::Result<(OwnedFd, Stat)> {
    let fd = open_below(parent, name, listed.mount_root)?;

    let opened = stat_fd_identity(&fd)?;
    if !listed.matches(&opened) {
        return Err(io::Error::other(
            "the directory was replaced or mounted over while the scan was running",
        ));
    }
    Ok((fd, opened))
}

/// `openat2` without symlinks and, unless the directory is a mount root, without crossing a mount;
/// plain `openat` where `openat2` is not usable in this process (see [`openat2_usable`]).
fn open_below(parent: impl AsFd, name: &CStr, is_mount_root: bool) -> io::Result<OwnedFd> {
    open_with(
        openat2_usable(),
        || {
            let mut resolve = NO_SYMLINKS;
            if !is_mount_root {
                resolve |= ResolveFlags::NO_XDEV;
            }

            openat2(&parent, name, DIR_FLAGS, Mode::empty(), resolve)
        },
        || openat(&parent, name, DIR_FLAGS, Mode::empty()),
    )
}

/// Opens with `with_openat2` when `usable`, with `plain` otherwise. There is no fallback from the one to
/// the other: an error of `openat2` (`EPERM` included, which a filesystem can return for what it does not
/// like) is the error of this directory, and nothing is remembered about it.
fn open_with(
    usable: bool,
    with_openat2: impl FnOnce() -> Result<OwnedFd, Errno>,
    plain: impl FnOnce() -> Result<OwnedFd, Errno>,
) -> io::Result<OwnedFd> {
    let result = if usable { with_openat2() } else { plain() };

    Ok(result?)
}

/// Whether this process can use `openat2`: asked once, with a probe that does not depend on anything
/// that is scanned, so no filesystem under a scan root can influence the answer.
fn openat2_usable() -> bool {
    *OPENAT2_USABLE.get_or_init(|| usable_after_probe(probe_openat2()))
}

/// `openat2` of `/` with the flags of production (`NO_XDEV` included: opening `/` crosses no mount).
fn probe_openat2() -> Result<(), Errno> {
    let resolve = NO_SYMLINKS | ResolveFlags::NO_XDEV;

    openat2(CWD, "/", DIR_FLAGS, Mode::empty(), resolve).map(drop)
}

/// What the probe says: unusable when the kernel does not know the syscall (`ENOSYS`, before 5.6) or a
/// seccomp filter or security module refuses it (`EPERM`, `EACCES`); usable on success, and on any
/// other error too, because those (out of descriptors, say) show that the syscall exists and was
/// allowed to run.
fn usable_after_probe(probe: Result<(), Errno>) -> bool {
    !matches!(probe, Err(Errno::NOSYS | Errno::PERM | Errno::ACCESS))
}

fn is_fd_exhausted(error: &io::Error) -> bool {
    error
        .raw_os_error()
        .is_some_and(|code| code == Errno::MFILE.raw_os_error() || code == Errno::NFILE.raw_os_error())
}

/// Runs `open`, sleeping and retrying while the process is out of file descriptors; other workers
/// close theirs as they finish, so this normally succeeds within a few milliseconds.
fn retry_exhausted<T>(mut open: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut delay = Duration::from_millis(1);

    for _ in 0..EXHAUSTED_RETRIES {
        match open() {
            Err(error) if is_fd_exhausted(&error) => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(50));
            }
            result => return result,
        }
    }
    open()
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::os::fd::OwnedFd;
    use std::path::Path;

    use rustix::fs::CWD;

    use super::*;
    use crate::linux::stat::{stat_at, stat_fd};

    fn open_dir(path: &Path) -> OwnedFd {
        openat(CWD, path, DIR_FLAGS, Mode::empty()).expect("open the directory")
    }

    fn listed(parent: &OwnedFd, name: &CStr, mount_root: bool) -> Identity {
        let stat = stat_at(parent, name).expect("stat the entry");
        Identity::new(&stat, mount_root)
    }

    fn some_fd() -> OwnedFd {
        open_dir(Path::new("/"))
    }

    #[test]
    fn the_probe_decides_from_the_error_it_got() {
        assert!(usable_after_probe(Ok(())));

        for refused in [Errno::NOSYS, Errno::PERM, Errno::ACCESS] {
            assert!(!usable_after_probe(Err(refused)), "{refused:?} means unusable");
        }
        for other in [
            Errno::MFILE,
            Errno::NFILE,
            Errno::NOMEM,
            Errno::INTR,
            Errno::NOENT,
        ] {
            assert!(
                usable_after_probe(Err(other)),
                "{other:?} shows the syscall exists"
            );
        }
    }

    #[test]
    fn the_real_probe_finds_openat2_on_this_kernel_and_the_answer_is_final() {
        // The kernel of any machine that runs the test suite is newer than 5.6; in a sandbox that refuses
        // the syscall the probe says no, and then so does every later question.
        let first = openat2_usable();

        assert_eq!(first, usable_after_probe(probe_openat2()));
        assert_eq!(openat2_usable(), first);
    }

    #[test]
    fn an_openat2_error_on_a_real_directory_is_that_directorys_error_and_falls_back_to_nothing() {
        let mut plain_calls = 0;

        for refused in [Errno::PERM, Errno::NOSYS, Errno::XDEV, Errno::ACCESS] {
            let error = open_with(
                true,
                || Err(refused),
                || {
                    plain_calls += 1;
                    Ok(some_fd())
                },
            )
            .expect_err("no second try with plain openat");

            assert_eq!(error.raw_os_error(), Some(refused.raw_os_error()));
        }
        assert_eq!(
            plain_calls, 0,
            "a plain openat would drop NO_XDEV and NO_SYMLINKS"
        );

        // The decision is the probe's alone, and the same arguments give the same answer afterwards.
        let again = open_with(true, || Err(Errno::PERM), || Ok(some_fd()));
        assert!(again.is_err());
    }

    #[test]
    fn where_the_probe_found_openat2_unusable_the_plain_open_is_used_and_where_usable_it_is_not() {
        let mut calls = (0, 0);

        let plain = open_with(
            false,
            || {
                calls.0 += 1;
                Ok(some_fd())
            },
            || {
                calls.1 += 1;
                Ok(some_fd())
            },
        );
        assert!(plain.is_ok());
        assert_eq!(calls, (0, 1));

        let modern = open_with(
            true,
            || {
                calls.0 += 1;
                Ok(some_fd())
            },
            || {
                calls.1 += 1;
                Ok(some_fd())
            },
        );
        assert!(modern.is_ok());
        assert_eq!(calls, (1, 1));
    }

    #[test]
    fn an_unchanged_directory_opens_and_a_replaced_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        let parent = open_dir(dir.path());
        let identity = listed(&parent, c"sub", false);

        assert!(open_checked(&parent, c"sub", &identity).is_ok());

        fs::rename(dir.path().join("sub"), dir.path().join("sub-old")).unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();

        let error =
            open_checked(&parent, c"sub", &identity).expect_err("another directory is behind the name");
        assert!(error.to_string().contains("replaced"), "{error}");
    }

    #[test]
    fn a_directory_swapped_for_a_symlink_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::create_dir(dir.path().join("elsewhere")).unwrap();
        let parent = open_dir(dir.path());
        let identity = listed(&parent, c"sub", false);

        fs::remove_dir(dir.path().join("sub")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), dir.path().join("sub")).unwrap();

        assert!(open_checked(&parent, c"sub", &identity).is_err());
    }

    #[test]
    fn the_identity_compares_device_inode_and_mount_id() {
        let dir = tempfile::tempdir().unwrap();
        let fd = open_dir(dir.path());
        let stat = stat_fd(&fd).unwrap();
        let identity = Identity::new(&stat, false);

        assert!(identity.matches(&stat));
        assert!(
            !Identity {
                ino: identity.ino + 1,
                ..identity
            }
            .matches(&stat)
        );
        assert!(
            !Identity {
                dev: identity.dev + 1,
                ..identity
            }
            .matches(&stat)
        );
        if let Some(mount_id) = identity.mount_id {
            assert!(
                !Identity {
                    mount_id: Some(mount_id + 1),
                    ..identity
                }
                .matches(&stat)
            );
        }
        assert!(
            Identity {
                mount_id: None,
                ..identity
            }
            .matches(&stat),
            "a kernel without mount ids compares device and inode only"
        );
    }

    #[test]
    fn a_directory_that_became_a_mount_is_refused_unless_it_was_listed_as_one() {
        // `/proc` is a mount root of another filesystem on every Linux system that has it.
        let Ok(proc_stat) = std::fs::metadata("/proc") else {
            return;
        };
        let root = open_dir(Path::new("/"));
        let stat = stat_at(&root, c"proc").unwrap();
        if !stat.meta.is_mount_root || proc_stat.file_type().is_symlink() {
            return;
        }

        // Listed as an ordinary directory (as if the mount had appeared since): refused. Whether the
        // kernel refuses the open (EXDEV) or the identity check does, depends on its version.
        let as_listed_before_the_mount = Identity {
            mount_root: false,
            ..Identity::new(&stat, false)
        };
        assert!(open_checked(&root, c"proc", &as_listed_before_the_mount).is_err());

        // The same directory listed as the mount root it is: opens.
        assert!(open_checked(&root, c"proc", &Identity::new(&stat, true)).is_ok());
    }

    #[test]
    fn opening_from_the_root_checks_every_component() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        File::create(dir.path().join("a/b/f")).unwrap();
        let root = open_dir(dir.path());

        let a = listed(&root, c"a", false);
        let a_fd = open_dir(&dir.path().join("a"));
        let b = listed(&a_fd, c"b", false);

        let chain_a = Arc::new(Chain {
            name: c"a".to_owned(),
            identity: a,
            parent: None,
        });
        let chain_b = Chain {
            name: c"b".to_owned(),
            identity: b,
            parent: Some(chain_a),
        };
        assert!(open_from_root(&root, &chain_b).is_ok());

        // `a` is replaced by a copy of itself, `b` included; the last component still looks right.
        fs::rename(dir.path().join("a"), dir.path().join("a-old")).unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();

        let error = open_from_root(&root, &chain_b).expect_err("the middle component changed");
        assert!(error.to_string().contains("replaced"), "{error}");
    }
}
