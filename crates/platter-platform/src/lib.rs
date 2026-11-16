//! OS adapters for Platter: implementations of the `platter-core` ports.
//!
//! - [`environment`]: the home directory, clock and OS that a finished scan is judged against.
//! - [`generic`]: `DirSource` on plain `std::fs`; works everywhere.
//! - `linux` (Linux only): the fast `DirSource` plus default exclusions.
//! - [`volumes`]: `VolumeProvider` on `sysinfo`.
//! - [`memory`]: a scan memory budget that fits the memory the machine has.
//! - [`paths`]: the exact OS path of a scanned node, including names that are not valid UTF-8.
//! - [`root`]: resolving the folder a scan starts from, with the identity to pin it to.
//! - `testing` (feature `testing`): helpers for other crates' tests that need OS-specific code.
//!
//! All `cfg(target_os)` code lives in this crate. Supporting another OS means adding a faster
//! `DirSource` module here and pointing [`DefaultDirSource`] at it; `platter-core`, the CLI and the IPC
//! layer do not change.

pub mod environment;
pub mod generic;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod memory;
pub mod paths;
mod pseudo;
pub mod root;
#[cfg(feature = "testing")]
pub mod testing;
pub mod volumes;

pub use environment::current_environment;
pub use generic::GenericDirSource;
pub use memory::suggested_memory_budget;
pub use volumes::SystemVolumes;

/// Raises the soft limit on open file descriptors to the hard limit, so deep scans on many threads do
/// not run short of descriptors. Call once at startup. Returns the soft limit now in force (`None`:
/// unlimited). A no-op returning `Ok(None)` on platforms where the limit is not an issue.
pub fn raise_fd_limit() -> std::io::Result<Option<u64>> {
    #[cfg(target_os = "linux")]
    {
        linux::raise_fd_limit()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(None)
    }
}

/// The best directory reader for the OS this binary runs on.
#[cfg(target_os = "linux")]
pub type DefaultDirSource = linux::LinuxDirSource;

/// The best directory reader for the OS this binary runs on.
#[cfg(not(target_os = "linux"))]
pub type DefaultDirSource = generic::GenericDirSource;

/// Everything an app needs from the OS, in one value.
#[derive(Debug, Default)]
pub struct Platform {
    /// Pass to `platter_core::scan`.
    pub dir_source: DefaultDirSource,
    /// Implements `platter_core::VolumeProvider`.
    pub volumes: SystemVolumes,
}

impl Platform {
    /// The adapters for the current OS.
    pub fn current() -> Self {
        Self::default()
    }

    /// The home directory, clock and OS right now, for judging a finished scan.
    pub fn environment(&self) -> platter_core::Environment {
        current_environment()
    }
}
