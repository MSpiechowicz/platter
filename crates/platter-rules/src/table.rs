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
const SYSTEM_PREFIXES: &[&str] = &["/usr", "/opt"];

const fn node_modules() -> Named {
    Named::new(&["node_modules"])
        .next_to("package.json")
        .next_to_any(NODE_LOCKFILES)
        .not_inside(NODE_TOOL_DIRS)
        .not_below(SYSTEM_PREFIXES)
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
        "Files moved to the trash. Emptying it deletes them for good; a file that is also hard-linked elsewhere keeps its space."),
    Rule::new("coredumps", "Core dumps", Safe, Matcher::Fixed("/var/lib/systemd/coredump"),
        "Memory dumps saved when programs crashed. They are mainly useful for debugging those crashes.").only_on(Os::Linux),
    Rule::new("steam-shader-cache", "Steam shader cache", Safe,
        Matcher::Home(".local/share/Steam/steamapps/shadercache"),
        "Compiled shaders Steam keeps for games. Steam rebuilds them the next time a game needs them, which can cause brief stutter."),

    // Review: probably removable, but only the owner can tell.
    Rule::new("pacman-cache", "Pacman package cache", Review, Matcher::Fixed("/var/cache/pacman/pkg"),
        "Package files pacman keeps after installing them, including older versions you could downgrade to. Pacman downloads a package again if it needs one that is gone, but an old version may no longer be available.").only_on(Os::Linux),
    Rule::new("journal", "System journal", Review, Matcher::Fixed("/var/log/journal"),
        "System logs kept by systemd-journald, including the current boot. Removing entries loses that history; journalctl can trim them for you.").only_on(Os::Linux),
    Rule::new("app-cache", "Application caches", Review, Matcher::Home(".cache"),
        "Rebuilt by the apps that use it. Large downloads kept here, such as models, would have to be downloaded again, and some apps keep things here that cannot be rebuilt."),
    Rule::new("maven-repository", "Maven repository", Review, Matcher::Home(".m2/repository"),
        "Artifacts Maven downloaded, and those your own builds installed with mvn install. Downloaded ones are fetched again when a build needs them; installed ones exist only here."),
    Rule::new("docker-data", "Docker data", Review, Matcher::Fixed("/var/lib/docker"),
        "Images, containers and volumes. Volumes can hold data that exists nowhere else, so look before removing anything.").only_on(Os::Linux),
    Rule::new("docker-desktop-vm", "Docker Desktop virtual machine", Review, Matcher::Home(".docker/desktop/vms"),
        "The disk image Docker Desktop keeps its images, containers and volumes in. Removing it deletes all of them."),
    Rule::new("btrfs-snapshots", "btrfs snapshots", Review, Matcher::Fixed("/.snapshots"),
        "Snapshots in the default snapper location. Deleting them removes those rollback points; how much space that frees depends on how much the snapshots share with the live system.").only_on(Os::Linux),
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
        "Installed games. Left alone on purpose: they would have to be downloaded again."),
    Rule::new("documents", "Documents", Keep, Matcher::UserDir(UserDir::Documents),
        "Your documents folder. Left alone on purpose; nothing inside is suggested for cleanup."),
    Rule::new("pictures", "Pictures", Keep, Matcher::UserDir(UserDir::Pictures),
        "Your pictures folder. Left alone on purpose; nothing inside is suggested for cleanup."),
    Rule::new("videos", "Videos", Keep, Matcher::UserDir(UserDir::Videos),
        "Your videos folder. Left alone on purpose; nothing inside is suggested for cleanup."),
    Rule::new("music", "Music", Keep, Matcher::UserDir(UserDir::Music),
        "Your music folder. Left alone on purpose; nothing inside is suggested for cleanup."),
    Rule::new("desktop", "Desktop", Keep, Matcher::UserDir(UserDir::Desktop),
        "Your desktop folder. Left alone on purpose; nothing inside is suggested for cleanup."),
];
