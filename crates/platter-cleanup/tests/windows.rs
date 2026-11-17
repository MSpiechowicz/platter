//! The Windows syntax, protected places and actions, on any system: the style comes from the
//! environment, not from the OS the tests run on.

use std::path::PathBuf;

use platter_cleanup::{CleanupAction, CleanupOptions, PathStyle, Protected, action_for};
use platter_core::testing::NOW;
use platter_core::{Environment, Os, UserDir};

fn windows() -> Environment {
    Environment {
        home: Some(PathBuf::from("C:\\Users\\Me")),
        user_dirs: vec![(UserDir::Documents, PathBuf::from("D:\\Docs"))],
        now: NOW,
        os: Os::Windows,
        system_root: Some(PathBuf::from("C:\\Windows")),
    }
}

fn refused(protected: &Protected, path: &str) -> bool {
    protected.refuses(path.as_bytes()).is_some()
}

#[test]
fn what_windows_keeps_for_itself_and_the_user_is_protected_whatever_the_spelling() {
    let protected = Protected::for_environment(&windows()).unwrap();

    for path in [
        "C:\\",
        "D:\\",
        "E:\\",
        "C:\\Users",
        "C:\\Users\\Me",
        "c:\\users\\me",
        "C:\\Windows",
        "c:\\WINDOWS\\System32\\cmd.exe",
        "C:\\Windows\\Temp\\x",
        "C:\\Program Files\\App",
        "C:\\Program Files (x86)\\Vendor\\App\\app.exe",
        "C:\\ProgramData",
        "C:\\ProgramData\\Vendor\\cache",
        "C:\\Recovery",
        "C:\\$Recycle.Bin\\S-1-5-21\\$R1",
        "C:\\System Volume Information\\x",
        "D:\\$Recycle.Bin",
        "C:\\Users\\Me\\AppData\\Roaming",
        "C:\\Users\\Me\\AppData\\Roaming\\Code\\User\\settings.json",
        "C:\\Users\\Me\\.ssh",
        "C:\\Users\\Me\\.ssh\\id_ed25519",
        "C:\\Users\\Me\\.aws\\credentials",
        "C:\\Users\\Me\\Desktop",
        "C:\\Users\\Me\\Pictures\\2026\\a.jpg",
        "D:\\Docs",
        "D:\\Docs\\thesis.docx",
        "C:\\Users\\Me\\AppData\\Local\\Google\\Chrome\\User Data\\Default",
        "C:\\Users\\Me\\AppData\\Local\\Microsoft\\Windows\\UsrClass.dat",
    ] {
        assert!(refused(&protected, path), "{path}");
    }
}

#[test]
fn a_folder_that_holds_a_protected_place_is_protected_too_so_no_target_takes_it_along() {
    let protected = Protected::for_environment(&windows()).unwrap();

    for path in [
        "C:\\Users\\Me\\AppData",
        "C:\\Users\\Me\\AppData\\Local",
        "c:\\users\\me\\appdata\\local",
        "C:\\Users\\Me\\AppData\\Local\\Microsoft",
        "C:\\Users\\Me\\AppData\\Local\\Google",
        "C:\\Users\\Me\\AppData\\Local\\Google\\Chrome",
        "C:\\Users\\Me\\AppData\\Local\\Microsoft\\Edge",
        "C:\\Users\\Me\\AppData\\Local\\BraveSoftware",
    ] {
        assert!(refused(&protected, path), "{path}");
    }
}

#[test]
fn names_are_folded_the_way_ntfs_folds_them_not_only_in_ascii() {
    let protected = Protected::for_environment(&windows()).unwrap();

    // The long s (`\u{17f}`) uppercases to `S` and the dotless i (`\u{131}`) to `I`.
    for path in [
        "d:\\DOCS\\x",
        "C:\\Windows\\Sy\u{17f}tem32\\x",
        "C:\\Users\\Me\\.\u{17f}sh\\id",
        "C:\\Program F\u{131}les\\App",
    ] {
        assert!(refused(&protected, path), "{path}");
    }

    let mut env = windows();
    env.user_dirs = vec![(UserDir::Documents, PathBuf::from("D:\\Żółć"))];
    let protected = Protected::for_environment(&env).unwrap();
    assert!(refused(&protected, "d:\\żÓŁĆ\\thesis.docx"));
}

#[test]
fn what_is_meant_to_be_cleaned_is_not_protected() {
    let protected = Protected::for_environment(&windows()).unwrap();

    for path in [
        "C:\\Users\\Me\\AppData\\Local\\Temp",
        "C:\\Users\\Me\\AppData\\Local\\npm-cache\\_cacache",
        "C:\\Users\\Me\\AppData\\Local\\NuGet\\v3-cache",
        "C:\\Users\\Me\\AppData\\Local\\CrashDumps\\a.dmp",
        "C:\\Users\\Me\\AppData\\Local\\Programs\\x",
        "C:\\Users\\Me\\AppData\\Local\\Google2",
        "C:\\Users\\Me\\code\\app\\target",
        "C:\\Users\\Me\\Downloads\\old.iso",
        "C:\\Users\\Me\\Documents\\kept-by-name-only-where-the-desktop-says",
        "D:\\data\\big.bin",
        "C:\\Users\\Other\\scratch",
        "C:\\Program Filesystem\\x",
        "C:\\Windows.old\\x",
    ] {
        assert!(!refused(&protected, path), "{path}");
    }
}

#[test]
fn a_path_that_is_not_plainly_a_drive_absolute_one_is_refused() {
    let protected = Protected::for_environment(&windows()).unwrap();

    for path in [
        "C:/Users/Me/code",
        "C:\\Users\\Me\\code/app",
        "\\\\server\\share\\x",
        "\\\\?\\C:\\Users\\Me\\code",
        "C:code",
        "code\\app",
        "/home/me/code",
        "C:\\Users\\Me\\code\\..\\..\\Windows",
        "C:\\Users\\Me\\code.",
        "C:\\Users\\Me\\NUL",
    ] {
        assert!(refused(&protected, path), "{path}");
    }
}

#[test]
fn without_the_system_folder_the_usual_place_on_the_home_drive_is_protected() {
    let mut env = windows();
    env.system_root = None;
    let protected = Protected::for_environment(&env).unwrap();

    assert!(refused(&protected, "C:\\Windows\\System32"));
}

#[test]
fn a_home_that_is_not_a_profile_folder_protects_nothing_and_plans_nothing() {
    for home in ["C:\\", "/home/me", "C:", "relative", "\\\\server\\share\\me"] {
        let mut env = windows();
        env.home = Some(PathBuf::from(home));

        assert!(Protected::for_environment(&env).is_none(), "{home}");
    }
    let mut env = windows();
    env.home = None;
    assert!(Protected::for_environment(&env).is_none());
}

#[test]
fn the_component_store_may_be_cleaned_by_its_tool_and_by_nothing_else() {
    let protected = Protected::for_environment(&windows()).unwrap();

    // By path alone nothing in the Windows folder is a target.
    assert!(refused(&protected, "C:\\Windows\\WinSxS"));
    assert!(refused(&protected, "c:\\windows\\winsxs\\x"));
}

#[test]
fn on_windows_nothing_is_deleted_for_good_and_a_folder_is_emptied_into_the_bin() {
    let options = CleanupOptions::default();
    let windows = |id| action_for(id, &options, Os::Windows);

    for id in [
        "rust-target",
        "rust-target-recent",
        "node-modules",
        "node-modules-recent",
        "python-cache",
    ] {
        assert_eq!(windows(id), Some(CleanupAction::Trash), "{id}");
    }
    for id in [
        "win-temp",
        "win-npm-cache",
        "win-pip-cache",
        "win-nuget-http-cache",
        "win-nuget-v3-cache",
        "win-crash-dumps",
        "cargo-registry",
        "nuget-packages",
    ] {
        assert_eq!(windows(id), Some(CleanupAction::TrashChildren), "{id}");
    }
    for id in ["old-downloads", "disk-images", "user-safe"] {
        assert_eq!(windows(id), Some(CleanupAction::Trash), "{id}");
    }
}

#[test]
fn the_tools_and_rules_of_linux_do_not_exist_on_windows() {
    let options = CleanupOptions::default();

    for id in [
        "trash",
        "app-cache",
        "pacman-cache",
        "apt-cache",
        "dnf-cache",
        "journal",
        "coredumps",
        "docker-data",
        "docker-desktop-vm",
        "steam-shader-cache",
        "btrfs-snapshots",
        "documents",
        "no-such-rule",
    ] {
        assert_eq!(action_for(id, &options, Os::Windows), None, "{id}");
    }
}

#[test]
fn the_component_store_is_cleaned_only_by_dism_as_the_administrator() {
    let action = action_for("win-component-store", &CleanupOptions::default(), Os::Windows);

    let Some(CleanupAction::RunTool(tool)) = action else {
        panic!("{action:?}")
    };
    assert_eq!(tool.program(), "dism");
    assert_eq!(
        tool.args(),
        ["/Online", "/Cleanup-Image", "/StartComponentCleanup"]
    );
    assert!(tool.needs_elevation());
    assert_eq!(
        action_for("win-component-store", &CleanupOptions::default(), Os::Linux),
        None
    );
}

#[test]
fn linux_actions_do_not_change() {
    let options = CleanupOptions::default();

    assert_eq!(
        action_for("rust-target", &options, Os::Linux),
        Some(CleanupAction::Delete)
    );
    assert_eq!(
        action_for("app-cache", &options, Os::Linux),
        Some(CleanupAction::EmptyFolder)
    );
    assert_eq!(
        action_for("trash", &options, Os::Linux),
        Some(CleanupAction::EmptyTrash)
    );
    assert_eq!(
        action_for("old-downloads", &options, Os::Linux),
        Some(CleanupAction::Trash)
    );
}

#[test]
fn each_os_has_its_own_style() {
    assert_eq!(PathStyle::for_os(Os::Windows), PathStyle::Windows);
    assert_eq!(PathStyle::for_os(Os::Linux), PathStyle::Posix);
}
