//! Test support: an in-memory [`DirSource`](crate::ports::DirSource) and small scan helpers.

mod fake_fs;

pub use fake_fs::FakeFs;

use std::path::Path;

use crate::model::Tree;
use crate::scan::{CancelToken, ScanConfig, scan};

/// Scan time used by [`scan_fake`], so age buckets are reproducible.
pub const NOW: i64 = 1_700_000_000;

/// Scans the whole fake filesystem (`/`) with a fixed clock, no cancellation and no progress sink.
pub fn scan_fake(fs: &FakeFs, one_file_system: bool) -> Tree {
    let config = ScanConfig {
        one_file_system,
        now: Some(NOW),
        ..ScanConfig::default()
    };

    scan(fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).expect("scan of the fake succeeds")
}
