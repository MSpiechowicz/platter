//! The Windows adapters (P1). Every module here exists from package W0, so the packages that fill them
//! (W1 to W3) never edit `lib.rs` or this file: each fills the files it owns.
//!
//! | Module | Filled by | What it is |
//! |---|---|---|
//! | [`identity`] | W0 (done) | the file identity every other module compares: volume serial, file index, link count, creation time |
//! | [`reader`] | W1 | [`WindowsDirSource`]: the scan's directory reader |
//! | [`exclusions`] | W1 | default exclusions (`pagefile.sys`, `System Volume Information`, ...) |
//! | [`volumes`] | W1 | volume labels: drive letter plus volume label |
//! | [`probe`] | W2 | [`WindowsEntryProbe`]: looks at a path component by component |
//! | [`remove`] | W2 | [`WindowsEntryRemover`]: moves a target to the Recycle Bin, identity checked |
//! | [`recycle`] | W2 | finding and restoring items in the Recycle Bin |
//! | [`tools`] | W2 | [`WindowsToolRunner`]: the tools that own a cache, elevated through UAC |
//! | [`runtime`] | W3 | the advisor's helper process: pipes, Job Object, exit codes |
//! | [`pinned`] | W3 | opening the model file so that what was checked is what is loaded |
//!
//! Paths on Windows are `std::path::Path`s of the form `X:\a\b` (never `\\?\`, see `dunce`). Safe wrapper
//! crates only: `unsafe` is forbidden in this workspace.

pub mod exclusions;
pub mod identity;
pub mod pinned;
pub mod probe;
pub mod reader;
pub mod recycle;
pub mod remove;
pub mod runtime;
pub mod tools;
pub mod volumes;

pub use probe::WindowsEntryProbe;
pub use reader::WindowsDirSource;
pub use remove::WindowsEntryRemover;
pub use tools::WindowsToolRunner;
