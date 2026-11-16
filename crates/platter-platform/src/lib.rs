//! OS adapters for Platter: implementations of the `platter-core` ports.
//!
//! - [`environment`]: the home directory, clock and OS that a finished scan is judged against.
//! - [`generic`]: `DirSource` on plain `std::fs`; works everywhere.
//! - `linux` (Linux only): the fast `DirSource` plus default exclusions.
//! - [`volumes`]: `VolumeProvider` on `sysinfo`.
//! - [`localtime`]: the local time zone's UTC offset, from the system's TZif files.
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
pub mod localtime;
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

/// The probe that looks at a path before cleanup acts on it.
#[cfg(target_os = "linux")]
pub type DefaultEntryProbe = linux::probe::TimedEntryProbe;

/// What cleanup acts on the disk through.
#[cfg(target_os = "linux")]
pub type DefaultEntryRemover = linux::remove::LinuxEntryRemover;

/// What runs the tools that own a cache.
#[cfg(target_os = "linux")]
pub type DefaultToolRunner = linux::tools::LinuxToolRunner;

/// Everything an app needs from the OS, in one value.
#[derive(Debug, Default)]
pub struct Platform {
    /// Pass to `platter_core::scan`.
    pub dir_source: DefaultDirSource,
    /// Implements `platter_core::VolumeProvider`.
    pub volumes: SystemVolumes,
    /// Implements `platter_core::EntryProbe`.
    #[cfg(target_os = "linux")]
    pub entry_probe: DefaultEntryProbe,
    /// Implements `platter_core::EntryRemover`.
    #[cfg(target_os = "linux")]
    pub entry_remover: DefaultEntryRemover,
    /// Implements `platter_core::ToolRunner`.
    #[cfg(target_os = "linux")]
    pub tool_runner: DefaultToolRunner,
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

    /// [`Platform::environment`] as seen from a scan of `scan_root` (see
    /// [`environment::environment_for_scan`]): use this for a scan's suggestions and cleanup.
    pub fn environment_for(&self, scan_root: &std::path::Path) -> platter_core::Environment {
        environment::environment_for_scan(&current_environment(), scan_root)
    }
}

/// Reads at most `cap` bytes of the regular file at `path`. A FIFO, device or folder is refused, and on
/// Linux the open itself never blocks (a FIFO in `TZ` or `XDG_CONFIG_HOME` cannot hang the caller).
pub(crate) fn read_regular_file(path: &std::path::Path, cap: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;

    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }

    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }

    let mut bytes = Vec::new();
    file.take(cap).read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(all(test, target_os = "linux"))]
mod read_tests {
    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("zone");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
        assert!(made.success());

        let error = super::read_regular_file(&fifo, 1024).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
