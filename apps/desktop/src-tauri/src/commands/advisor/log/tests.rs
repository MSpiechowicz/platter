use std::path::Path;

use serde_json::{Value, json};

use super::*;

fn entry(n: usize, answer: &str) -> Entry {
    Entry {
        time: utc_time(1_700_000_000),
        scan: 1,
        question: format!("question {n}"),
        polish: false,
        signals: json!({}),
        model: json!("not asked"),
        understood: json!({ "intent": "largest" }),
        clarify: None,
        pick: None,
        rows: 0,
        meta: Vec::new(),
        answer: answer.to_owned(),
        stage_a_ms: 1,
        total_ms: 2,
    }
}

fn lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("every line is JSON"))
        .collect()
}

#[test]
fn utc_time_is_the_unix_second_in_utc() {
    assert_eq!(utc_time(0), "1970-01-01T00:00:00Z");
    assert_eq!(utc_time(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(utc_time(1_700_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(now().len(), "2026-11-16T21:05:33Z".len());
}

#[test]
fn an_entry_is_one_line_with_the_fields_of_the_question() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data/platter").join(FILE_NAME);
    let log = QuestionLog::new(Some(path.clone()));

    let mut first = entry(1, "Safe to clean: nothing.");
    first.meta = vec!["This folder is covered by the rule “x” (verdict safe).".to_owned()];
    first.clarify = Some(json!({ "prompt": "Which one?", "choices": [] }));
    first.pick = Some(1);
    log.append(&first);
    log.append(&entry(2, "second"));

    let written = lines(&path);
    assert_eq!(written.len(), 2);
    assert_eq!(
        written[0],
        json!({
            "time": "2023-11-14T22:13:20Z",
            "scan": 1,
            "question": "question 1",
            "polish": false,
            "signals": {},
            "model": "not asked",
            "understood": { "intent": "largest" },
            "clarify": { "prompt": "Which one?", "choices": [] },
            "pick": 1,
            "rows": 0,
            "meta": ["This folder is covered by the rule “x” (verdict safe)."],
            "answer": "Safe to clean: nothing.",
            "stageAMs": 1,
            "totalMs": 2,
        })
    );
    assert!(written[1].get("clarify").is_none() && written[1].get("pick").is_none());
}

#[cfg(unix)]
#[test]
fn the_log_is_private_and_a_wider_one_is_closed() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("data/platter");
    let path = dir.join(FILE_NAME);
    let log = QuestionLog::new(Some(path.clone()));

    log.append(&entry(1, "a"));

    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&dir), 0o700);

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    log.append(&entry(2, "b"));

    assert_eq!(mode(&path), 0o600);
    assert_eq!(lines(&path).len(), 2);
}

#[test]
fn over_the_cap_the_oldest_lines_go_and_the_file_stays_valid_json_lines() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE_NAME);
    let log = QuestionLog::new(Some(path.clone()));
    let answer = "x".repeat(40_000);
    let count = 60;

    for n in 0..count {
        log.append(&entry(n, &answer));
    }

    let size = std::fs::metadata(&path).unwrap().len() as usize;
    assert!(size <= MAX_BYTES, "{size}");
    let kept = lines(&path);
    assert!(kept.len() < count && kept.len() > 10, "{}", kept.len());

    // The newest ones are the ones kept, in order and without a gap.
    let numbers: Vec<usize> = kept
        .iter()
        .map(|line| {
            line["question"]
                .as_str()
                .unwrap()
                .trim_start_matches("question ")
                .parse()
                .unwrap()
        })
        .collect();
    assert_eq!(*numbers.last().unwrap(), count - 1);
    assert!(
        numbers.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "{numbers:?}"
    );
    assert!(
        std::fs::read_dir(root.path()).unwrap().count() == 1,
        "no temporary file is left"
    );
}

#[test]
fn trimming_starts_at_a_line_and_closes_a_line_that_was_cut_short() {
    let line = |n: usize| format!("{{\"n\":{n}}}\n").into_bytes();
    let new = vec![b'y'; MAX_BYTES / 2];
    let mut newest = new.clone();
    newest.push(b'\n');

    let mut old = Vec::new();
    for n in 0..4 {
        let mut big = vec![b'x'; MAX_BYTES / 4];
        big.push(b'\n');
        old.extend(big);
        old.extend(line(n));
    }
    old.extend(b"{\"cut\":");

    let out = jsonl::trimmed(&old, &newest, MAX_BYTES);

    assert!(out.len() <= MAX_BYTES);
    assert!(out.ends_with(&newest));
    let text = String::from_utf8(out).unwrap();
    for each in text.lines() {
        assert!(
            each.chars().all(|c| c == 'y' || c == 'x') || each.starts_with('{'),
            "a whole line: {}",
            &each[..each.len().min(20)]
        );
    }
    assert!(
        !text.contains("{\"cut\":{") && !text.contains("{\"cut\":x"),
        "the cut line did not run into the next one"
    );
}

#[test]
fn a_line_cut_short_by_an_earlier_failure_does_not_run_into_the_next() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE_NAME);
    std::fs::write(&path, "{\"half\":").unwrap();
    let log = QuestionLog::new(Some(path.clone()));

    log.append(&entry(1, "a"));

    let text = std::fs::read_to_string(&path).unwrap();
    let mut each = text.lines();
    assert_eq!(each.next(), Some("{\"half\":"));
    assert!(serde_json::from_str::<Value>(each.next().unwrap()).is_ok());
}

#[cfg(unix)]
#[test]
fn a_link_at_the_name_is_not_followed() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("precious");
    std::fs::write(&target, b"do not touch").unwrap();
    let path = root.path().join(FILE_NAME);
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let log = QuestionLog::new(Some(path.clone()));

    log.append(&entry(1, "a"));
    // Over the cap too: the rewrite must not go through the link either.
    std::fs::write(&target, vec![b'z'; MAX_BYTES]).unwrap();
    log.append(&entry(2, "b"));

    assert_eq!(std::fs::read(&target).unwrap(), vec![b'z'; MAX_BYTES]);
    assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    assert!(log.clear().is_err(), "a link is left alone");
    assert!(std::fs::symlink_metadata(&path).is_ok());
    assert!(target.exists());

    // A link to a folder is no log either.
    let folder = root.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let other = root.path().join("other.jsonl");
    std::os::unix::fs::symlink(&folder, &other).unwrap();
    QuestionLog::new(Some(other)).append(&entry(3, "c"));
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
}

#[test]
fn clear_deletes_the_log_and_asking_again_is_fine() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data").join(FILE_NAME);
    let log = QuestionLog::new(Some(path.clone()));

    assert!(log.clear().is_ok(), "no log yet");
    log.append(&entry(1, "a"));
    assert!(path.exists());

    assert!(log.clear().is_ok());
    assert!(!path.exists());
    assert!(log.clear().is_ok(), "a second clear finds nothing and is fine");

    log.append(&entry(2, "b"));
    assert_eq!(lines(&path).len(), 1, "the log starts again");
}

#[test]
fn clear_leaves_a_folder_at_the_name_alone_and_without_a_place_it_is_fine() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE_NAME);
    std::fs::create_dir(&path).unwrap();

    assert!(QuestionLog::new(Some(path.clone())).clear().is_err());
    assert!(path.is_dir());
    assert!(QuestionLog::new(None).clear().is_ok());
    QuestionLog::new(None).append(&entry(1, "a"));
}

#[test]
fn a_write_that_fails_is_skipped_without_a_word() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("not-a-folder");
    std::fs::write(&file, b"x").unwrap();

    // The folder of the log is a file; the name of the log is a folder.
    QuestionLog::new(Some(file.join(FILE_NAME))).append(&entry(1, "a"));
    let folder = root.path().join("folder.jsonl");
    std::fs::create_dir(&folder).unwrap();
    QuestionLog::new(Some(folder.clone())).append(&entry(2, "b"));

    assert_eq!(std::fs::read(&file).unwrap(), b"x");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
}

#[test]
fn controls_and_bidi_marks_from_disk_names_are_escaped_and_read_back_the_same() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data/platter").join(FILE_NAME);
    let log = QuestionLog::new(Some(path.clone()));
    let hostile = "~/a\u{9b}31m\u{9d}0;title\u{7}\u{7f}b\u{202e}gpj.exe\u{2028}c\u{1b}[2J";

    let mut written = entry(1, hostile);
    written.meta = vec![hostile.to_owned()];
    written.understood = json!({ "scope": hostile });
    log.append(&written);

    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(
        !text.chars().any(|c| c != '\n' && must_be_escaped(c)),
        "no control or bidi character is written raw: {text:?}"
    );
    assert_eq!(text.lines().count(), 1);
    let line = &lines(&path)[0];
    assert_eq!(line["answer"], hostile);
    assert_eq!(line["meta"][0], hostile);
    assert_eq!(line["understood"]["scope"], hostile);
}

/// The characters the log must never carry raw: controls (C0, DEL, C1) and the format and separator marks.
fn must_be_escaped(c: char) -> bool {
    c.is_control() || matches!(c as u32, 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x206F | 0xFEFF)
}
