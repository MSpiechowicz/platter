use serde::{Deserialize, Serialize};

/// What a directory entry is, as reported by a [`DirSource`](crate::ports::DirSource).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    /// Sockets, FIFOs, device nodes and anything else that is not one of the above.
    Other,
}

/// What a node in the scanned tree is. Wire names are lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Dir,
    File,
    Symlink,
    /// A mount point or a folder the user excluded that was deliberately not descended into; size 0.
    Mount,
    Other,
    /// Synthetic node merging the children a LOD query had no room for.
    Collapsed,
}

impl NodeKind {
    /// True for kinds that contribute to the file count (everything but directories and synthetic nodes).
    pub const fn is_file_like(self) -> bool {
        matches!(self, NodeKind::File | NodeKind::Symlink | NodeKind::Other)
    }
}

/// Why a mount point or a folder was left out of the scan (a `mount` node). Wire names are camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// A different filesystem (another disk, a partition, a removable drive) that was not entered
    /// because the scan stays on the root's disk.
    OtherDisk,
    /// A pseudo, virtual, network or FUSE filesystem that is never entered: `/proc`, `/sys`, a share.
    System,
    /// The same filesystem as one the scan covers already, so entering it would count its data twice
    /// (a bind mount, a snapshot mount, a second mount of one disk).
    Duplicate,
    /// A folder the user asked the scan to leave out. It is recorded like a mount that was not entered,
    /// so nothing below it is read, counted, suggested or cleaned.
    Excluded,
    /// The reader cannot tell: the mount is not in the mount table it read at the start of the scan, or
    /// that table was not available.
    Unknown,
}
