//! The macOS protected places, path syntax and actions, on any system: the style comes from the
//! environment, not from the OS the tests run on.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use platter_cleanup::{
    CleanupAction, CleanupOptions, Disk, PathStyle, Protected, TargetOutcome, action_for, execute,
    plan_cleanup,
};
use platter_core::ports::{ChildTrashed, TrashedChild, TrashedChildren};
use platter_core::testing::{FakeFs, NOW};
use platter_core::{
    CancelToken, EntryRemover, Environment, ExpectedEntry, Os, RemoveMode, Removed, ScanConfig, Tree,
    UserDir, scan,
};
use platter_rules::RuleSet;

const DAY: i64 = 86_400;

fn mac() -> Environment {
    Environment {
        home: Some(PathBuf::from("/Users/me")),
        user_dirs: Vec::new(),
        now: NOW,
        os: Os::MacOs,
        system_root: None,
    }
}

fn refused(protected: &Protected, path: &str) -> bool {
    protected.refuses(path.as_bytes()).is_some()
}

#[test]
fn what_macos_keeps_for_itself_and_the_user_is_protected_whatever_the_case() {
    let protected = Protected::for_environment(&mac()).unwrap();

    for path in [
        "/",
        "/System",
        "/System/Library/CoreServices",
        "/system/library",
        "/Library",
        "/Library/Caches/com.apple.x",
        "/library/preferences",
        "/Applications",
        "/Applications/Safari.app",
        "/applications/safari.app/Contents",
        "/Users",
        "/users",
        "/Users/me",
        "/users/ME",
        "/Users/me/Desktop",
        "/Users/me/desktop/notes",
        "/Users/me/Documents/thesis.pages",
        "/Users/me/Pictures/2026/a.heic",
        "/Users/me/Movies/edit.mov",
        "/Users/me/Music/Music/Media",
        "/Volumes",
        "/private",
        "/private/var",
        "/private/tmp",
        "/private/etc/hosts",
        "/private/var/db/dslocal",
        "/private/var/root/.zshrc",
        "/private/var/vm/sleepimage",
        "/var",
        "/var/db",
        "/var/root",
        "/tmp",
        "/etc/hosts",
        "/usr/bin/env",
        "/usr/local/bin/brew",
        "/bin/sh",
        "/sbin/mount",
        "/opt/homebrew/bin/brew",
        "/cores",
        "/cores/core.123",
    ] {
        assert!(refused(&protected, path), "{path}");
    }
}

#[test]
fn the_places_of_the_library_that_are_not_the_users_to_clean_are_protected_whatever_the_case() {
    let protected = Protected::for_environment(&mac()).unwrap();

    for place in [
        ".ssh",
        ".gnupg",
        ".config",
        ".aws",
        ".Trash",
        "Library/Keychains",
        "Library/Mail",
        "Library/Messages",
        "Library/Cookies",
        "Library/Mobile Documents",
        "Library/CloudStorage",
        "Library/Containers",
        "Library/Group Containers",
        "Library/Preferences",
        "Library/Safari",
        "Library/Application Support/Google/Chrome",
        "Library/Application Support/Firefox",
        "Library/Application Support/BraveSoftware",
        "Library/Application Support/Microsoft Edge",
        "Library/Application Support/MobileSync/Backup",
        "Pictures/Photos Library.photoslibrary",
    ] {
        for home in ["/Users/me", "/users/ME", "/USERS/me"] {
            let folder = format!("{home}/{place}");
            let inside = format!("{folder}/x/y");
            let lower = format!("/Users/me/{}", place.to_lowercase());

            assert!(refused(&protected, &folder), "{folder}");
            assert!(refused(&protected, &inside), "{inside}");
            assert!(refused(&protected, &lower), "{lower}");
        }
    }

    assert!(refused(
        &protected,
        "/users/me/library/keychains/login.keychain-db"
    ));
    assert!(refused(
        &protected,
        "/Users/me/Library/Mobile Documents/com~apple~CloudDocs/a"
    ));
    assert!(refused(
        &protected,
        "/Users/me/pictures/photos library.photoslibrary/originals/a"
    ));
}

#[test]
fn a_folder_that_holds_a_closed_place_is_protected_but_what_is_meant_to_be_cleaned_beside_it_is_not() {
    let protected = Protected::for_environment(&mac()).unwrap();

    // Moving the Library, `Application Support` or `Google` whole takes the Keychains, the browser
    // profiles or Chrome with it (as `AppData` on Windows). The same holds under another spelling.
    for path in [
        "/Users/me",
        "/Users/me/Library",
        "/users/ME/library",
        "/Users/me/Library/Application Support",
        "/users/me/LIBRARY/application support",
        "/Users/me/Library/Application Support/Google",
        "/Users/me/Pictures",
    ] {
        assert!(refused(&protected, path), "{path}");
    }

    // What is meant to be cleaned in the Library holds no closed place.
    for path in [
        "/Users/me/Library/Caches",
        "/Users/me/Library/Caches/com.example.app",
        "/Users/me/Library/Logs/DiagnosticReports",
        "/Users/me/Library/Developer/Xcode/DerivedData",
        "/Users/me/Library/Developer/Xcode/iOS DeviceSupport/17.2",
        "/Users/me/Library/Developer/CoreSimulator/Caches",
        "/Users/me/Library/pnpm/store",
        "/Users/me/Library/Application Support/Code/Cache",
        "/Users/me/Library/Application Support/Google/Other",
        "/Users/me/Library2/x",
        "/Users/me/code/app/target",
        "/Users/me/Downloads/old.dmg",
        "/Users/me/.npm/_cacache",
        "/Users/me/.cargo/registry",
        "/Users/other/scratch",
        "/Users/mem/x",
        "/Volumes/Ext/data/big.bin",
        "/private/var/folders/zz/T/x",
        "/private/var/db2/x",
        "/Systems/x",
        "/Libraries/x",
        "/Applications2/x",
    ] {
        assert!(!refused(&protected, path), "{path}");
    }
}

#[test]
fn a_personal_folder_the_machine_names_otherwise_is_protected_under_that_name() {
    let mut env = mac();
    env.user_dirs = vec![(UserDir::Videos, PathBuf::from("/Volumes/Media/Films"))];
    let protected = Protected::for_environment(&env).unwrap();

    assert!(refused(&protected, "/volumes/media/FILMS/edit.mov"));
    assert!(
        !refused(&protected, "/Users/me/Movies/x"),
        "the place the machine names replaces the default, as on Linux"
    );
}

#[test]
fn the_videos_folder_is_called_movies_by_default_on_a_mac_and_videos_on_linux() {
    let protected = Protected::for_environment(&mac()).unwrap();
    assert!(refused(&protected, "/Users/me/Movies/x"));
    assert!(!refused(&protected, "/Users/me/Videos/x"));

    let linux = Environment {
        home: Some(PathBuf::from("/home/me")),
        os: Os::Linux,
        ..mac()
    };
    let protected = Protected::for_environment(&linux).unwrap();
    assert!(refused(&protected, "/home/me/Videos/x"));
    assert!(!refused(&protected, "/home/me/Movies/x"));
}

#[test]
fn macos_paths_are_posix_paths_and_anything_else_is_refused() {
    let protected = Protected::for_environment(&mac()).unwrap();

    for path in [
        "Users/me/code",
        "/Users/me/code/../Documents",
        "/Users/me/./code",
        "/Users//me/code",
        "C:\\Users\\me\\code",
        "",
    ] {
        assert!(refused(&protected, path), "{path:?}");
    }
}

#[test]
fn linux_stays_case_sensitive_and_a_mac_home_of_the_top_protects_nothing() {
    let linux = Environment {
        os: Os::Linux,
        ..mac()
    };
    let protected = Protected::for_environment(&linux).unwrap();

    assert!(refused(&protected, "/Users/me/Documents/a"));
    assert!(!refused(&protected, "/users/ME/documents/a"));

    for home in [Some(PathBuf::from("/")), Some(PathBuf::from("relative")), None] {
        let env = Environment { home, ..mac() };

        assert!(Protected::for_environment(&env).is_none());
    }
}

#[test]
fn on_macos_nothing_is_deleted_for_good_and_a_folder_is_emptied_into_the_trash() {
    let options = CleanupOptions::default();
    let on_mac = |id| action_for(id, &options, Os::MacOs);

    for id in [
        "rust-target",
        "rust-target-recent",
        "node-modules",
        "node-modules-recent",
        "python-cache",
        "old-downloads",
        "disk-images",
        "user-safe",
    ] {
        assert_eq!(on_mac(id), Some(CleanupAction::Trash), "{id}");
    }
    for id in [
        "npm-cache",
        "bun-cache",
        "pnpm-store",
        "yarn-cache",
        "cargo-registry",
        "gradle-cache",
        "maven-repository",
        "nuget-packages",
        "go-modules",
        "mac-xcode-derived-data",
        "mac-simulator-caches",
        "mac-pnpm-store",
        "mac-app-caches",
        "mac-logs",
        "mac-device-support",
    ] {
        assert_eq!(on_mac(id), Some(CleanupAction::TrashChildren), "{id}");
    }
}

#[test]
fn what_platter_cannot_clean_on_macos_is_shown_but_has_no_action() {
    let options = CleanupOptions::default();

    for id in [
        // The Trash itself, and a disk image in a protected folder.
        "mac-trash",
        "mac-docker-vm",
        // Kept on purpose.
        "mac-iphone-backups",
        "mac-mail",
        "mac-messages",
        "mac-keychains",
        "documents",
        "videos",
        // Linux rules and tools.
        "trash",
        "app-cache",
        "coredumps",
        "pacman-cache",
        "apt-cache",
        "dnf-cache",
        "journal",
        "docker-data",
        "docker-desktop-vm",
        "steam-shader-cache",
        "btrfs-snapshots",
        // Windows.
        "win-temp",
        "win-component-store",
        "no-such-rule",
    ] {
        assert_eq!(action_for(id, &options, Os::MacOs), None, "{id}");
    }
}

#[test]
fn no_action_on_macos_empties_a_trash_or_runs_a_tool() {
    let options = CleanupOptions::default();

    for rule in RuleSet::builtin().rules() {
        let action = action_for(rule.id, &options, Os::MacOs);

        assert!(
            matches!(
                action,
                None | Some(CleanupAction::Trash | CleanupAction::TrashChildren)
            ),
            "{}: {action:?}",
            rule.id
        );
    }
}

#[test]
fn the_rules_of_macos_have_no_action_on_linux() {
    for id in [
        "mac-xcode-derived-data",
        "mac-app-caches",
        "mac-logs",
        "mac-trash",
    ] {
        assert_eq!(
            action_for(id, &CleanupOptions::default(), Os::Linux),
            None,
            "{id}"
        );
    }
}

#[test]
fn each_os_has_its_own_style() {
    assert_eq!(PathStyle::for_os(Os::MacOs), PathStyle::Mac);
    assert_eq!(PathStyle::for_os(Os::Linux), PathStyle::Posix);
    assert_eq!(PathStyle::for_os(Os::Windows), PathStyle::Windows);
}

/// Records what is asked of it. Deleting for good is a failure of the test.
#[derive(Default)]
struct Trash {
    calls: Mutex<Vec<(String, &'static str, PathBuf)>>,
}

impl EntryRemover for Trash {
    fn remove(&self, path: &[u8], _expected: &ExpectedEntry, _mode: RemoveMode) -> io::Result<Removed> {
        panic!("deleted {} for good", String::from_utf8_lossy(path));
    }

    fn trash(&self, path: &[u8], _expected: &ExpectedEntry, trash_dir: &Path) -> io::Result<Vec<u8>> {
        self.calls.lock().unwrap().push((
            String::from_utf8_lossy(path).into_owned(),
            "trash",
            trash_dir.to_path_buf(),
        ));

        Ok(trash_dir.join("x").into_os_string().into_encoded_bytes())
    }

    fn trash_children(
        &self,
        path: &[u8],
        _expected: &ExpectedEntry,
        trash_dir: &Path,
        _limit: usize,
    ) -> io::Result<TrashedChildren> {
        self.calls.lock().unwrap().push((
            String::from_utf8_lossy(path).into_owned(),
            "children",
            trash_dir.to_path_buf(),
        ));

        Ok(TrashedChildren {
            children: vec![TrashedChild {
                name: b"inner".to_vec(),
                dev: 1,
                ino: 2,
                is_dir: false,
                allocated: 10,
                outcome: ChildTrashed::To(trash_dir.join("inner").into_os_string().into_encoded_bytes()),
            }],
            more: 0,
        })
    }

    fn free_bytes(&self, _path: &Path) -> io::Result<u64> {
        Ok(0)
    }
}

fn scan_root(fs: &FakeFs) -> Tree {
    let config = ScanConfig {
        now: Some(NOW),
        ..ScanConfig::default()
    };
    scan(fs, Path::new("/"), &config, &CancelToken::new(), &|_| {}).unwrap()
}

fn disk() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/Users/me/Library/Developer/Xcode/DerivedData/App/x", 5_000)
        .file("/Users/me/Library/Caches/com.example/blob", 4_000)
        .file("/Users/me/Library/Mail/V10/m.emlx", 3_000)
        .file(
            "/Users/me/Library/Containers/com.docker.docker/Data/vms/0/Docker.raw",
            2_000,
        )
        .file("/Users/me/.Trash/old", 1_000)
        .file("/Users/me/code/rs/Cargo.toml", 1);
    fs.file("/Users/me/code/rs/target/x", 6_000)
        .set_mtime("/Users/me/code/rs/target/x", NOW - 90 * DAY);
    fs
}

#[test]
fn a_cleanup_on_macos_only_ever_moves_to_the_trash_folder_of_the_home() {
    let fs = disk();
    let plan = plan_cleanup(
        &scan_root(&fs),
        &mac(),
        &RuleSet::builtin(),
        &["mac-xcode-derived-data", "mac-app-caches", "rust-target"],
        &fs,
        &CleanupOptions::default(),
    );
    assert!(plan.refused.is_empty(), "{:?}", plan.refused);
    let remover = Trash::default();
    let trash = Path::new("/Users/me/.Trash");
    let disk = Disk {
        probe: &fs,
        remover: &remover,
        trash_dir: Some(trash),
        tools: None,
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    let mut calls = remover.calls.lock().unwrap().clone();
    calls.sort();
    assert_eq!(
        calls,
        [
            (
                "/Users/me/Library/Caches".to_owned(),
                "children",
                PathBuf::from("/Users/me/.Trash")
            ),
            (
                "/Users/me/Library/Developer/Xcode/DerivedData".to_owned(),
                "children",
                PathBuf::from("/Users/me/.Trash")
            ),
            (
                "/Users/me/code/rs/target".to_owned(),
                "trash",
                PathBuf::from("/Users/me/.Trash")
            ),
        ]
    );
    assert!(
        report
            .results
            .iter()
            .all(|result| matches!(result.outcome, TargetOutcome::Trashed(_))),
        "{:?}",
        report.results
    );
}

#[test]
fn without_a_trash_folder_nothing_is_moved_on_macos() {
    let fs = disk();
    let plan = plan_cleanup(
        &scan_root(&fs),
        &mac(),
        &RuleSet::builtin(),
        &["rust-target"],
        &fs,
        &CleanupOptions::default(),
    );
    let remover = Trash::default();
    let disk = Disk {
        probe: &fs,
        remover: &remover,
        trash_dir: None,
        tools: None,
    };

    let report = execute(&plan, &disk, &CancelToken::new(), &mut |_| {});

    assert!(remover.calls.lock().unwrap().is_empty());
    assert!(matches!(&report.results[0].outcome, TargetOutcome::Skipped(why) if why.contains("trash")));
}

#[test]
fn the_trash_itself_a_protected_library_folder_and_the_mail_are_never_planned() {
    let fs = disk();

    for id in ["mac-trash", "mac-docker-vm", "mac-mail"] {
        let plan = plan_cleanup(
            &scan_root(&fs),
            &mac(),
            &RuleSet::builtin(),
            &[id],
            &fs,
            &CleanupOptions::default(),
        );

        assert!(plan.groups().is_empty(), "{id}");
    }
}
