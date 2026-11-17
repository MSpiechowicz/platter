//! OS adapters for Platter: implementations of the `platter-core` ports.
//!
//! - [`app_dirs`]: the app's config and data folders, and private files in them.
//! - [`cpu`]: physical cores, for sizing a thread pool.
//! - [`environment`]: the home directory, clock and OS that a finished scan is judged against.
//! - [`generic`]: `DirSource` on plain `std::fs`; works everywhere.
//! - `linux` (Linux only): the fast `DirSource` plus default exclusions.
//! - `windows` (Windows only): the `DirSource`, probe, remover and tool runner, and the file identity they share.
//! - `unsupported` (neither): cleanup adapters that refuse everything.
//! - [`volumes`]: `VolumeProvider` on `sysinfo`.
//! - [`localtime`]: the local time zone's UTC offset, from the system's TZif files.
//! - [`memory`]: a scan memory budget that fits the memory the machine has.
//! - [`paths`]: the exact OS path of a scanned node, including names that are not valid UTF-8.
//! - [`user_rules`]: reading and writing the user's rules file, which is never overwritten when broken.
//! - [`root`]: resolving the folder a scan starts from, with the identity to pin it to.
//! - [`runtime`] (Unix and Windows): starting the advisor's helper process and taking its descriptors in it.
//! - `testing` (feature `testing`): helpers for other crates' tests that need OS-specific code.
//!
//! All `cfg(target_os)` code lives in this crate. Supporting another OS means adding a module here
//! with its `DirSource`, probe, remover and tool runner and pointing the `Default*` aliases at them;
//! `platter-core`, the CLI and the IPC layer do not change.

pub mod app_dirs;
pub mod cpu;
pub mod environment;
pub mod generic;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod localtime;
pub mod memory;
pub mod paths;
mod pseudo;
pub mod root;
#[cfg(any(unix, windows))]
pub mod runtime;
#[cfg(feature = "testing")]
pub mod testing;
#[cfg(not(any(target_os = "linux", windows)))]
pub mod unsupported;
pub mod user_rules;
pub mod volumes;
#[cfg(windows)]
pub mod windows;

pub use app_dirs::{config_dir, create_private_dir, data_dir, free_space, lock_exclusive, open_private_file};
pub use cpu::physical_cores;
pub use environment::current_environment;
pub use generic::GenericDirSource;
pub use memory::{
    available_memory_bytes, resident_bytes, resident_bytes_of, suggested_memory_budget, total_memory_bytes,
};
#[cfg(windows)]
pub use runtime::SharedChild;
#[cfg(any(unix, windows))]
pub use runtime::{
    Exit, HelperEnd, RuntimeChild, RuntimeMode, advisor_runtime_child, describe_exit, exit_text,
    runtime_environment, runtime_program, spawn_advisor_runtime,
};
pub use volumes::SystemVolumes;

/// The only argument of the advisor's helper process: the binary started with exactly this runs the
/// model runtime and nothing else.
pub const ADVISOR_RUNTIME_ARG: &str = "--advisor-runtime";

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

/// Whether this OS can hold a model file open and hand it to a library by path (what the advisor needs
/// to load only what it has checked). Linux (`/proc/self/fd/N`) and Windows (a share mode that keeps the
/// name from changing, see `windows::pinned`) can.
pub const fn model_runtime_supported() -> bool {
    #[cfg(windows)]
    {
        windows::pinned::model_runtime_supported()
    }
    #[cfg(not(windows))]
    {
        cfg!(target_os = "linux")
    }
}

#[cfg(target_os = "linux")]
pub use linux::pinned::{Pin, open_pinned};
#[cfg(windows)]
pub use windows::pinned::{Pin, open_pinned};

/// Where nothing can be pinned (an OS with no adapters yet): a pin that names no path, so a model is never
/// loaded.
#[cfg(not(any(target_os = "linux", windows)))]
#[derive(Debug)]
pub struct Pin(());

#[cfg(not(any(target_os = "linux", windows)))]
impl Pin {
    /// The pin of a file that was handed over already open.
    pub fn inherited() -> Self {
        Self(())
    }

    /// No path names the file here.
    pub fn path(&self, _file: &std::fs::File) -> Option<std::path::PathBuf> {
        None
    }
}

/// Fails with `Unsupported`: nothing can be pinned here.
#[cfg(not(any(target_os = "linux", windows)))]
pub fn open_pinned(path: &std::path::Path) -> std::io::Result<(std::fs::File, Pin)> {
    let _ = path;
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

/// Opens `path` for reading, never following a link at the path, as a regular file (a model file that
/// is checked and then handed to a library). Fails with `Unsupported` where this cannot be done safely.
pub fn open_nofollow(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        linux::pinned::open_nofollow(path)
    }
    #[cfg(windows)]
    {
        windows::pinned::open_nofollow(path)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = path;
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

/// A path that names the file `file` holds open, however its name changes afterwards. `None` where the
/// system has no such path (Windows: the file is held by [`Pin`] and the path is the one it was opened by).
pub fn pinned_path(file: &std::fs::File) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        Some(linux::pinned::pinned_path(file))
    }
    #[cfg(windows)]
    {
        windows::pinned::pinned_path(file)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = file;
        None
    }
}

/// The best directory reader for the OS this binary runs on.
#[cfg(target_os = "linux")]
pub type DefaultDirSource = linux::LinuxDirSource;

/// The best directory reader for the OS this binary runs on.
#[cfg(windows)]
pub type DefaultDirSource = windows::WindowsDirSource;

/// The best directory reader for the OS this binary runs on.
#[cfg(not(any(target_os = "linux", windows)))]
pub type DefaultDirSource = generic::GenericDirSource;

/// The probe that looks at a path before cleanup acts on it.
#[cfg(target_os = "linux")]
pub type DefaultEntryProbe = linux::probe::TimedEntryProbe;

/// The probe that looks at a path before cleanup acts on it.
#[cfg(windows)]
pub type DefaultEntryProbe = windows::WindowsEntryProbe;

/// The probe that looks at a path before cleanup acts on it.
#[cfg(not(any(target_os = "linux", windows)))]
pub type DefaultEntryProbe = unsupported::UnsupportedEntryProbe;

/// What cleanup acts on the disk through.
#[cfg(target_os = "linux")]
pub type DefaultEntryRemover = linux::remove::LinuxEntryRemover;

/// What cleanup acts on the disk through.
#[cfg(windows)]
pub type DefaultEntryRemover = windows::WindowsEntryRemover;

/// What cleanup acts on the disk through.
#[cfg(not(any(target_os = "linux", windows)))]
pub type DefaultEntryRemover = unsupported::UnsupportedEntryRemover;

/// What runs the tools that own a cache.
#[cfg(target_os = "linux")]
pub type DefaultToolRunner = linux::tools::LinuxToolRunner;

/// What runs the tools that own a cache.
#[cfg(windows)]
pub type DefaultToolRunner = windows::WindowsToolRunner;

/// What runs the tools that own a cache.
#[cfg(not(any(target_os = "linux", windows)))]
pub type DefaultToolRunner = unsupported::UnsupportedToolRunner;

/// Everything an app needs from the OS, in one value.
#[derive(Debug, Default)]
pub struct Platform {
    /// Pass to `platter_core::scan`.
    pub dir_source: DefaultDirSource,
    /// Implements `platter_core::VolumeProvider`.
    pub volumes: SystemVolumes,
    /// Implements `platter_core::EntryProbe`.
    pub entry_probe: DefaultEntryProbe,
    /// Implements `platter_core::EntryRemover`.
    pub entry_remover: DefaultEntryRemover,
    /// Implements `platter_core::ToolRunner`.
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
