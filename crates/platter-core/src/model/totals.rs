use serde::{Deserialize, Serialize};

use crate::dto::wire_u64;

/// Aggregate numbers of a finished scan. Wire format: camelCase; every number is clamped to
/// [`MAX_WIRE_INTEGER`](crate::dto::MAX_WIRE_INTEGER) when serialized, the struct keeps the exact values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    /// Bytes allocated on disk (blocks), hardlinks counted once.
    #[serde(serialize_with = "wire_u64")]
    pub allocated: u64,
    /// Sum of file lengths, hardlinks counted once.
    #[serde(serialize_with = "wire_u64")]
    pub apparent: u64,
    /// Non-directory entries below the root (files, symlinks, special files).
    #[serde(serialize_with = "wire_u64")]
    pub files: u64,
    /// Directories below the root, including mount points that were not descended.
    #[serde(serialize_with = "wire_u64")]
    pub dirs: u64,
    /// Directories or entries that could not be read.
    #[serde(serialize_with = "wire_u64")]
    pub errors: u64,
}

/// A snapshot of a running scan.
///
/// The counters only grow, and `bytes` is a lower bound of the final [`Totals::allocated`]: entries with
/// more than one hardlink are left out of it, because which link owns the bytes is only decided when
/// the tree is assembled. So the running number never exceeds the final one, but it falls short of it
/// on trees full of hardlinks (a package store, git objects) and then jumps when the scan finishes.
/// The `finished` event carries the exact [`Totals`]; those are what to show once the scan is done.
/// `files`, `dirs` and `errors` are exact counts of what has been read so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub files: u64,
    pub dirs: u64,
    /// Allocated bytes seen so far, hardlinked files excluded (see above).
    pub bytes: u64,
    pub errors: u64,
    /// Directory being read when the snapshot was taken (lossy UTF-8).
    pub current_path: String,
}
