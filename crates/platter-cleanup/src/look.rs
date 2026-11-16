//! Looking folders up on the disk within one time limit, for the places that are protected by identity
//! (the kept folders and the protected places).

use std::io;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use platter_core::EntryProbe;

use crate::path::owned;

/// How many folders are looked at at the same time. A look at a mount that does not answer holds one of
/// these until the probe gives up on it, so a few more than one keep the rest of the list from waiting
/// behind it.
const LOOKS_AT_ONCE: usize = 4;

/// How many folders have their links read at the same time. Reading a link is one light call per path
/// component, so many more are in flight than for the looks above: a few paths on mounts that do not
/// answer must not keep the links of the rest from being read.
const LINK_READS_AT_ONCE: usize = 16;

/// How many links one path may lead through when it is followed by text (the kernel allows 40; a path
/// that needs more than this is not worked out and is treated as not resolved).
const LINK_HOPS: usize = 16;

/// What the disk said about one folder.
#[derive(Debug, Clone, Default)]
pub(crate) struct Looked {
    /// Where the folder really is, when that is somewhere else (it is reached through a link) and is
    /// not the top folder.
    pub real: Option<Vec<Vec<u8>>>,
    /// Device and inode of the folder (of where it really is), when that could be looked up in time.
    pub identity: Option<(u64, u64)>,
}

/// What reading only the links of one path found out: each component below `/` is read with
/// `read_link`, which looks at the entry itself and never at what a link leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Links {
    /// No link on the way (or nothing there at all).
    Absent,
    /// There is a link on the way; the folder really is here (empty: the top folder).
    To(Vec<Vec<u8>>),
    /// This pass found nothing out: the probe cannot read links, or a look failed before any link was
    /// met. Nothing is known to be wrong, so the path is taken as it is, as it was before this pass.
    Unknown,
    /// A link on the way was met, but where it leads could not be worked out (too many links, or a look
    /// on the way failed or ran out of time). The path may well be somewhere else.
    Lost,
}

/// What one keep folder came to: what the disk said, and whether where the folder really is was
/// settled. It is not when a link on its way (or the folder not being reached at all) could not be
/// worked out within the limit: then it might be somewhere that only its real path would protect.
#[derive(Debug, Clone)]
pub(crate) struct KeepLook {
    pub looked: Looked,
    pub settled: bool,
}

/// Looks every one of `paths` up (where it really is, then its device and inode), several at a time,
/// and starts no look once `budget` has run out.
///
/// The answer has one entry per path, in order; `None` for a path that was not reached in time. The
/// limit is checked before each look and again between the two halves of one, so the call takes
/// `budget` plus at most one look, which the probe bounds. A look that hangs holds one of the
/// [`LOOKS_AT_ONCE`] places only: the others go on, so a folder behind a hung one is still looked at,
/// and where it really is is always recorded when that was found out.
///
/// The top folder is never looked up as the place a link leads to (a link to `/` is not a folder worth
/// protecting: it would hold everything), and its identity is never learned.
pub(crate) fn look_up(probe: &impl EntryProbe, paths: &[Vec<u8>], budget: Duration) -> Vec<Option<Looked>> {
    let started = Instant::now();
    let answers: Mutex<Vec<Option<Looked>>> = Mutex::new(vec![None; paths.len()]);

    run_pool(LOOKS_AT_ONCE, paths.len(), started, budget, |index| {
        let (looked, _) = look(probe, &paths[index], &Links::Unknown, started, budget);
        answers.lock().unwrap_or_else(|poisoned| poisoned.into_inner())[index] = Some(looked);
    });

    answers
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// [`look_up`] for the folders the user keeps: the links of every path are read first, by text alone
/// and a good many at a time, so that where a keep really is gets known before any look at the disk
/// that may hang (a keep that is a link is the one that must not be missed). Only then are the folders
/// looked at as in [`look_up`].
///
/// The answer has one entry per path. One whose links could not be settled, or that the limit ran out
/// before (it could be a link), is `settled: false`.
pub(crate) fn look_up_keeps(probe: &impl EntryProbe, paths: &[Vec<u8>], budget: Duration) -> Vec<KeepLook> {
    let started = Instant::now();

    let read: Mutex<Vec<Option<Links>>> = Mutex::new(vec![None; paths.len()]);
    run_pool(LINK_READS_AT_ONCE, paths.len(), started, budget, |index| {
        let links = read_links(probe, &paths[index], started, budget);
        read.lock().unwrap_or_else(|poisoned| poisoned.into_inner())[index] = Some(links);
    });
    // A path not reached could be a link: nothing is known of it.
    let links: Vec<Links> = read
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .into_iter()
        .map(|links| links.unwrap_or(Links::Lost))
        .collect();

    let answers: Mutex<Vec<Option<KeepLook>>> = Mutex::new(vec![None; paths.len()]);
    run_pool(LOOKS_AT_ONCE, paths.len(), started, budget, |index| {
        let (looked, settled) = look(probe, &paths[index], &links[index], started, budget);
        answers.lock().unwrap_or_else(|poisoned| poisoned.into_inner())[index] =
            Some(KeepLook { looked, settled });
    });

    answers
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .into_iter()
        .zip(&links)
        .zip(paths)
        .map(|((answer, links), path)| answer.unwrap_or_else(|| from_links(links, path)))
        .collect()
}

/// Runs `task` for every index below `count`, `workers` at a time, and starts none once `budget` (from
/// `started`) has run out. A task that hangs holds one worker only.
fn run_pool(workers: usize, count: usize, started: Instant, budget: Duration, task: impl Fn(usize) + Sync) {
    let next = AtomicUsize::new(0);

    let work = || {
        loop {
            if started.elapsed() >= budget {
                break;
            }

            let index = next.fetch_add(1, Ordering::SeqCst);
            if index >= count {
                break;
            }

            task(index);
        }
    };

    let workers = count.min(workers);
    if workers <= 1 {
        work();
    } else {
        thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(work);
            }
        });
    }
}

/// One folder: where it really is, then, if there is still time, its identity. `links` is what reading
/// its links already found out; when that settled where the folder is, the disk is not asked again.
fn look(
    probe: &impl EntryProbe,
    path: &[u8],
    links: &Links,
    started: Instant,
    budget: Duration,
) -> (Looked, bool) {
    let literal = owned(path);
    let elsewhere = |parts: Vec<Vec<u8>>| {
        Some(parts).filter(|parts| !parts.is_empty() && Some(parts) != literal.as_ref())
    };

    let (mut real, mut settled) = match links {
        Links::Absent => (None, true),
        Links::To(parts) => (elsewhere(parts.clone()), true),
        Links::Unknown => (None, true),
        Links::Lost => (None, false),
    };
    if matches!(links, Links::Unknown | Links::Lost) {
        // The canonical path, when the disk tells it: it settles the question either way.
        if let Some(parts) = probe.resolve(path).ok().and_then(|resolved| owned(&resolved)) {
            real = elsewhere(parts);
            settled = true;
        }
    }

    if started.elapsed() >= budget {
        return (Looked { real, identity: None }, settled);
    }

    let looked_at = real.as_deref().map_or_else(|| path.to_vec(), join_parts);
    let identity = probe
        .probe(&looked_at)
        .ok()
        .and_then(|found| found.last().copied())
        .map(|entry| (entry.meta.dev, entry.meta.ino));

    (Looked { real, identity }, settled)
}

/// What a keep folder came to when no look at the disk was made for it (the limit ran out): only what
/// reading its links found.
fn from_links(links: &Links, path: &[u8]) -> KeepLook {
    let literal = owned(path);
    let real = match links {
        Links::To(parts) => {
            Some(parts.clone()).filter(|parts| !parts.is_empty() && Some(parts) != literal.as_ref())
        }
        _ => None,
    };

    KeepLook {
        looked: Looked { real, identity: None },
        settled: matches!(links, Links::Absent | Links::To(_) | Links::Unknown),
    }
}

/// Works out where `path` really is from the text of its links alone: each component below `/` is read
/// with [`EntryProbe::read_link`] (the entry itself, never what a link leads to), a link's text (absolute
/// or relative, with `.` and `..` taken away) replaces the component, and the result is followed on, up to
/// [`LINK_HOPS`] links. No disk that a link leads to is touched, so a link to a mount that does not
/// answer is still known for where it leads. Every call is bounded by the probe, and none is made once
/// `budget` has run out.
fn read_links(probe: &impl EntryProbe, path: &[u8], started: Instant, budget: Duration) -> Links {
    let Some(mut pending) = owned(path) else {
        return Links::Unknown;
    };
    pending.reverse();

    let mut real: Vec<Vec<u8>> = Vec::new();
    let mut hops = 0;

    while let Some(part) = pending.pop() {
        if part == b"." {
            continue;
        }
        if part == b".." {
            real.pop();
            continue;
        }
        if started.elapsed() >= budget {
            return failed(hops);
        }

        real.push(part);
        match probe.read_link(&join_parts(&real)) {
            Ok(text) => {
                hops += 1;
                if hops > LINK_HOPS || text.is_empty() {
                    return Links::Lost;
                }

                real.pop();
                if text.starts_with(b"/") {
                    real.clear();
                }
                pending.extend(
                    text.split(|&byte| byte == b'/')
                        .filter(|part| !part.is_empty())
                        .rev()
                        .map(<[u8]>::to_vec),
                );
            }
            // Not a link: stays as it is.
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {}
            // Nothing is there (any more): what is left of the path is taken as written.
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                while let Some(part) = pending.pop() {
                    match part.as_slice() {
                        b"." => {}
                        b".." => {
                            real.pop();
                        }
                        _ => real.push(part),
                    }
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::Unsupported && hops == 0 => return Links::Unknown,
            Err(_) => return failed(hops),
        }
    }

    if hops == 0 { Links::Absent } else { Links::To(real) }
}

/// A look failed or ran out of time after `hops` links were met: a link is known to be on the way when
/// there was one.
fn failed(hops: usize) -> Links {
    if hops == 0 { Links::Unknown } else { Links::Lost }
}

/// The path of `parts`.
pub(crate) fn join_parts(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut path = Vec::new();
    for part in parts {
        path.push(b'/');
        path.extend_from_slice(part);
    }

    path
}
