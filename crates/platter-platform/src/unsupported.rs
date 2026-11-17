//! Cleanup adapters for an OS that has none yet (neither Linux nor Windows): a probe, a remover and a
//! tool runner that refuse everything, so that cleanup fails closed and nothing on the disk is touched.
//! macOS gets its own adapters in P2.

use std::io;
use std::path::Path;

use platter_core::{
    EntryProbe, EntryRemover, ExpectedEntry, ProbedEntry, RemoveMode, Removed, ToolCommand, ToolFailure,
    ToolRunner,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedEntryProbe;

impl EntryProbe for UnsupportedEntryProbe {
    fn probe(&self, _path: &[u8]) -> io::Result<Vec<ProbedEntry>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedEntryRemover;

impl EntryRemover for UnsupportedEntryRemover {
    fn remove(&self, _path: &[u8], _expected: &ExpectedEntry, _mode: RemoveMode) -> io::Result<Removed> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn trash(&self, _path: &[u8], _expected: &ExpectedEntry, _trash_dir: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn free_bytes(&self, _path: &Path) -> io::Result<u64> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedToolRunner;

impl ToolRunner for UnsupportedToolRunner {
    fn run(&self, _command: &ToolCommand<'_>) -> Result<(), ToolFailure> {
        Err(ToolFailure::NoElevation)
    }
}
