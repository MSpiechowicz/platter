//! Intermediate, per-directory scan result.
//!
//! Workers produce one [`Chunk`] per directory without any shared state; the assembler later turns the
//! chunk tree into the arena in a fixed (breadth-first, name-sorted) order, which is what makes the
//! result independent of thread scheduling.

use crate::model::{NodeKind, SkipReason};

#[derive(Debug, Default)]
pub(crate) struct Chunk {
    /// Names of all entries, concatenated.
    pub names: Vec<u8>,
    /// Entries sorted by name.
    pub entries: Vec<ChunkEntry>,
    /// Hardlink candidates (`nlink > 1`), ordered by entry index.
    pub links: Vec<LinkRef>,
    /// Why mount entries were left out, as `(entry index, reason)` ascending. Only reasons other than
    /// `Unknown` are kept; a mount entry that is not listed is unknown.
    pub skips: Option<Box<[(u32, SkipReason)]>>,
    /// One chunk per entry with `descended`, in entry order.
    pub subdirs: Vec<Chunk>,
    /// The directory was entered but could not be read (listing failed, loop, too deep): the chunk is
    /// empty and the directory's node is flagged as unreadable.
    pub unreadable: bool,
}

impl Chunk {
    /// The chunk of a directory that could not be read.
    pub fn unreadable() -> Self {
        Self {
            unreadable: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChunkEntry {
    pub name_start: u32,
    pub name_len: u16,
    pub kind: NodeKind,
    pub category: crate::model::Category,
    /// A sub-chunk exists for this directory.
    pub descended: bool,
    pub allocated: u64,
    pub apparent: u64,
    pub mtime: i64,
}

impl ChunkEntry {
    pub fn name_range(&self) -> std::ops::Range<usize> {
        let start = self.name_start as usize;
        start..start + self.name_len as usize
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LinkRef {
    pub entry: u32,
    pub dev: u64,
    pub ino: u64,
}
