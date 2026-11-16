use std::cell::Cell;

use rayon::prelude::*;

use super::cancel::CancelToken;
use super::chunk::{Chunk, ChunkEntry, LinkRef};
use super::progress::{Delta, PathChain, Reporter};
use crate::budget::{EntryBudget, LimitHit, memory_cost};
use crate::model::{Category, EntryKind, NodeKind, SkipReason};
use crate::ports::{DirListing, DirSource, RawEntry};

/// Directories nested deeper than this are not read (counted as an error). It bounds recursion depth,
/// and with it the stack the workers need.
pub(crate) const MAX_DEPTH: u32 = 1024;

/// Most `walk` frames that may be on one worker's stack at the same time (see [`Walker::walk`]). At about
/// 9 KB per frame in a debug build, the worst measured size of a frame, this is 18 MB of the 32 MB each
/// worker gets, and far less in a release build.
pub(crate) const MAX_STACK_FRAMES: usize = 2 * MAX_DEPTH as usize;

thread_local! {
    /// `walk` frames on this thread's stack right now, whichever task they belong to.
    static FRAMES: Cell<usize> = const { Cell::new(0) };
}

/// Counts one `walk` frame on the current thread for as long as it lives.
struct FrameGuard;

impl FrameGuard {
    fn enter() -> Self {
        FRAMES.set(FRAMES.get() + 1);
        FrameGuard
    }
}

impl Drop for FrameGuard {
    fn drop(&mut self) {
        FRAMES.set(FRAMES.get() - 1);
    }
}

/// `walk` frames on the current thread's stack (for tests).
#[cfg(test)]
pub(crate) fn frames_on_this_thread() -> usize {
    FRAMES.get()
}

/// Pretends the current thread already has `frames` frames on its stack (for tests).
#[cfg(test)]
pub(crate) fn set_frames_on_this_thread(frames: usize) {
    FRAMES.set(frames);
}

/// Most entries a tree can hold: node ids are `u32` and `u32::MAX` is reserved (`NO_PARENT`), and the
/// root takes one id. Mirrors the check the assembler makes after the fact.
pub(crate) const MAX_ENTRIES: u64 = u32::MAX as u64 - 2;

/// Most bytes of names the arena can address (`u32` offsets: 4 GiB). Mirrors the check the assembler makes
/// after the fact; the walker stops as soon as the running sum would pass it.
pub(crate) const MAX_NAME_BYTES: u64 = u32::MAX as u64;

/// Parallel directory walk.
///
/// Each directory is read by one worker, which then fans its subdirectories out with rayon. Workers own
/// everything they build (a [`Chunk`]); the only shared state is the cancel flag, the entry budget and a
/// few relaxed atomic counters flushed once per directory, so there is no lock on the per-file path (the
/// budget takes its lock once per batch of a thousand entries).
///
/// Three guards keep a hostile or broken filesystem from exhausting memory or looping: the entry budget
/// (see [`crate::budget`]), which the source draws on while it lists a directory, so the walk stops as
/// soon as the entry cap, the memory estimate or the arena's 4 GiB of names would be passed, however
/// many directories are being read at that moment, and the scan fails with `TooLarge`; and an
/// ancestor-chain check that refuses to enter a directory whose (dev, ino) equals one of its own
/// ancestors. The chain lives on the walk's stack, so the check costs a few comparisons per directory and
/// needs no shared set or lock; it catches the loops a filesystem can really produce (bind mounts,
/// corrupt images), where a cycle always passes through an ancestor.
///
/// # Stack use
///
/// `walk` recurses once per directory level, so the stack grows with depth ([`MAX_DEPTH`] levels at
/// most). A worker that has forked its subdirectories and waits for a half another worker stole does
/// not sleep: rayon lets it steal further tasks and run them on top of its current frames, so frames of
/// unrelated deep chains can pile up on one stack. To bound that, a directory forks only while the
/// thread has room for one more maximum-depth task on top ([`MAX_STACK_FRAMES`]); otherwise it reads its
/// subdirectories one after the other. Serial reading never waits, hence never steals, so a stack holds
/// at most `MAX_STACK_FRAMES` frames whatever the shape of the tree. Real trees are far shallower than
/// the limit, so they always fork. Directories with a single subdirectory never fork.
pub(crate) struct Walker<'a, S: DirSource> {
    pub source: &'a S,
    pub cancel: &'a CancelToken,
    pub reporter: &'a Reporter<'a>,
    /// What the walk may collect; once a limit is hit every worker stops and the scan reports `TooLarge`.
    pub budget: &'a EntryBudget,
}

impl<S: DirSource> Walker<'_, S> {
    pub fn walk(&self, handle: S::Handle, depth: u32, path: &PathChain<'_>) -> Chunk {
        if self.cancel.is_cancelled() || self.budget.is_exhausted() {
            return Chunk::default();
        }
        if depth > MAX_DEPTH {
            self.reporter.add(Delta {
                errors: 1,
                ..Delta::default()
            });
            return Chunk::unreadable();
        }

        let _frame = FrameGuard::enter();
        self.reporter.tick(path);

        // The source reserves room for every entry it lists, so what a listing holds is already counted
        // when it is returned. A listing that fails midway gives its room back (the lease is dropped).
        let mut lease = self.budget.lease();
        let listing = match self.source.read_dir(&handle, &mut lease) {
            Ok(listing) => listing,
            Err(_) => {
                self.reporter.add(Delta {
                    errors: 1,
                    ..Delta::default()
                });
                return Chunk::unreadable();
            }
        };
        // Release the directory before descending so open descriptors stay proportional to depth.
        drop(handle);

        // A source that stopped because it was refused room has returned part of a directory.
        let (name_bytes, memory) = listing_cost(&listing.entries);
        if self.budget.is_exhausted() || !lease.commit(listing.entries.len(), name_bytes, memory) {
            return Chunk::default();
        }

        let Some((mut chunk, jobs, delta)) = self.build_chunk(listing, name_bytes) else {
            self.budget.note_hit(LimitHit::NameBytes);
            return Chunk::default();
        };
        self.reporter.add(delta);

        let visit = |(index, handle, identity): Job<S::Handle>| {
            if path.is_inside(identity) {
                self.reporter.add(Delta {
                    errors: 1,
                    ..Delta::default()
                });
                return Chunk::unreadable();
            }

            let name = &chunk.names[chunk.entries[index].name_range()];
            self.walk(
                handle,
                depth + 1,
                &PathChain {
                    parent: Some(path),
                    name,
                    identity,
                },
            )
        };

        let may_wait = jobs.len() > 1 && FRAMES.get() + MAX_DEPTH as usize <= MAX_STACK_FRAMES;
        let subdirs: Vec<Chunk> = if may_wait {
            jobs.into_par_iter().map(visit).collect()
        } else {
            jobs.into_iter().map(visit).collect()
        };
        chunk.subdirs = subdirs;
        chunk
    }

    /// Turns a raw listing into a name-sorted chunk plus the subdirectories still to be read (with their
    /// identity). `name_bytes` is what [`listing_name_bytes`] says for it: the chunk's name buffer is
    /// allocated at exactly that size (growing it by doubling would keep up to twice as much alive until
    /// the tree is assembled). `None` when the directory's names do not fit the `u32` offsets of a chunk.
    fn build_chunk(&self, listing: DirListing<S::Handle>, name_bytes: u64) -> Option<Built<S::Handle>> {
        let DirListing { mut entries, errors } = listing;
        entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));

        let mut chunk = Chunk::default();
        chunk.entries.reserve_exact(entries.len());
        chunk.names.reserve_exact(usize::try_from(name_bytes).ok()?);
        let mut jobs = Vec::new();
        let mut skips = Vec::new();
        let mut delta = Delta {
            errors: u64::from(errors),
            ..Delta::default()
        };

        for raw in entries {
            let meta = raw.meta;
            let index = chunk.entries.len();
            let name_len = raw.name.len().min(usize::from(u16::MAX));

            let name_start = u32::try_from(chunk.names.len()).ok()?;
            chunk.names.extend_from_slice(&raw.name[..name_len]);

            let mut entry = ChunkEntry {
                name_start,
                name_len: name_len as u16,
                kind: NodeKind::Other,
                category: Category::Other,
                descended: false,
                allocated: meta.allocated,
                apparent: meta.apparent,
                mtime: meta.mtime,
            };

            // Hardlinked entries are deduplicated only when the tree is assembled, so which link owns
            // the bytes is not known here. They stay out of the running number: progress therefore
            // never exceeds the final total (it is a lower bound that jumps up when the scan ends).
            let mut counts_toward_progress = true;

            match meta.kind {
                EntryKind::Dir => {
                    delta.dirs += 1;
                    // A directory's own length is filesystem bookkeeping, not data; only its blocks count.
                    entry.apparent = 0;

                    // Whether a mount root is entered is the source's decision (it knows what counts as
                    // the same filesystem); a missing handle on a mount root makes it a `mount` node.
                    let not_descended = meta.is_mount_root && raw.child.is_none();
                    if not_descended {
                        entry.kind = NodeKind::Mount;
                        entry.allocated = 0;
                        entry.apparent = 0;

                        // `Unknown` is what a mount without a reason is, so it is not stored.
                        if let Some(reason) = raw.skipped.filter(|&reason| reason != SkipReason::Unknown) {
                            skips.push((index as u32, reason));
                        }
                    } else {
                        entry.kind = NodeKind::Dir;
                        if let Some(child) = raw.child {
                            entry.descended = true;
                            jobs.push((index, child, (meta.dev, meta.ino)));
                        }
                    }
                }
                kind => {
                    delta.files += 1;

                    entry.kind = match kind {
                        EntryKind::File => NodeKind::File,
                        EntryKind::Symlink => NodeKind::Symlink,
                        _ => NodeKind::Other,
                    };
                    if entry.kind == NodeKind::File {
                        entry.category = Category::of_file(&raw.name[..name_len], meta.apparent);
                    }
                    if meta.nlink > 1 {
                        counts_toward_progress = false;
                        chunk.links.push(LinkRef {
                            entry: index as u32,
                            dev: meta.dev,
                            ino: meta.ino,
                        });
                    }
                }
            }

            if counts_toward_progress {
                delta.bytes = delta.bytes.saturating_add(entry.allocated);
            }
            chunk.entries.push(entry);
        }

        chunk.skips = (!skips.is_empty()).then(|| skips.into_boxed_slice());

        Some((chunk, jobs, delta))
    }
}

/// Bytes of names a listing adds to the arena (names are cut at `u16::MAX`) and its estimated memory.
fn listing_cost<H>(entries: &[RawEntry<H>]) -> (u64, u64) {
    let mut name_bytes = 0;
    let mut memory = 0;

    for entry in entries {
        let name = entry.name.len().min(usize::from(u16::MAX)) as u64;

        name_bytes += name;
        memory += memory_cost(name, entry.meta.kind == EntryKind::Dir);
    }
    (name_bytes, memory)
}

/// A subdirectory still to be read: its entry index, its handle and its (dev, ino).
type Job<H> = (usize, H, (u64, u64));

/// A built chunk with the subdirectories still to be read and the counters to flush.
type Built<H> = (Chunk, Vec<Job<H>>, Delta);
