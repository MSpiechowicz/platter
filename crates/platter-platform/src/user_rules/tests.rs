use platter_core::{UserVerdict, render_user_rules};

use super::*;

fn file_with(folders: &[(&str, UserVerdict)]) -> UserRulesFile {
    let mut file = UserRulesFile::default();
    for (folder, verdict) in folders {
        file.set(Path::new(folder), *verdict).unwrap();
    }
    file
}

#[test]
fn a_missing_file_means_no_rules() {
    let root = tempfile::tempdir().unwrap();

    let loaded = load(&root.path().join("platter/rules.json"));

    assert_eq!(loaded, Loaded::Rules(UserRulesFile::default()));
}

#[test]
fn what_is_saved_is_read_back_and_the_folder_is_created_private() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config/platter/rules.json");
    let file = file_with(&[
        ("/home/u/Videos", UserVerdict::Keep),
        ("/home/u/tmp", UserVerdict::Safe),
    ]);

    save(&path, &file).unwrap();

    assert_eq!(load(&path), Loaded::Rules(file));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
    }
}

#[test]
fn saving_replaces_the_file_and_leaves_no_temporary_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");

    save(&path, &file_with(&[("/a", UserVerdict::Keep)])).unwrap();
    save(&path, &file_with(&[("/b", UserVerdict::Safe)])).unwrap();

    let Loaded::Rules(file) = load(&path) else {
        panic!("the file reads back");
    };
    assert_eq!(file.rules[0].folder, Path::new("/b"));
    let names: Vec<_> = std::fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["rules.json"]);
}

#[test]
fn a_folder_at_the_path_is_broken_and_nothing_is_written_next_to_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");
    std::fs::create_dir(&path).unwrap();

    let loaded = load(&path);
    let error = save(&path, &UserRulesFile::default()).unwrap_err();

    assert!(matches!(loaded, Loaded::Broken(_)), "{loaded:?}");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn a_broken_file_is_reported_and_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");
    let garbage = b"{ this is not json";
    std::fs::write(&path, garbage).unwrap();

    let loaded = load(&path);
    let error = save(&path, &file_with(&[("/a", UserVerdict::Keep)])).unwrap_err();

    assert!(matches!(loaded, Loaded::Broken(_)), "{loaded:?}");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read(&path).unwrap(), garbage);
    assert_eq!(
        std::fs::read_dir(root.path()).unwrap().count(),
        1,
        "no temporary file stays"
    );
}

#[test]
fn a_file_over_the_cap_or_with_a_bad_schema_is_broken() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");

    let mut huge = render_user_rules(&UserRulesFile::default());
    huge.resize(MAX_RULES_FILE_BYTES + 1, b' ');
    std::fs::write(&path, &huge).unwrap();
    assert!(matches!(load(&path), Loaded::Broken(why) if why.contains("larger")));

    std::fs::write(
        &path,
        br#"{"version":1,"next_id":2,"rules":[{"id":1,"verdict":"keep","folder":"relative"}]}"#,
    )
    .unwrap();
    assert!(matches!(load(&path), Loaded::Broken(_)));
}

#[test]
fn a_file_of_exactly_the_cap_is_read() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");
    let mut bytes = render_user_rules(&UserRulesFile::default());
    bytes.resize(MAX_RULES_FILE_BYTES, b' ');
    std::fs::write(&path, &bytes).unwrap();

    assert_eq!(load(&path), Loaded::Rules(UserRulesFile::default()));
}

#[cfg(unix)]
#[test]
fn a_link_at_the_path_is_refused_not_followed() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real.json");
    save(&real, &file_with(&[("/a", UserVerdict::Keep)])).unwrap();
    let path = root.path().join("rules.json");
    std::os::unix::fs::symlink(&real, &path).unwrap();

    let loaded = load(&path);
    let error = save(&path, &UserRulesFile::default()).unwrap_err();

    assert!(matches!(loaded, Loaded::Broken(_)), "{loaded:?}");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    assert_eq!(
        load(&real),
        Loaded::Rules(file_with(&[("/a", UserVerdict::Keep)]))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_fifo_at_the_path_is_broken_without_blocking() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );

    assert!(matches!(load(&path), Loaded::Broken(_)));
}

#[test]
fn moving_a_broken_file_aside_keeps_its_bytes_and_clears_the_name() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");
    std::fs::write(&path, b"garbage").unwrap();

    let moved = move_aside(&path).unwrap();

    assert_eq!(moved, root.path().join("rules.json.broken"));
    assert_eq!(std::fs::read(&moved).unwrap(), b"garbage");
    assert_eq!(load(&path), Loaded::Rules(UserRulesFile::default()));
    save(&path, &file_with(&[("/a", UserVerdict::Keep)])).unwrap();
}

#[test]
fn an_earlier_broken_file_is_not_lost_when_another_is_moved_aside() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");
    std::fs::write(&path, b"first").unwrap();
    move_aside(&path).unwrap();
    std::fs::write(&path, b"second").unwrap();

    let moved = move_aside(&path).unwrap();

    assert_eq!(moved, root.path().join("rules.json.broken.1"));
    assert_eq!(
        std::fs::read(root.path().join("rules.json.broken")).unwrap(),
        b"first"
    );
    assert_eq!(std::fs::read(moved).unwrap(), b"second");
}

#[test]
fn moving_aside_a_file_that_is_not_there_fails() {
    let root = tempfile::tempdir().unwrap();

    assert!(move_aside(&root.path().join("rules.json")).is_err());
}

#[test]
fn a_file_that_would_be_too_large_to_read_back_is_refused_and_the_old_file_stays() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");
    let small = file_with(&[("/a", UserVerdict::Keep)]);
    save(&path, &small).unwrap();
    let before = std::fs::read(&path).unwrap();

    // Control characters render as six bytes each; built by hand, as `set` would not allow it.
    let mut huge = UserRulesFile::default();
    for index in 1..=120u32 {
        huge.rules.push(platter_core::UserRule {
            id: index,
            verdict: UserVerdict::Keep,
            folder: format!("/{}/{index}", "\u{1}".repeat(400)).into(),
        });
    }
    huge.next_id = 121;
    assert!(render_user_rules(&huge).len() > MAX_RULES_FILE_BYTES, "control");

    let error = save(&path, &huge).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(std::fs::read(&path).unwrap(), before, "the file is as it was");
    assert_eq!(load(&path), Loaded::Rules(small));
    assert_eq!(
        std::fs::read_dir(root.path()).unwrap().count(),
        1,
        "no temporary file stays"
    );
}

#[test]
fn the_largest_file_that_is_saved_is_read_back() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");
    let mut file = UserRulesFile::default();
    // As many long folders as fit under the cap.
    let mut index = 0;
    loop {
        let folder = format!("/{}/{index}", "\u{1}".repeat(300));
        if file.set(Path::new(&folder), UserVerdict::Keep).is_err() {
            break;
        }
        index += 1;
    }
    assert!(index > 1, "control");

    save(&path, &file).unwrap();

    assert_eq!(load(&path), Loaded::Rules(file));
}

#[test]
fn a_hand_written_compact_file_near_the_cap_can_be_changed_and_saved_again() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");
    // As a tool would write it: no spaces, no newline.
    let mut rules: Vec<String> = Vec::new();
    let written = loop {
        let id = rules.len() + 1;
        rules.push(format!(
            r#"{{"id":{id},"verdict":"keep","folder":"/home/u/{}{id}"}}"#,
            "a".repeat(1000)
        ));
        let compact = format!(
            r#"{{"version":1,"next_id":{},"rules":[{}]}}"#,
            id + 1,
            rules.join(",")
        )
        .into_bytes();
        if id >= 256 || compact.len() > MAX_RULES_FILE_BYTES - 2000 {
            break compact;
        }
    };
    std::fs::write(&path, &written).unwrap();
    assert!(
        written.len() > MAX_RULES_FILE_BYTES - 2000,
        "control: near the cap"
    );
    let Loaded::Rules(mut held) = load(&path) else {
        panic!("the compact file is read");
    };

    // The pretty form of this file is over the cap; the change is written compact all the same.
    assert!(held.remove(1));
    save(&path, &held).expect("removing a rule is saved");
    assert_eq!(load(&path), Loaded::Rules(held.clone()));

    let folder = held.rules[0].folder.clone();
    held.set(&folder, UserVerdict::Safe).expect("the change fits");
    save(&path, &held).expect("changing a verdict is saved");
    assert_eq!(load(&path), Loaded::Rules(held));
}

#[cfg(unix)]
#[test]
fn moving_aside_never_replaces_whatever_is_at_the_broken_names() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rules.json");
    std::fs::write(&path, b"garbage").unwrap();
    // A folder and a dangling link already hold the first two names.
    std::fs::create_dir(root.path().join("rules.json.broken")).unwrap();
    std::os::unix::fs::symlink("nowhere", root.path().join("rules.json.broken.1")).unwrap();

    let moved = move_aside(&path).unwrap();

    assert_eq!(moved, root.path().join("rules.json.broken.2"));
    assert_eq!(std::fs::read(&moved).unwrap(), b"garbage");
    assert!(root.path().join("rules.json.broken").is_dir());
    assert!(
        std::fs::symlink_metadata(root.path().join("rules.json.broken.1"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
