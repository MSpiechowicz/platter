use std::path::Path;

use super::*;

fn file_with(rules: &str, next_id: u32) -> Vec<u8> {
    format!(r#"{{"version":1,"next_id":{next_id},"rules":[{rules}]}}"#).into_bytes()
}

/// A POSIX-looking absolute path as this host writes it: unchanged on Unix, on a drive on Windows.
fn abs(posix: &str) -> String {
    if cfg!(windows) && posix.starts_with('/') {
        format!("C:{}", posix.replace('/', "\\"))
    } else {
        posix.to_owned()
    }
}

/// What `abs` adds in front of a path that starts with `/`.
const ABS_PREFIX_BYTES: usize = if cfg!(windows) { 2 } else { 0 };

fn rule(id: u32, folder: &str) -> String {
    let folder = serde_json::to_string(&abs(folder)).unwrap();

    format!(r#"{{"id":{id},"verdict":"keep","folder":{folder}}}"#)
}

#[test]
fn a_file_that_was_rendered_reads_back_the_same() {
    let mut file = UserRulesFile::default();
    file.set(Path::new(&abs("/home/u/Videos")), UserVerdict::Keep)
        .unwrap();
    file.set(Path::new(&abs("/home/u/tmp")), UserVerdict::Safe)
        .unwrap();

    let again = parse(&render(&file)).unwrap();

    assert_eq!(again, file);
    assert_eq!(again.next_id, 3);
}

#[test]
fn the_empty_file_has_no_rules() {
    let file = parse(&file_with("", 1)).unwrap();

    assert!(file.rules.is_empty());
}

#[test]
fn the_schema_is_enforced() {
    let two_rules = format!("{},{}", rule(1, "/a"), rule(1, "/b"));
    let many: String = (1..=257)
        .map(|id| rule(id, &format!("/f{id}")))
        .collect::<Vec<_>>()
        .join(",");
    let long = format!("/{}", "a".repeat(MAX_RULE_PATH_BYTES));
    let huge = format!(
        "{}{}",
        String::from_utf8(file_with("", 1)).unwrap(),
        " ".repeat(MAX_RULES_FILE_BYTES)
    );

    let cases: Vec<(&str, Vec<u8>, RulesFileError)> = vec![
        (
            "other version",
            br#"{"version":2,"next_id":1,"rules":[]}"#.to_vec(),
            RulesFileError::Version(2),
        ),
        (
            "no version",
            br#"{"next_id":1,"rules":[]}"#.to_vec(),
            RulesFileError::Syntax(String::new()),
        ),
        (
            "duplicate id",
            file_with(&two_rules, 2),
            RulesFileError::DuplicateId(1),
        ),
        (
            "id at next_id",
            file_with(&rule(5, "/a"), 5),
            RulesFileError::NextId { next: 5, id: 5 },
        ),
        ("too many", file_with(&many, 300), RulesFileError::TooMany),
        ("too large", huge.into_bytes(), RulesFileError::TooLarge),
        (
            "relative",
            file_with(&rule(1, "home/u"), 2),
            RulesFileError::BadFolder(String::new()),
        ),
        (
            "dot dot",
            file_with(&rule(1, "/home/../etc"), 2),
            RulesFileError::BadFolder(String::new()),
        ),
        (
            "dot",
            file_with(&rule(1, "/home/./u"), 2),
            RulesFileError::BadFolder(String::new()),
        ),
        (
            "doubled slash",
            file_with(&rule(1, "/home//u"), 2),
            RulesFileError::BadFolder(String::new()),
        ),
        (
            "trailing slash",
            file_with(&rule(1, "/home/u/"), 2),
            RulesFileError::BadFolder(String::new()),
        ),
        (
            "root",
            file_with(&rule(1, "/"), 2),
            RulesFileError::BadFolder(String::new()),
        ),
        (
            "empty",
            file_with(&rule(1, ""), 2),
            RulesFileError::BadFolder(String::new()),
        ),
        (
            "NUL",
            file_with(&rule(1, "/home/\u{0}x"), 2),
            RulesFileError::BadFolder(String::new()),
        ),
        (
            "too long path",
            file_with(&rule(1, &long), 2),
            RulesFileError::BadFolder(String::new()),
        ),
        (
            "unknown field",
            br#"{"version":1,"next_id":1,"rules":[],"extra":1}"#.to_vec(),
            RulesFileError::Syntax(String::new()),
        ),
        (
            "bad verdict",
            br#"{"version":1,"next_id":2,"rules":[{"id":1,"verdict":"maybe","folder":"/a"}]}"#.to_vec(),
            RulesFileError::Syntax(String::new()),
        ),
        (
            "not json",
            b"keep everything".to_vec(),
            RulesFileError::Syntax(String::new()),
        ),
    ];

    for (name, bytes, expected) in cases {
        let error = parse(&bytes).expect_err(name);

        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(&expected),
            "{name}: {error}"
        );
    }
}

#[test]
fn exactly_the_limits_are_accepted() {
    let all: String = (1..=256)
        .map(|id| rule(id, &format!("/f{id}")))
        .collect::<Vec<_>>()
        .join(",");
    let longest = format!("/{}", "a".repeat(MAX_RULE_PATH_BYTES - 1 - ABS_PREFIX_BYTES));

    assert_eq!(parse(&file_with(&all, 257)).unwrap().rules.len(), 256);
    parse(&file_with(&rule(1, &longest), 2)).unwrap();
}

#[test]
fn setting_a_folder_again_replaces_its_verdict_and_keeps_its_id() {
    let mut file = UserRulesFile::default();
    let first = file.set(Path::new(&abs("/data/a")), UserVerdict::Keep).unwrap();
    let second = file.set(Path::new(&abs("/data/b")), UserVerdict::Keep).unwrap();

    let again = file.set(Path::new(&abs("/data/a")), UserVerdict::Safe).unwrap();

    assert_eq!(again, first);
    assert_ne!(second, first);
    assert_eq!(file.rules.len(), 2);
    assert_eq!(file.rules[0].verdict, UserVerdict::Safe);
}

#[test]
fn ids_of_removed_rules_are_not_reused() {
    let mut file = UserRulesFile::default();
    let first = file.set(Path::new(&abs("/data/a")), UserVerdict::Keep).unwrap();

    assert!(file.remove(first));
    assert!(!file.remove(first));
    let next = file.set(Path::new(&abs("/data/b")), UserVerdict::Keep).unwrap();

    assert_ne!(next, first);
}

#[test]
fn the_list_is_capped() {
    let mut file = UserRulesFile::default();
    for index in 0..MAX_USER_RULES {
        file.set(Path::new(&abs(&format!("/data/{index}"))), UserVerdict::Keep)
            .unwrap();
    }

    assert_eq!(
        file.set(Path::new(&abs("/data/more")), UserVerdict::Keep),
        Err(RulesFileError::TooMany)
    );
    // An existing folder can still change its verdict.
    file.set(Path::new(&abs("/data/0")), UserVerdict::Safe).unwrap();
    assert_eq!(file.rules.len(), MAX_USER_RULES);
}

#[test]
fn a_folder_that_is_not_acceptable_changes_nothing() {
    let mut file = UserRulesFile::default();

    for folder in ["/", "relative", "/a/../b", "/a/", ""] {
        assert!(
            file.set(Path::new(&abs(folder)), UserVerdict::Keep).is_err(),
            "{folder:?}"
        );
    }

    assert_eq!(file, UserRulesFile::default());
}

#[cfg(unix)]
#[test]
fn a_folder_that_is_not_text_is_refused() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let path = Path::new(OsStr::from_bytes(b"/home/u/\xff"));

    assert!(matches!(check_folder(path), Err(RulesFileError::BadFolder(_))));
}

#[test]
fn a_repeated_key_or_folder_is_refused_not_resolved() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "folder key twice in a rule",
            br#"{"version":1,"next_id":3,"rules":[{"id":1,"verdict":"keep","folder":"/a","folder":"/b"}]}"#
                .to_vec(),
        ),
        (
            "verdict key twice in a rule",
            br#"{"version":1,"next_id":3,"rules":[{"id":1,"verdict":"keep","verdict":"safe","folder":"/a"}]}"#
                .to_vec(),
        ),
        (
            "rules key twice",
            br#"{"version":1,"next_id":3,"rules":[{"id":1,"verdict":"keep","folder":"/a"}],"rules":[]}"#
                .to_vec(),
        ),
        (
            "version key twice",
            br#"{"version":1,"version":1,"next_id":3,"rules":[]}"#.to_vec(),
        ),
        (
            "next_id key twice",
            br#"{"version":1,"next_id":1,"next_id":9,"rules":[]}"#.to_vec(),
        ),
    ];
    for (name, bytes) in cases {
        assert!(
            matches!(parse(&bytes), Err(RulesFileError::Syntax(_))),
            "{name}: {:?}",
            parse(&bytes)
        );
    }

    let twice = format!("{},{}", rule(1, "/home/u/a"), rule(2, "/home/u/a"));
    assert_eq!(
        parse(&file_with(&twice, 3)),
        Err(RulesFileError::DuplicateFolder(abs("/home/u/a")))
    );
}

#[test]
fn a_file_of_another_version_is_reported_as_that_whatever_else_is_in_it() {
    let other = br#"{"version":2,"next_id":1,"rules":[{"id":1,"surprise":true}],"extra":null}"#;

    assert_eq!(parse(other), Err(RulesFileError::Version(2)));
}

#[test]
fn a_rule_that_would_make_the_file_too_large_is_refused_and_what_is_kept_reads_back() {
    // A control character renders as six bytes (`\u0001`), so a path that passes the 4096-byte check
    // renders far larger.
    let component = "\u{1}".repeat(250);
    let long = |index: usize| {
        let mut path = String::from("/home/u");
        for _ in 0..15 {
            path.push('/');
            path.push_str(&component);
        }
        format!("{path}/d{index}")
    };

    let mut file = UserRulesFile::default();
    let mut accepted = 0;
    let refused = loop {
        let before = file.clone();
        match file.set(Path::new(&abs(&long(accepted))), UserVerdict::Keep) {
            Ok(_) => accepted += 1,
            Err(error) => {
                assert_eq!(file, before, "a refused change leaves the file as it was");
                break error;
            }
        }
        assert!(accepted <= MAX_USER_RULES, "the cap must stop it");
    };

    assert_eq!(refused, RulesFileError::WouldBeTooLarge);
    assert!(accepted > 0);
    let bytes = render(&file);
    assert!(bytes.len() <= MAX_RULES_FILE_BYTES);
    assert_eq!(parse(&bytes).unwrap(), file, "what was accepted reads back");
}

/// A hand-written compact file just under the cap: many long folders, as a tool might generate them.
fn compact_file_near_the_cap() -> (UserRulesFile, Vec<u8>) {
    let mut rules = Vec::new();
    let mut id = 1u32;
    loop {
        let folder = abs(&format!("/home/u/{}{id}", "a".repeat(1000)));
        rules.push(UserRule {
            id,
            verdict: UserVerdict::Keep,
            folder: folder.into(),
        });

        let file = UserRulesFile {
            version: RULES_FILE_VERSION,
            next_id: id + 1,
            rules: rules.clone(),
        };
        let compact = serde_json::to_vec(&file).unwrap();
        if id as usize >= MAX_USER_RULES || compact.len() > MAX_RULES_FILE_BYTES - 2000 {
            return (file, compact);
        }
        id += 1;
    }
}

#[test]
fn a_compact_file_near_the_cap_can_be_changed_and_is_written_back_within_the_cap() {
    let (file, compact) = compact_file_near_the_cap();
    assert!(
        compact.len() > MAX_RULES_FILE_BYTES - 2000,
        "control: near the cap"
    );
    assert!(
        serde_json::to_vec_pretty(&file).unwrap().len() > MAX_RULES_FILE_BYTES,
        "control: the pretty form would not fit"
    );
    let mut held = parse(&compact).expect("the compact file is read");

    // Removing a rule.
    assert!(held.remove(1));
    let bytes = render(&held);
    assert!(bytes.len() <= MAX_RULES_FILE_BYTES);
    assert_eq!(parse(&bytes).unwrap(), held, "what is written reads back");

    // Changing a verdict (a change of the same size).
    let folder = held.rules[0].folder.clone();
    held.set(&folder, UserVerdict::Safe).expect("the change fits");
    let bytes = render(&held);
    assert!(bytes.len() <= MAX_RULES_FILE_BYTES);
    assert_eq!(parse(&bytes).unwrap(), held);
}

#[test]
fn a_small_file_is_still_rendered_pretty_for_a_person_to_read() {
    let mut file = UserRulesFile::default();
    file.set(Path::new(&abs("/home/u/tmp")), UserVerdict::Safe)
        .unwrap();

    let text = String::from_utf8(render(&file)).unwrap();

    assert!(text.lines().count() > 3, "{text}");
    assert!(text.ends_with('\n'));
}

#[test]
fn a_change_is_refused_only_when_even_the_compact_file_would_not_fit() {
    let (mut file, _) = compact_file_near_the_cap();
    let compact = |file: &UserRulesFile| serde_json::to_vec(file).unwrap().len();

    // Grow it until one more rule no longer fits compactly.
    let mut index = 0usize;
    let refused = loop {
        let before = file.clone();
        let folder = format!("/home/u/{}z{index}", "b".repeat(1000));
        match file.set(Path::new(&abs(&folder)), UserVerdict::Keep) {
            Ok(_) => {
                assert!(compact(&file) <= MAX_RULES_FILE_BYTES, "accepted means it fits");
                index += 1;
            }
            Err(error) => {
                assert_eq!(file, before);
                break error;
            }
        }
        assert!(index < MAX_USER_RULES, "the cap must stop it");
    };

    assert!(matches!(
        refused,
        RulesFileError::WouldBeTooLarge | RulesFileError::TooMany
    ));
    assert!(parse(&render(&file)).is_ok());
}
