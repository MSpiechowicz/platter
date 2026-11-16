//! The allowance of one scan: how many entries, name bytes and bytes of memory it may collect.
//!
//! The limits are a shared reservation, not a check made afterwards. Every worker reads a directory
//! through a [`Lease`]; the source asks the lease for room before it materialises an entry
//! ([`Lease::admit`]), and the lease takes that room from the scan's [`EntryBudget`] in batches. When
//! the room is gone the listing stops at once and the scan ends with `TooLarge`. So what is held in
//! memory is bounded by what was reserved, however many directories are being read at the same moment.
//!
//! # Bounds
//!
//! - Entries, name bytes and the memory estimate never exceed their limits: a reservation is granted
//!   only if it fits (under one lock, so no race can overshoot), and entries are materialised only
//!   after they were reserved. For a source that honours the contract the overshoot is zero.
//! - A worker may hold up to [`BATCH_ENTRIES`] entries (and [`BATCH_NAME_BYTES`] of names) that it
//!   reserved but has not used yet. All workers together hold at most `workers * BATCH_ENTRIES`
//!   entries of room that nobody can use for the moment, so a scan whose total is within
//!   `workers * BATCH_ENTRIES` of a limit may be refused although it would just fit. The batch shrinks
//!   with the room left (a worker asks for at most `room / (2 * workers)`), so near a limit the idle
//!   room is about one entry per worker, and a scan on one worker is exact.
//! - A source that does not use the lease at all is still bounded: the walker settles the finished
//!   listing against the budget (`Lease::commit`). The listing was read in full before that, so the
//!   overshoot is then up to one listing per worker, as before the lease existed.
//!
//! # The memory estimate
//!
//! A scan is also bounded by an estimate of the memory it needs at its peak, which is when a directory
//! is turned into a chunk and later into the tree: [`ENTRY_BASE_COST_BYTES`] for an entry, twice the
//! length of its name (the name is alive twice at the peak) and [`DIR_EXTRA_COST_BYTES`] more for a
//! directory (its handle and its chunk). The estimate is reserved like the entries: it is a third
//! counter, with its own limit.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Bytes of memory counted for an entry besides its name. Measured peak of a scan of one huge
/// directory with short names (the raw listing and the chunk are alive together, then the chunk and the
/// tree): 203 bytes per entry, rounded up. Many small directories cost about half of that.
pub const ENTRY_BASE_COST_BYTES: u64 = 208;

/// How many times the name of an entry is counted: at the peak it is held twice (raw listing and chunk,
/// or chunk and arena). Measured with 250-byte names: 685 bytes per entry, which is 203 + 1.93 * 250.
pub const NAME_COPIES: u64 = 2;

/// Bytes counted for a directory on top of [`ENTRY_BASE_COST_BYTES`]: the handle the source keeps until
/// the directory is read, and its (mostly empty) chunk.
pub const DIR_EXTRA_COST_BYTES: u64 = 320;

/// Most entries a worker reserves at once.
pub const BATCH_ENTRIES: u64 = 1024;

/// Most name bytes a worker reserves at once (32 bytes for each entry of a batch).
pub const BATCH_NAME_BYTES: u64 = 32 * 1024;

/// Most memory a worker reserves at once, by the estimate: a batch of entries with short names.
pub const BATCH_MEMORY_BYTES: u64 = BATCH_ENTRIES * (ENTRY_BASE_COST_BYTES + 64);

/// What a scan may collect. A limit of `u64::MAX` is no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetLimits {
    pub max_entries: u64,
    /// Bytes of names (the arena's `u32` offsets allow 4 GiB).
    pub max_name_bytes: u64,
    /// Bytes of memory by the estimate of the module documentation.
    pub max_memory_bytes: u64,
}

impl BudgetLimits {
    pub const UNLIMITED: BudgetLimits = BudgetLimits {
        max_entries: u64::MAX,
        max_name_bytes: u64::MAX,
        max_memory_bytes: u64::MAX,
    };
}

/// The limit that ended a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitHit {
    Entries,
    NameBytes,
    Memory,
}

impl LimitHit {
    const fn code(self) -> u8 {
        match self {
            LimitHit::Entries => 1,
            LimitHit::NameBytes => 2,
            LimitHit::Memory => 3,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(LimitHit::Entries),
            2 => Some(LimitHit::NameBytes),
            3 => Some(LimitHit::Memory),
            _ => None,
        }
    }
}

/// Entries, the bytes of their names and the memory they are estimated to need: what is reserved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Amount {
    entries: u64,
    name_bytes: u64,
    memory: u64,
}

impl Amount {
    fn plus(self, other: Amount) -> Amount {
        Amount {
            entries: self.entries.saturating_add(other.entries),
            name_bytes: self.name_bytes.saturating_add(other.name_bytes),
            memory: self.memory.saturating_add(other.memory),
        }
    }

    fn minus(self, other: Amount) -> Amount {
        Amount {
            entries: self.entries.saturating_sub(other.entries),
            name_bytes: self.name_bytes.saturating_sub(other.name_bytes),
            memory: self.memory.saturating_sub(other.memory),
        }
    }

    fn is_zero(self) -> bool {
        self == Amount::default()
    }
}

/// The estimated memory of an entry with a name of `name_bytes` bytes.
pub(crate) fn memory_cost(name_bytes: u64, is_dir: bool) -> u64 {
    let extra = if is_dir { DIR_EXTRA_COST_BYTES } else { 0 };

    ENTRY_BASE_COST_BYTES + NAME_COPIES * name_bytes + extra
}

/// The shared allowance of one scan (see the module documentation).
#[derive(Debug)]
pub struct EntryBudget {
    limits: BudgetLimits,
    workers: u64,
    reserved: Mutex<Amount>,
    /// The first limit that was hit, as [`LimitHit::code`]; 0 while none was.
    hit: AtomicU8,
}

impl EntryBudget {
    /// A budget for a scan on `workers` threads.
    pub fn new(limits: BudgetLimits, workers: usize) -> Self {
        Self {
            limits,
            workers: workers.max(1) as u64,
            reserved: Mutex::new(Amount::default()),
            hit: AtomicU8::new(0),
        }
    }

    /// A budget that never refuses: for reading a directory outside a scan.
    pub fn unlimited() -> Self {
        Self::new(BudgetLimits::UNLIMITED, 1)
    }

    /// A handle to read one directory with.
    pub fn lease(&self) -> Lease<'_> {
        Lease {
            budget: self,
            reserved: Amount::default(),
            taken: Amount::default(),
        }
    }

    /// Entries and name bytes reserved right now. Once every listing of a scan is settled this is what
    /// the scan collected.
    pub fn used(&self) -> (u64, u64) {
        let reserved = *self.lock();

        (reserved.entries, reserved.name_bytes)
    }

    /// The memory estimate reserved right now (see the module documentation).
    pub fn used_memory(&self) -> u64 {
        self.lock().memory
    }

    /// The first limit that was hit, if any.
    pub fn limit_hit(&self) -> Option<LimitHit> {
        LimitHit::from_code(self.hit.load(Ordering::Relaxed))
    }

    /// Whether a limit was hit: the scan is lost and every worker should stop.
    pub fn is_exhausted(&self) -> bool {
        self.hit.load(Ordering::Relaxed) != 0
    }

    /// Records that `hit` ended the scan. Only the first one is kept.
    pub(crate) fn note_hit(&self, hit: LimitHit) {
        let _ = self
            .hit
            .compare_exchange(0, hit.code(), Ordering::Relaxed, Ordering::Relaxed);
    }

    fn lock(&self) -> MutexGuard<'_, Amount> {
        self.reserved.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes between `min` and `want` from the budget, or `None` (and a recorded limit) when even `min`
    /// does not fit.
    fn reserve(&self, want: Amount, min: Amount) -> Option<Amount> {
        let mut reserved = self.lock();

        // Under the lock: the limit is noted under it too, so no worker can take room that a listing of
        // the lost scan gives back a moment later.
        if self.is_exhausted() {
            return None;
        }

        let room = self.room(*reserved);
        if let Some(hit) = self.shortfall(room, min) {
            self.note_hit(hit);
            return None;
        }

        // Never ask for more than a fair share of what is left, so what the other workers hold unused
        // stays small near a limit.
        let share = |room: u64| (room / (2 * self.workers)).max(1);
        let grant = |want: u64, min: u64, room: u64| want.min(share(room)).max(min).min(room);
        let grant = Amount {
            entries: grant(want.entries, min.entries, room.entries),
            name_bytes: grant(want.name_bytes, min.name_bytes, room.name_bytes),
            memory: grant(want.memory, min.memory, room.memory),
        };

        *reserved = reserved.plus(grant);
        Some(grant)
    }

    /// Room left in each dimension.
    fn room(&self, reserved: Amount) -> Amount {
        Amount {
            entries: self.limits.max_entries.saturating_sub(reserved.entries),
            name_bytes: self.limits.max_name_bytes.saturating_sub(reserved.name_bytes),
            memory: self.limits.max_memory_bytes.saturating_sub(reserved.memory),
        }
    }

    /// The limit that `need` does not fit in, if any.
    fn shortfall(&self, room: Amount, need: Amount) -> Option<LimitHit> {
        if need.entries > room.entries {
            Some(LimitHit::Entries)
        } else if need.name_bytes > room.name_bytes {
            Some(LimitHit::NameBytes)
        } else if need.memory > room.memory {
            Some(LimitHit::Memory)
        } else {
            None
        }
    }

    fn release(&self, amount: Amount) {
        let mut reserved = self.lock();

        *reserved = reserved.minus(amount);
    }

    /// Replaces a reservation of `held` by one of exactly `actual`. False, with the reservation given
    /// back, when `actual` does not fit.
    fn settle(&self, held: Amount, actual: Amount) -> bool {
        let mut reserved = self.lock();
        *reserved = reserved.minus(held);

        if self.is_exhausted() {
            return false;
        }

        let room = self.room(*reserved);
        if let Some(hit) = self.shortfall(room, actual) {
            self.note_hit(hit);
            return false;
        }

        *reserved = reserved.plus(actual);
        true
    }
}

/// The room one directory listing has reserved. A source calls [`Lease::admit`] for every entry it is
/// about to return, and stops listing when that says no; the walker then settles the finished listing
/// with `Lease::commit`. A lease dropped without that (the listing failed) gives all its room back.
#[derive(Debug)]
pub struct Lease<'a> {
    budget: &'a EntryBudget,
    /// What was taken from the budget.
    reserved: Amount,
    /// What the source has admitted so far; never more than `reserved`.
    taken: Amount,
}

impl Lease<'_> {
    /// Asks for room for one more entry whose name is `name_len` bytes long, `is_dir` when it is a
    /// directory. When the answer is no the listing is to stop: the scan has hit a limit and ends with
    /// `TooLarge`. The source returns the entries it has so far (the caller discards them) or an error,
    /// whichever is simpler.
    ///
    /// Names are cut at 65 535 bytes in the tree, and so are they here.
    pub fn admit(&mut self, name_len: usize, is_dir: bool) -> bool {
        let name_bytes = name_len.min(usize::from(u16::MAX)) as u64;
        let need = Amount {
            entries: 1,
            name_bytes,
            memory: memory_cost(name_bytes, is_dir),
        };

        let held = self.reserved.minus(self.taken);
        let short =
            held.entries < need.entries || held.name_bytes < need.name_bytes || held.memory < need.memory;
        if short && !self.refill(held, need) {
            return false;
        }

        self.taken = self.taken.plus(need);
        true
    }

    /// Entries admitted so far.
    pub fn admitted(&self) -> u64 {
        self.taken.entries
    }

    /// Takes another batch from the budget: at least what `need` is short of.
    fn refill(&mut self, held: Amount, need: Amount) -> bool {
        if self.budget.is_exhausted() {
            return false;
        }

        // Each kind of room is asked for when it runs out, on its own: asking for all of them with
        // every batch would pile up room nobody has used.
        let min = need.minus(held);
        let batch = |short: u64, batch: u64| if short > 0 { batch.max(short) } else { 0 };
        let want = Amount {
            entries: batch(min.entries, BATCH_ENTRIES),
            name_bytes: batch(min.name_bytes, BATCH_NAME_BYTES),
            memory: batch(min.memory, BATCH_MEMORY_BYTES),
        };

        match self.budget.reserve(want, min) {
            Some(grant) => {
                self.reserved = self.reserved.plus(grant);
                true
            }
            None => false,
        }
    }

    /// Settles the finished listing of `entries` entries, `name_bytes` bytes of names and an estimated
    /// `memory` bytes: what was reserved beyond that goes back, and what a source took without asking is
    /// reserved now. False when that does not fit (the limit is recorded).
    pub(crate) fn commit(mut self, entries: usize, name_bytes: u64, memory: u64) -> bool {
        let actual = Amount {
            entries: entries as u64,
            name_bytes,
            memory,
        };

        let held = std::mem::take(&mut self.reserved);
        self.budget.settle(held, actual)
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if !self.reserved.is_zero() {
            self.budget.release(self.reserved);
        }
    }
}

#[cfg(test)]
mod tests;
