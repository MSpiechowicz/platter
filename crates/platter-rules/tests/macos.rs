//! The rules on a macOS environment. The fake disk has `/Users/u` paths and the environment says `os`,
//! so these run on any system: rules fire on mac paths only on macOS, and rule matching stays case
//! sensitive there (only the cleanup guard folds case).

mod common;

use std::path::PathBuf;

use common::*;
use platter_core::dto::Verdict;
use platter_core::testing::FakeFs;
use platter_rules::{Environment, Os, RuleSet, Suggestions, evaluate};

fn mac() -> Environment {
    Environment {
        os: Os::MacOs,
        ..env_with_home(Some("/Users/u"))
    }
}

fn linux_on_the_same_disk() -> Environment {
    env_with_home(Some("/Users/u"))
}

fn evaluated(fs: &FakeFs, env: &Environment) -> Suggestions {
    evaluate(&scan_at(fs, "/", false), env, &RuleSet::builtin())
}

fn verdicts(found: &Suggestions) -> Vec<(String, Verdict)> {
    found
        .suggestions
        .iter()
        .map(|s| (s.id.clone(), s.verdict))
        .collect()
}

fn verdict(found: &Suggestions, id: &str) -> Option<Verdict> {
    get(found, id).map(|s| s.verdict)
}

fn library() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file(
        "/Users/u/Library/Developer/Xcode/DerivedData/App-abc/Build/x",
        9_000,
    )
    .file("/Users/u/Library/Developer/CoreSimulator/Caches/dyld/y", 8_000)
    .file("/Users/u/Library/pnpm/store/v3/files/z", 7_000)
    .file("/Users/u/Library/Caches/com.example.app/blob", 6_000)
    .file("/Users/u/Library/Logs/DiagnosticReports/a.ips", 5_000)
    .file(
        "/Users/u/Library/Developer/Xcode/iOS DeviceSupport/17.2/Symbols/s",
        4_000,
    )
    .file(
        "/Users/u/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw",
        3_000,
    )
    .file("/Users/u/.Trash/old.txt", 2_000);
    fs
}

#[test]
fn the_rules_of_macos_are_offered_there_with_their_verdicts() {
    let found = evaluated(&library(), &mac());

    for id in ["mac-xcode-derived-data", "mac-simulator-caches", "mac-pnpm-store"] {
        assert_eq!(verdict(&found, id), Some(Verdict::Safe), "{id}");
    }
    for id in [
        "mac-app-caches",
        "mac-logs",
        "mac-device-support",
        "mac-docker-vm",
        "mac-trash",
    ] {
        assert_eq!(verdict(&found, id), Some(Verdict::Review), "{id}");
    }
    assert_eq!(
        get(&found, "mac-xcode-derived-data").unwrap().path,
        "~/Library/Developer/Xcode/DerivedData"
    );
    assert_eq!(
        get(&found, "mac-device-support").unwrap().path,
        "~/Library/Developer/Xcode/iOS DeviceSupport"
    );
}

#[test]
fn the_rules_of_macos_are_not_offered_on_linux_and_the_rules_of_linux_not_on_macos() {
    let mut fs = library();
    fs.file("/Users/u/.local/share/Trash/files/old", 600)
        .file("/Users/u/.cache/app/blob", 700)
        .file("/Users/u/.docker/desktop/vms/disk.img", 800)
        .file("/Users/u/.local/share/Steam/steamapps/shadercache/1/c", 900)
        .file("/var/cache/pacman/pkg/a", 100)
        .file("/var/cache/apt/archives/a.deb", 100)
        .file("/var/cache/dnf/a", 100)
        .file("/var/log/journal/j", 100)
        .file("/var/lib/systemd/coredump/c", 100)
        .file("/var/lib/docker/x", 100)
        .file("/.snapshots/1/x", 100);

    let on_mac = verdicts(&evaluated(&fs, &mac()));
    let on_linux = verdicts(&evaluated(&fs, &linux_on_the_same_disk()));

    for id in [
        "trash",
        "app-cache",
        "docker-desktop-vm",
        "steam-shader-cache",
        "pacman-cache",
        "apt-cache",
        "dnf-cache",
        "journal",
        "coredumps",
        "docker-data",
        "btrfs-snapshots",
    ] {
        assert!(on_mac.iter().all(|(known, _)| known != id), "{id} on macOS");
        assert!(on_linux.iter().any(|(known, _)| known == id), "{id} on Linux");
    }
    assert!(
        on_linux.iter().all(|(known, _)| !known.starts_with("mac-")),
        "{on_linux:?}"
    );
}

#[test]
fn every_rule_named_for_macos_is_restricted_to_it() {
    let set = RuleSet::builtin();

    let named: Vec<_> = set
        .rules()
        .iter()
        .filter(|rule| rule.id.starts_with("mac-"))
        .collect();

    assert!(!named.is_empty());
    for rule in named {
        assert_eq!(rule.os, Some(Os::MacOs), "{}", rule.id);
    }
}

#[test]
fn the_cross_platform_rules_work_below_a_mac_home() {
    let mut fs = FakeFs::new();
    fs.file("/Users/u/.npm/_cacache/x", 3_000)
        .file("/Users/u/.cargo/registry/src/y", 2_000)
        .file("/Users/u/go/pkg/mod/z", 1_000);
    aged_file(&mut fs, "/Users/u/Downloads/old.iso", 5_000, 400);
    aged_file(&mut fs, "/Users/u/code/rs/Cargo.toml", 1, 400);
    aged_file(&mut fs, "/Users/u/code/rs/target/x", 4_000, 400);

    let found = evaluated(&fs, &mac());

    assert_eq!(verdict(&found, "npm-cache"), Some(Verdict::Safe));
    assert_eq!(verdict(&found, "cargo-registry"), Some(Verdict::Safe));
    assert_eq!(verdict(&found, "go-modules"), Some(Verdict::Safe));
    assert_eq!(verdict(&found, "rust-target"), Some(Verdict::Safe));
    assert_eq!(verdict(&found, "old-downloads"), Some(Verdict::Review));
}

#[test]
fn the_caches_of_package_managers_belong_to_the_one_review_folder_that_holds_them() {
    let mut fs = FakeFs::new();
    fs.file("/Users/u/Library/Caches/Homebrew/downloads/a", 5_000)
        .file("/Users/u/Library/Caches/pip/http/b", 4_000)
        .file("/Users/u/Library/Caches/Yarn/v6/c", 3_000);

    let found = evaluated(&fs, &mac());

    assert_eq!(verdicts(&found), [("mac-app-caches".to_owned(), Verdict::Review)]);
    assert_eq!(get(&found, "mac-app-caches").unwrap().path, "~/Library/Caches");
}

#[test]
fn what_macos_keeps_is_kept_and_nothing_inside_is_suggested() {
    let mut fs = FakeFs::new();
    fs.file(
        "/Users/u/Library/Application Support/MobileSync/Backup/ab12/Manifest.db",
        9_000,
    )
    .file("/Users/u/Library/Mail/V10/box/m.emlx", 8_000)
    .file("/Users/u/Library/Messages/chat.db", 7_000)
    .file("/Users/u/Library/Keychains/login.keychain-db", 6_000);
    old_project(&mut fs, "/Users/u/Movies/edit");
    old_project(&mut fs, "/Users/u/Documents/work");
    old_project(&mut fs, "/Users/u/Desktop/tmp");
    old_project(&mut fs, "/Users/u/Pictures/p");
    old_project(&mut fs, "/Users/u/Music/m");

    let found = evaluated(&fs, &mac());

    for id in [
        "mac-iphone-backups",
        "mac-mail",
        "mac-messages",
        "mac-keychains",
        "videos",
        "documents",
        "desktop",
        "pictures",
        "music",
    ] {
        assert_eq!(verdict(&found, id), Some(Verdict::Keep), "{id}");
    }
    assert!(
        found.suggestions.iter().all(|s| s.verdict == Verdict::Keep),
        "{:?}",
        verdicts(&found)
    );
    assert_eq!(get(&found, "videos").unwrap().path, "~/Movies");
}

fn old_project(fs: &mut FakeFs, dir: &str) {
    fs.file(&format!("{dir}/Cargo.toml"), 1);
    aged_file(fs, &format!("{dir}/target/x"), 100, 400);
}

#[test]
fn the_videos_folder_is_found_as_movies_and_a_configured_place_wins() {
    let mut fs = FakeFs::new();
    old_project(&mut fs, "/Users/u/Movies/edit");
    old_project(&mut fs, "/Volumes/Media/Films/edit");

    let by_default = evaluated(&fs, &mac());
    assert_eq!(get(&by_default, "videos").unwrap().path, "~/Movies");
    assert_eq!(get(&by_default, "rust-target").map(|s| s.items), Some(1));

    let mut configured = mac();
    configured.user_dirs.push((
        platter_core::UserDir::Videos,
        PathBuf::from("/Volumes/Media/Films"),
    ));
    let found = evaluated(&fs, &configured);

    assert_eq!(get(&found, "videos").unwrap().path, "/Volumes/Media/Films");
    assert_eq!(
        get(&found, "rust-target").map(|s| s.items),
        Some(1),
        "~/Movies is an ordinary folder once the desktop says the videos are elsewhere"
    );

    // On Linux the same disk has no Movies folder of any role: `~/Videos` is the name.
    let linux = evaluated(&fs, &linux_on_the_same_disk());
    assert!(
        linux.suggestions.iter().all(|s| s.id != "videos"),
        "{:?}",
        verdicts(&linux)
    );
}

#[test]
fn rule_matching_stays_case_sensitive_on_macos() {
    let mut fs = FakeFs::new();
    fs.file("/Users/u/library/caches/app/blob", 6_000)
        .file("/Users/u/LIBRARY/Logs/a.ips", 5_000);

    let found = evaluated(&fs, &mac());

    assert!(found.suggestions.is_empty(), "{:?}", verdicts(&found));
}

#[test]
fn dependencies_below_the_applications_folder_are_not_a_project() {
    let mut fs = FakeFs::new();
    for project in [
        "/Applications/Tool.app/Contents/Resources/app",
        "/Users/u/code/web",
    ] {
        aged_file(&mut fs, &format!("{project}/package.json"), 1, 100);
        aged_file(&mut fs, &format!("{project}/package-lock.json"), 1, 100);
        aged_file(&mut fs, &format!("{project}/node_modules/dep/i.js"), 1_500, 100);
    }

    let found = evaluated(&fs, &mac());

    assert_eq!(get(&found, "node-modules").map(|s| s.items), Some(1));
}
