use std::path::PathBuf;

use crate::model::NodeId;

/// Why a scan did not produce a tree.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// The cancel flag was raised; no tree is produced.
    #[error("scan cancelled")]
    Cancelled,

    #[error("cannot open {}: {source}", path.display())]
    Root {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),

    /// The folder is bigger than the scan may hold: more entries than the entry cap or the `u32` arena
    /// allows, or more memory than the scan's budget. `limit` is the one that was hit.
    #[error("{limit}")]
    TooLarge { limit: ScanLimit },

    #[error("cannot start the scan thread pool: {0}")]
    ThreadPool(String),
}

/// Which limit ended a scan with [`ScanError::TooLarge`], with its value. Its text is the plain-language
/// message shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanLimit {
    /// The most entries (files and directories) a scan may collect.
    Entries(u64),
    /// The most memory, in bytes, a scan may use (by the estimate of [`crate::budget`]; names count
    /// towards it).
    Memory(u64),
}

/// A memory limit below this (half of about 256 MiB of free memory) gets the "little free memory"
/// wording: the machine, not the folder, is what is small.
const LOW_MEMORY_LIMIT_BYTES: u64 = 128 << 20;

impl std::fmt::Display for ScanLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            ScanLimit::Entries(limit) => write!(
                f,
                "This folder has more entries than Platter can hold in memory (limit {limit} entries). \
                 Scan a smaller folder."
            ),
            ScanLimit::Memory(limit) if limit < LOW_MEMORY_LIMIT_BYTES => write!(
                f,
                "There is very little free memory right now, so Platter may use only {} for a scan, \
                 which fits only a small folder. Close other programs or scan a smaller folder.",
                size_text(limit)
            ),
            ScanLimit::Memory(limit) => write!(
                f,
                "This folder needs more memory than Platter may use for a scan (limit {}, half of the \
                 memory that is free). Scan a smaller folder.",
                size_text(limit)
            ),
        }
    }
}

/// `1073741824` is `1 GiB`, `1610612736` is `1.5 GiB`.
fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    if value.fract() == 0.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Why a query failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    #[error("node {0} does not exist")]
    NotFound(NodeId),
}
