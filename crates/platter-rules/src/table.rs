//! The built-in rules, one entry each. To add a rule, add a line here (and nothing else): the matchers
//! in `rule.rs` and the evaluator do the work. Order only matters when two rules match the very same
//! folder: the earlier one wins.
//!
//! Reasons are templates (see [`Rule`]); they must say only what the matcher can know. Whether a
//! package is still needed, for example, is not in the tree, so the text speaks about what the folder is.

use platter_core::dto::Verdict::{Keep, Review, Safe};
use platter_core::{Category, Os, UserDir, UserVerdict};

use crate::rule::{Files, Matcher, Named, Rule};

const GIB: u64 = 1 << 30;

/// A project that installs its dependencies has one of these next to `package.json`.
const NODE_LOCKFILES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
];
/// Places where tools install packages of their own; deleting their `node_modules` breaks the tool.
const NODE_TOOL_DIRS: &[&str] = &[
    "node_modules",
    ".nvm",
    ".volta",
    ".fnm",
    ".npm-global",
    ".vscode",
    ".vscode-insiders",
    ".vscode-oss",
    ".vscode-server",
    ".cursor",
    ".cursor-server",
    ".windsurf",
];
/// Folders of the system, of the machine's apps and of macOS: nothing below them is a project of the user's.
const SYSTEM_PREFIXES: &[&str] = &["/usr", "/opt", "/Applications", "/Library", "/System"];
/// The same on Windows: below `%SystemRoot%` and these folders at the root of the system drive.
const WINDOWS_SYSTEM_FOLDERS: &[&str] = &["Program Files", "Program Files (x86)", "ProgramData"];

const fn node_modules() -> Named {
    Named::new(&["node_modules"])
        .next_to("package.json")
        .next_to_any(NODE_LOCKFILES)
        .not_inside(NODE_TOOL_DIRS)
        .not_below(SYSTEM_PREFIXES)
        .not_below_drive(WINDOWS_SYSTEM_FOLDERS)
}

/// The user's `keep` rules as one rule: always the first of a set that has user rules, so it wins when
/// another rule matches the same folder. Its folders come from the set's user rules.
pub(crate) const USER_KEEP: Rule = Rule::new(
    "user-keep",
    "Folders you keep",
    Keep,
    Matcher::User(UserVerdict::Keep),
    "Chosen by you: {n:folder} to always keep. Nothing inside is suggested or cleaned.",
);

/// The user's `safe` rules as one rule: always the last of a set that has user rules, so every built-in
/// rule that matches the same folder wins over it.
pub(crate) const USER_SAFE: Rule = Rule::new(
    "user-safe",
    "Folders you marked safe",
    Safe,
    Matcher::User(UserVerdict::Safe),
    "Chosen by you: {n:folder} that can be cleaned. Cleaning only moves a folder to the trash, so it can be put back.",
);

#[rustfmt::skip]
pub(crate) const RULES: &[Rule] = &[
    // Safe: regenerated or downloaded again on demand.
    Rule::new("apt-cache", "APT package archives", Safe, Matcher::Fixed("/var/cache/apt/archives"),
        "Package files APT downloaded for installs and upgrades. APT downloads them again when it needs them.").only_on(Os::Linux),
    Rule::new("dnf-cache", "DNF cache", Safe, Matcher::Fixed("/var/cache/dnf"),
        "Package metadata and downloads that DNF keeps. DNF downloads them again when it needs them.").only_on(Os::Linux),
    Rule::new("rust-target", "Rust build output", Safe,
        Matcher::Named(Named::new(&["target"]).next_to("Cargo.toml").idle(30)),
        "Build output that cargo creates again on the next build. {n:project} with nothing modified in over {days} days."),
    Rule::new("node-modules", "node_modules", Safe,
        Matcher::Named(node_modules().idle(60)),
        "Dependencies that a package manager installs again from package.json. {n:project} with nothing modified in over {days} days."),
    Rule::new("python-cache", "Python caches", Safe,
        Matcher::Named(Named::new(&["__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache"])),
        "Bytecode and tool caches that Python, pytest, mypy and ruff recreate on the next run. Found in {n:folder}."),
    Rule::new("npm-cache", "npm cache", Safe, Matcher::Home(".npm"),
        "Packages npm downloaded before. npm downloads them again when a project needs them."),
    Rule::new("bun-cache", "Bun install cache", Safe, Matcher::Home(".bun/install/cache"),
        "Packages Bun downloaded before. Bun downloads them again when a project needs them."),
    Rule::new("pnpm-store", "pnpm store", Safe, Matcher::Home(".pnpm-store"),
        "The shared package store of pnpm. pnpm downloads again what a project needs and the store no longer has. Files that projects still link to are not freed until those projects are cleaned too."),
    Rule::new("yarn-cache", "Yarn cache", Safe, Matcher::Home(".yarn/cache"),
        "Packages Yarn downloaded before. Yarn downloads them again when a project needs them."),
    Rule::new("cargo-registry", "Cargo registry", Safe, Matcher::Home(".cargo/registry"),
        "Crate sources and indexes cargo downloaded. Cargo fetches the ones a build needs again, which requires a network connection."),
    Rule::new("gradle-cache", "Gradle caches", Safe, Matcher::Home(".gradle/caches"),
        "Dependencies and build caches of Gradle. Gradle downloads or rebuilds what a build needs."),
    Rule::new("nuget-packages", "NuGet packages", Safe, Matcher::Home(".nuget/packages"),
        "Packages NuGet downloaded. NuGet restores the ones a build needs."),
    Rule::new("go-modules", "Go module cache", Safe, Matcher::Home("go/pkg/mod"),
        "Module sources Go downloaded. Go fetches the ones a build needs again, which requires a network connection."),
    Rule::new("trash", "Trash", Safe, Matcher::Home(".local/share/Trash"),
        "Files moved to the trash. Emptying it deletes them for good. A file that is also hard-linked elsewhere keeps its space.").only_on(Os::Linux),
    Rule::new("coredumps", "Core dumps", Safe, Matcher::Fixed("/var/lib/systemd/coredump"),
        "Memory dumps saved when programs crashed. They are mainly useful for debugging those crashes.").only_on(Os::Linux),
    Rule::new("steam-shader-cache", "Steam shader cache", Safe,
        Matcher::Home(".local/share/Steam/steamapps/shadercache"),
        "Compiled shaders Steam keeps for games. Steam rebuilds them the next time a game needs them, which can cause brief stutter.").only_on(Os::Linux),

    // Windows. Cleaning only moves to the Recycle Bin, so what a program still holds open is left alone.
    Rule::new("win-temp", "Temporary files", Safe, Matcher::Home("AppData/Local/Temp"),
        "Files that programs and installers leave in your Temp folder. Programs make them again when they need them. A file that is in use stays where it is.").only_on(Os::Windows),
    Rule::new("win-npm-cache", "npm package cache", Safe, Matcher::Home("AppData/Local/npm-cache"),
        "Packages npm downloaded before. npm downloads them again when a project needs them.").only_on(Os::Windows),
    Rule::new("win-pip-cache", "pip cache", Safe, Matcher::Home("AppData/Local/pip/Cache"),
        "Packages and wheels pip downloaded or built. pip fetches or builds them again when it needs them.").only_on(Os::Windows),
    Rule::new("win-nuget-http-cache", "NuGet HTTP cache", Safe, Matcher::Home("AppData/Local/NuGet/http-cache"),
        "Responses NuGet kept from its servers. NuGet downloads them again when it needs them.").only_on(Os::Windows),
    Rule::new("win-nuget-v3-cache", "NuGet v3 cache", Safe, Matcher::Home("AppData/Local/NuGet/v3-cache"),
        "Responses NuGet kept from its servers, in the folder older versions use. NuGet downloads them again when it needs them.").only_on(Os::Windows),
    Rule::new("win-crash-dumps", "Crash dumps", Safe, Matcher::Home("AppData/Local/CrashDumps"),
        "Memory dumps Windows saved when programs crashed. They are mainly useful for debugging those crashes.").only_on(Os::Windows),

    // macOS. Cleaning only moves to the Trash, so nothing is lost for good. `~/Library/Caches` is one
    // `review` rule that holds the caches of every app, the package managers' included (Homebrew, pip,
    // Yarn): the outermost match wins, so a rule of its own for one of those could never fire.
    Rule::new("mac-xcode-derived-data", "Xcode build data", Safe, Matcher::Home("Library/Developer/Xcode/DerivedData"),
        "Build products and indexes Xcode keeps for your projects. Xcode builds them again the next time you build.").only_on(Os::MacOs),
    Rule::new("mac-simulator-caches", "iOS Simulator caches", Safe, Matcher::Home("Library/Developer/CoreSimulator/Caches"),
        "Caches of the iOS Simulator. The Simulator makes them again when it needs them.").only_on(Os::MacOs),
    Rule::new("mac-pnpm-store", "pnpm store (Library)", Safe, Matcher::Home("Library/pnpm/store"),
        "The shared package store of pnpm. pnpm downloads again what a project needs and the store no longer has. Files that projects still link to are not freed until those projects are cleaned too.").only_on(Os::MacOs),

    // Review: probably removable, but only the owner can tell.
    Rule::new("mac-app-caches", "Library caches", Review, Matcher::Home("Library/Caches"),
        "Caches of your apps and tools, Homebrew and pip among them. Apps make them again, but a large download kept here would have to be downloaded again, and some apps keep things here that cannot be rebuilt.").only_on(Os::MacOs),
    Rule::new("mac-logs", "Application logs", Review, Matcher::Home("Library/Logs"),
        "Logs your apps and the system wrote for you. They are mainly useful for finding out why something went wrong.").only_on(Os::MacOs),
    Rule::new("mac-device-support", "iOS device support files", Review, Matcher::Home("Library/Developer/Xcode/iOS DeviceSupport"),
        "Symbols Xcode copied from iPhones and iPads you connected. Xcode copies them again the next time such a device is connected, which takes a while.").only_on(Os::MacOs),
    Rule::new("mac-docker-vm", "Docker Desktop disk image", Review, Matcher::Home("Library/Containers/com.docker.docker/Data/vms"),
        "The disk image Docker Desktop keeps its images, containers and volumes in. Removing it deletes all of them. Platter does not clean it: it lies in a folder that is protected. Use Docker Desktop to free its space.").only_on(Os::MacOs),
    Rule::new("mac-trash", "Trash folder", Review, Matcher::Home(".Trash"),
        "Files you moved to the Trash, and what Platter moved there. Platter cannot empty it. Empty it in Finder when you are sure you want them gone for good.").only_on(Os::MacOs),
    Rule::new("win-component-store", "Windows component store", Review, Matcher::System("WinSxS"),
        "Older versions of Windows components that Windows keeps for updates and repairs. Windows can clean it itself, which asks for administrator rights and can take a while. Platter never removes files from it directly.").only_on(Os::Windows),
    Rule::new("pacman-cache", "Pacman package cache", Review, Matcher::Fixed("/var/cache/pacman/pkg"),
        "Package files pacman keeps after installing them, including older versions you could downgrade to. Pacman downloads a package again if it needs one that is gone, but an old version may no longer be available.").only_on(Os::Linux),
    Rule::new("journal", "System journal", Review, Matcher::Fixed("/var/log/journal"),
        "System logs kept by systemd-journald, including the current boot. Removing entries loses that history. You can trim them with journalctl instead.").only_on(Os::Linux),
    Rule::new("app-cache", "Application caches", Review, Matcher::Home(".cache"),
        "Rebuilt by the apps that use it. Large downloads kept here, such as models, would have to be downloaded again, and some apps keep things here that cannot be rebuilt.").only_on(Os::Linux),
    Rule::new("maven-repository", "Maven repository", Review, Matcher::Home(".m2/repository"),
        "Artifacts Maven downloaded, and those your own builds installed with mvn install. Downloaded ones are fetched again when a build needs them. Installed ones exist only here."),
    Rule::new("docker-data", "Docker data", Review, Matcher::Fixed("/var/lib/docker"),
        "Images, containers and volumes. Volumes can hold data that exists nowhere else, so look before removing anything.").only_on(Os::Linux),
    Rule::new("docker-desktop-vm", "Docker Desktop virtual machine", Review, Matcher::Home(".docker/desktop/vms"),
        "The disk image Docker Desktop keeps its images, containers and volumes in. Removing it deletes all of them.").only_on(Os::Linux),
    Rule::new("btrfs-snapshots", "btrfs snapshots", Review, Matcher::Fixed("/.snapshots"),
        "Snapshots in the default snapper location. Deleting them removes those rollback points. How much space that frees depends on how much the snapshots share with the live system.").only_on(Os::Linux),
    Rule::new("old-downloads", "Old downloads", Review,
        Matcher::Files(Files::in_home("Downloads").idle(365)),
        "{n:file} in Downloads not modified for {days} days or more. These are personal files, so only you can tell whether you still need them."),
    Rule::new("disk-images", "Virtual machine and disk images", Review,
        Matcher::Files(Files::in_home("").of(Category::DiskImage).at_least(GIB)),
        "{n:image} of {size} or more in your home folder. Often a whole virtual machine, so check before removing one."),
    Rule::new("rust-target-recent", "Rust build output (recent)", Review,
        Matcher::Named(Named::new(&["target"]).next_to("Cargo.toml").recent(30)),
        "{n:project} touched in the last {days} days, so you are probably still working on them. Removing the build output means a full rebuild."),
    Rule::new("node-modules-recent", "node_modules (recent)", Review,
        Matcher::Named(node_modules().recent(60)),
        "{n:project} touched in the last {days} days, so you are probably still working on them. Removing the dependencies means a reinstall."),

    // Keep: shown so the user can see what is deliberately left alone. Nothing inside is suggested.
    Rule::new("steam-library", "Steam library", Keep, Matcher::Home(".local/share/Steam/steamapps/common"),
        "Installed games. Left alone on purpose: they would have to be downloaded again.").only_on(Os::Linux),
    Rule::new("mac-iphone-backups", "iPhone and iPad backups", Keep, Matcher::Home("Library/Application Support/MobileSync/Backup"),
        "Backups of your devices. Left alone on purpose: they may be the only copy of what is on them.").only_on(Os::MacOs),
    Rule::new("mac-mail", "Mail", Keep, Matcher::Home("Library/Mail"),
        "Your mail and its settings. Left alone on purpose. Nothing inside is suggested for cleanup.").only_on(Os::MacOs),
    Rule::new("mac-messages", "Messages", Keep, Matcher::Home("Library/Messages"),
        "Your messages and their attachments. Left alone on purpose. Nothing inside is suggested for cleanup.").only_on(Os::MacOs),
    Rule::new("mac-keychains", "Keychains", Keep, Matcher::Home("Library/Keychains"),
        "Your passwords and keys. Left alone on purpose. Nothing inside is suggested for cleanup.").only_on(Os::MacOs),
    Rule::new("documents", "Documents", Keep, Matcher::UserDir(UserDir::Documents),
        "Your documents folder. Left alone on purpose. Nothing inside is suggested for cleanup."),
    Rule::new("pictures", "Pictures", Keep, Matcher::UserDir(UserDir::Pictures),
        "Your pictures folder. Left alone on purpose. Nothing inside is suggested for cleanup."),
    Rule::new("videos", "Videos", Keep, Matcher::UserDir(UserDir::Videos),
        "Your videos folder. Left alone on purpose. Nothing inside is suggested for cleanup."),
    Rule::new("music", "Music", Keep, Matcher::UserDir(UserDir::Music),
        "Your music folder. Left alone on purpose. Nothing inside is suggested for cleanup."),
    Rule::new("desktop", "Desktop", Keep, Matcher::UserDir(UserDir::Desktop),
        "Your desktop folder. Left alone on purpose. Nothing inside is suggested for cleanup."),
];
