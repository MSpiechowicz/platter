//! How the matches of each rule are cleaned.

/// A program that cleans what it owns, with fixed arguments. The UI never supplies arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool {
    pub(crate) program: &'static str,
    pub(crate) args: &'static [&'static str],
    /// The tool has to run as the administrator.
    pub(crate) needs_elevation: bool,
}

/// Only [`action_for`] makes a `Tool`, so what runs is always one of the fixed commands of this file.
impl Tool {
    pub fn program(&self) -> &'static str {
        self.program
    }
    pub fn args(&self) -> &'static [&'static str] {
        self.args
    }
    pub fn needs_elevation(&self) -> bool {
        self.needs_elevation
    }
}

/// How many versions of each package the package cache tool keeps. Only the pacman cache is cleaned
/// this way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeepVersions {
    One,
    #[default]
    Two,
    Three,
}

impl KeepVersions {
    /// The version count a user chose, or `None` for anything but 1, 2 and 3.
    pub fn from_count(count: u64) -> Option<Self> {
        match count {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            _ => None,
        }
    }

    pub fn count(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
        }
    }

    /// The fixed arguments of the pacman cache tool for this count.
    fn paccache_args(self) -> &'static [&'static str] {
        match self {
            Self::One => &["-rk1"],
            Self::Two => &["-rk2"],
            Self::Three => &["-rk3"],
        }
    }
}

/// What the user chose for a cleanup. They are fixed into the plan when it is made, so the run and the
/// confirmation use the same values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupOptions {
    /// How many versions of each package the pacman cache keeps.
    pub keep_package_versions: KeepVersions,
    /// Tools that need administrator rights may be planned. When off, they are left out of the plan.
    pub ask_for_admin: bool,
}

impl Default for CleanupOptions {
    fn default() -> Self {
        Self {
            keep_package_versions: KeepVersions::default(),
            ask_for_admin: true,
        }
    }
}

/// What cleaning one matched target means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupAction {
    /// Remove the folder or file for good. For things that are rebuilt or downloaded again, where
    /// moving to the trash would free nothing.
    Delete,
    /// Remove everything inside the folder and keep the folder itself (a cache directory that apps
    /// expect to exist).
    EmptyFolder,
    /// Move to the trash, so it can be restored. For the user's own files.
    Trash,
    /// Empty a freedesktop trash folder: everything inside its `files/`, `info/` and `expunged/` goes, and
    /// its size cache; the folders themselves stay, so file managers keep a working trash.
    EmptyTrash,
    /// Let the owning tool clean it (package caches, the journal, Docker).
    RunTool(Tool),
}

/// The action for a rule, or `None` when Platter does not clean what the rule finds (a `keep` rule, or
/// something that needs a tool Platter does not drive yet, such as btrfs snapshots). The options choose
/// among fixed commands, never build one.
pub fn action_for(rule_id: &str, options: &CleanupOptions) -> Option<CleanupAction> {
    use CleanupAction::{Delete, EmptyFolder, EmptyTrash, RunTool, Trash};

    let action = match rule_id {
        "rust-target" | "rust-target-recent" | "node-modules" | "node-modules-recent" | "python-cache" => {
            Delete
        }
        "app-cache" | "npm-cache" | "bun-cache" | "pnpm-store" | "yarn-cache" | "cargo-registry"
        | "gradle-cache" | "maven-repository" | "nuget-packages" | "go-modules" | "steam-shader-cache" => {
            EmptyFolder
        }
        "trash" => EmptyTrash,
        // Owned by root and with no tool of their own: a fixed `find` that deletes what is inside the
        // folder, stays on its filesystem and follows no link.
        "coredumps" => RunTool(Tool {
            program: "find",
            args: &["/var/lib/systemd/coredump", "-xdev", "-mindepth", "1", "-delete"],
            needs_elevation: true,
        }),
        "old-downloads" | "disk-images" | "docker-desktop-vm" => Trash,
        // A folder the user called safe is still the user's own: it goes to the trash, never straight away.
        "user-safe" => Trash,
        "pacman-cache" => RunTool(Tool {
            program: "paccache",
            args: options.keep_package_versions.paccache_args(),
            needs_elevation: true,
        }),
        "apt-cache" => RunTool(Tool {
            program: "apt-get",
            args: &["clean"],
            needs_elevation: true,
        }),
        "dnf-cache" => RunTool(Tool {
            program: "dnf",
            args: &["clean", "packages"],
            needs_elevation: true,
        }),
        "journal" => RunTool(Tool {
            program: "journalctl",
            args: &["--vacuum-time=2weeks"],
            needs_elevation: true,
        }),
        // Always the local daemon: a context in `~/.docker/config.json` could point at a remote one,
        // while the size shown came from the local `/var/lib/docker`.
        "docker-data" => RunTool(Tool {
            program: "docker",
            args: &["--context", "default", "system", "prune", "--force"],
            needs_elevation: false,
        }),
        _ => return None,
    };

    Some(action)
}
