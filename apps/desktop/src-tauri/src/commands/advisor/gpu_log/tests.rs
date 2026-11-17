use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use serde_json::Value;

use super::*;

const XTX: &str = "AMD Radeon RX 7900 XTX (RADV NAVI31)";

fn incident(on_gpu: bool, what: IncidentKind, at_ms: u64) -> GpuIncident {
    GpuIncident {
        at: UNIX_EPOCH + Duration::from_millis(at_ms),
        card: on_gpu.then(|| XTX.to_owned()),
        on_gpu,
        what,
        exit: Some(Exit::Signal(11)),
        stderr_tail: "radv: GPU hang detected".to_owned(),
        detail: "The helper ended while it was asked to answer.".to_owned(),
    }
}

fn lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("every line is JSON"))
        .collect()
}

fn log_in(root: &tempfile::TempDir) -> (GpuLog, PathBuf) {
    let path = root.path().join("data/platter").join(FILE_NAME);

    (GpuLog::open(path.clone()), path)
}

#[test]
fn an_incident_on_the_card_is_one_line_and_becomes_the_pending_problem() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);

    log.record(&incident(true, IncidentKind::Crash, 1_763_370_000_123));

    let written = lines(&path);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0]["type"], "incident");
    assert_eq!(written[0]["id"], 1_763_370_000_123u64);
    assert_eq!(written[0]["time"], "2025-11-17T09:00:00Z");
    assert_eq!(written[0]["device"], "gpu");
    assert_eq!(written[0]["card"], XTX);
    assert_eq!(written[0]["what"], "crash");
    assert_eq!(written[0]["exit"], serde_json::json!({ "signal": 11 }));
    assert_eq!(written[0]["stderr"], "radv: GPU hang detected");
    let expected = Problem {
        id: 1_763_370_000_123,
        at: 1_763_370_000_123,
        card: Some(XTX.to_owned()),
        what: What::Crash,
    };
    assert_eq!(log.pending(), Some(expected.clone()));
    assert_eq!(log.last(), Some(expected));
}

#[test]
fn an_exit_code_and_a_missing_exit_are_written_as_they_are() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);
    let mut first = incident(true, IncidentKind::Error, 1_000);
    first.exit = Some(Exit::Code(3));
    let mut second = incident(true, IncidentKind::Hang, 2_000);
    second.exit = None;

    log.record(&first);
    log.record(&second);

    let written = lines(&path);
    assert_eq!(written[0]["exit"], serde_json::json!({ "code": 3 }));
    assert_eq!(written[1]["exit"], Value::Null);
}

#[test]
fn an_incident_on_the_processor_is_logged_and_never_asked_about() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);

    log.record(&incident(false, IncidentKind::Crash, 5_000));

    assert_eq!(lines(&path)[0]["device"], "cpu");
    assert_eq!(lines(&path)[0]["card"], Value::Null);
    assert_eq!(log.pending(), None);
    assert_eq!(
        log.last(),
        None,
        "the last problem is the last on a graphics card"
    );
    assert_eq!(GpuLog::open(path).pending(), None);
}

#[test]
fn two_incidents_in_the_same_millisecond_get_different_ids() {
    let root = tempfile::tempdir().unwrap();
    let (log, _) = log_in(&root);

    log.record(&incident(true, IncidentKind::Crash, 9_000));
    let first = log.pending().unwrap().id;
    log.record(&incident(true, IncidentKind::Hang, 9_000));
    let second = log.pending().unwrap();

    assert_ne!(first, second.id);
    assert!(second.id > first);
    assert_eq!(second.at, 9_000, "the time stays the time");
}

#[test]
fn an_incident_waits_for_its_answer_also_at_the_next_start() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);
    log.record(&incident(true, IncidentKind::Hang, 1_000));

    let reopened = GpuLog::open(path.clone());
    assert_eq!(reopened.pending().map(|problem| problem.what), Some(What::Hang));
    assert_eq!(
        reopened.pending().and_then(|problem| problem.card).as_deref(),
        Some(XTX)
    );

    reopened.answer(1_000, Choice::TryAgain).unwrap();

    let answered = GpuLog::open(path);
    assert_eq!(answered.pending(), None);
    assert_eq!(
        answered.last().map(|problem| problem.id),
        Some(1_000),
        "still the last one"
    );
}

#[test]
fn only_a_later_answer_to_the_same_incident_settles_it_and_unreadable_lines_are_skipped() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE_NAME);
    std::fs::write(
        &path,
        concat!(
            "not json at all\n",
            r#"{"type":"incident","id":10,"at":10,"device":"gpu","card":null,"what":"crash"}"#,
            "\n",
            r#"{"type":"answer","id":99,"choice":"useCpu"}"#,
            "\n",
            r#"{"type":"incident","id":20,"device":"cpu","what":"hang"}"#,
            "\n",
            r#"{"type":"incident","id":30,"device":"gpu","what":"melted"}"#,
            "\n",
            r#"{"type":"incident","device":"gpu","what":"hang"}"#,
            "\n",
            r#"{"type":"unknown","id":40}"#,
            "\n",
            r#"{"type":"incident","id":50,"at":50,"device":"gpu","what":"error""#,
        ),
    )
    .unwrap();

    let log = GpuLog::open(path.clone());

    assert_eq!(log.pending().map(|problem| problem.id), Some(10));
    assert_eq!(log.pending().unwrap().card, None);

    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"\n{\"type\":\"answer\",\"id\":10,\"choice\":\"useCpu\"}\n")
        .unwrap();
    assert_eq!(GpuLog::open(path).pending(), None);
}

#[test]
fn a_new_incident_replaces_one_that_was_never_answered() {
    let root = tempfile::tempdir().unwrap();
    let (log, _) = log_in(&root);
    log.record(&incident(true, IncidentKind::Crash, 1_000));
    log.record(&incident(true, IncidentKind::Error, 2_000));

    let stale = log.answer(1_000, Choice::UseCpu);

    assert!(stale.is_err(), "the first is not what waits any more");
    assert_eq!(log.pending().map(|problem| problem.id), Some(2_000));
    assert!(log.answer(2_000, Choice::UseCpu).is_ok());
    assert_eq!(log.pending(), None);
}

#[test]
fn an_answer_that_has_no_problem_waiting_is_refused_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);

    assert!(log.answer(1, Choice::UseCpu).is_err());

    assert!(!path.exists());
}

#[test]
fn answering_writes_a_line_and_the_processor_answer_for_settings_is_silent() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);

    assert!(!log.answer_for_cpu_only(), "nothing waits");
    log.record(&incident(true, IncidentKind::Crash, 1_000));
    assert!(log.answer_for_cpu_only());

    let written = lines(&path);
    assert_eq!(written.len(), 2);
    assert_eq!(written[1]["type"], "answer");
    assert_eq!(written[1]["id"], 1_000);
    assert_eq!(written[1]["choice"], "useCpu");
    assert_eq!(log.pending(), None);
}

#[test]
fn clearing_forgets_the_last_problem_but_a_pending_one_stays_and_can_still_be_answered() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);
    assert!(log.clear().is_ok(), "no log yet");
    log.record(&incident(true, IncidentKind::Crash, 1_000));

    log.clear().unwrap();

    assert!(!path.exists());
    assert_eq!(log.last(), None);
    assert_eq!(log.pending().map(|problem| problem.id), Some(1_000));
    assert!(log.clear().is_ok(), "a second clear finds nothing");

    log.answer(1_000, Choice::TryAgain).unwrap();
    assert_eq!(lines(&path).len(), 1, "the answer made the file again");
    assert_eq!(GpuLog::open(path).pending(), None);
}

#[test]
fn a_log_without_a_place_still_holds_the_problem() {
    let log = GpuLog::new(None);

    log.record(&incident(true, IncidentKind::Hang, 1_000));

    assert!(log.pending().is_some());
    assert!(log.answer(1_000, Choice::UseCpu).is_ok());
    assert!(log.clear().is_ok());
}

#[cfg(unix)]
#[test]
fn the_log_is_private_and_a_wider_one_is_closed() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;

    log.record(&incident(true, IncidentKind::Crash, 1_000));
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    log.record(&incident(true, IncidentKind::Crash, 2_000));

    assert_eq!(mode(&path), 0o600);
}

#[cfg(unix)]
#[test]
fn a_link_at_the_name_is_never_followed() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("precious");
    std::fs::write(&target, b"do not touch").unwrap();
    let path = root.path().join(FILE_NAME);
    std::os::unix::fs::symlink(&target, &path).unwrap();

    let log = GpuLog::open(path.clone());
    log.record(&incident(true, IncidentKind::Crash, 1_000));
    // Over the cap too: the rewrite must not go through the link either.
    std::fs::write(&target, vec![b'z'; MAX_BYTES]).unwrap();
    log.record(&incident(true, IncidentKind::Crash, 2_000));

    assert_eq!(std::fs::read(&target).unwrap(), vec![b'z'; MAX_BYTES]);
    assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    assert!(log.clear().is_err(), "a link is left alone");
    assert!(target.exists());
    assert_eq!(GpuLog::open(path).pending(), None, "a link is no log to read");
    assert!(log.pending().is_some(), "the problem is held all the same");
}

#[test]
fn over_the_cap_the_oldest_lines_go_and_the_file_stays_valid_json_lines() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);
    let mut noisy = incident(false, IncidentKind::Crash, 0);
    noisy.stderr_tail = "x".repeat(2_000);
    let count = 400;

    for n in 0..count {
        noisy.at = UNIX_EPOCH + Duration::from_millis(1_000 + n);
        log.record(&noisy);
    }

    let size = std::fs::metadata(&path).unwrap().len() as usize;
    assert!(size <= MAX_BYTES, "{size}");
    let kept = lines(&path);
    assert!(kept.len() < count as usize && kept.len() > 50, "{}", kept.len());
    let ids: Vec<u64> = kept.iter().map(|line| line["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.last(), Some(&(1_000 + count - 1)), "the newest are kept");
    assert!(ids.windows(2).all(|pair| pair[1] == pair[0] + 1), "without a gap");
}

#[test]
fn at_start_only_the_end_of_a_large_file_is_read() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE_NAME);
    let mut text =
        String::from("{\"type\":\"incident\",\"id\":1,\"at\":1,\"device\":\"gpu\",\"what\":\"crash\"}\n");
    let filler = format!(
        "{{\"type\":\"noise\",\"id\":2,\"pad\":\"{}\"}}\n",
        "y".repeat(1_000)
    );
    while text.len() < 3 * MAX_BYTES {
        text.push_str(&filler);
    }
    std::fs::write(&path, &text).unwrap();

    let log = GpuLog::open(path);

    assert_eq!(log.pending(), None, "the old incident is beyond twice the cap");
}

#[test]
fn a_card_name_read_back_is_cleaned_and_cut() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE_NAME);
    let long = "x".repeat(500);
    let line = serde_json::json!({
        "type": "incident", "id": 10, "at": 10, "device": "gpu", "what": "crash",
        "card": format!("  Card\u{1b}[31m\u{0}\n{long}"),
    });
    std::fs::write(&path, format!("{line}\n")).unwrap();

    let card = GpuLog::open(path).pending().unwrap().card.unwrap();

    assert_eq!(card.chars().count(), 120);
    assert!(card.starts_with("Card[31mxxx"), "{card}");
    assert!(!card.chars().any(char::is_control));
}

#[test]
fn a_card_name_with_nothing_left_after_cleaning_is_no_name() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE_NAME);
    std::fs::write(
        &path,
        "{\"type\":\"incident\",\"id\":10,\"device\":\"gpu\",\"what\":\"crash\",\"card\":\" \\u0007 \"}\n",
    )
    .unwrap();

    assert_eq!(GpuLog::open(path).pending().unwrap().card, None);
}

#[test]
fn an_id_a_script_cannot_hold_is_ignored_when_read_and_never_panics() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE_NAME);
    std::fs::write(
        &path,
        concat!(
            r#"{"type":"incident","id":18446744073709551615,"at":1,"device":"gpu","what":"crash"}"#,
            "\n",
            r#"{"type":"incident","id":9007199254740992,"at":1,"device":"gpu","what":"crash"}"#,
            "\n",
            r#"{"type":"incident","id":7,"at":18446744073709551615,"device":"gpu","what":"hang"}"#,
            "\n",
        ),
    )
    .unwrap();

    let log = GpuLog::open(path);

    let pending = log.pending().unwrap();
    assert_eq!(pending.id, 7, "the greater ids are skipped");
    assert_eq!(pending.at, 7, "a time above the limit is the id");

    log.record(&incident(true, IncidentKind::Crash, 5));
    let next = log.pending().unwrap().id;
    assert!(next > 7 && next < (1 << 53), "{next}");
}

#[test]
fn an_id_far_in_the_future_is_skipped_so_new_ids_stay_unique() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(FILE_NAME);
    std::fs::write(
        &path,
        concat!(
            r#"{"type":"incident","id":9007199254740990,"at":1,"device":"gpu","what":"crash"}"#,
            "\n",
        ),
    )
    .unwrap();

    let log = GpuLog::open(path);
    assert_eq!(
        log.pending(),
        None,
        "an id years ahead was not written by Platter"
    );

    log.record(&incident(true, IncidentKind::Crash, 5));
    let first = log.pending().unwrap().id;
    log.record(&incident(true, IncidentKind::Hang, 6));
    let second = log.pending().unwrap().id;
    assert!(second > first, "{first} then {second}");
}

#[test]
fn the_next_id_cannot_overflow() {
    let log = GpuLog::new(None);
    log.state.lock().unwrap().newest_id = u64::MAX;

    log.record(&incident(true, IncidentKind::Crash, 5));

    assert_eq!(log.pending().unwrap().id, (1 << 53) - 1);
}

#[test]
fn an_overlong_incident_is_cut_and_still_written() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);
    let mut big = incident(true, IncidentKind::Crash, 1_000);
    big.card = Some("c".repeat(MAX_BYTES * 2));
    big.stderr_tail = "\u{1}".repeat(MAX_BYTES * 2);
    big.detail = "d".repeat(MAX_BYTES * 2);

    log.record(&big);

    let written = lines(&path);
    assert_eq!(written.len(), 1, "the incident is not lost");
    assert_eq!(written[0]["card"].as_str().unwrap().chars().count(), 120);
    assert_eq!(written[0]["stderr"].as_str().unwrap().chars().count(), 8192);
    assert_eq!(written[0]["detail"].as_str().unwrap().chars().count(), 8192);
    assert!(std::fs::metadata(&path).unwrap().len() as usize <= MAX_BYTES);
    assert_eq!(GpuLog::open(path).pending().unwrap().id, 1_000);
}

#[test]
fn the_card_name_in_memory_and_in_the_file_is_the_cleaned_one() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);
    let mut named = incident(true, IncidentKind::Hang, 1_000);
    named.card = Some("Card\u{1b}[0m X".to_owned());

    log.record(&named);

    assert_eq!(log.pending().unwrap().card.as_deref(), Some("Card[0m X"));
    assert_eq!(lines(&path)[0]["card"], "Card[0m X");
}

#[test]
fn the_status_reads_do_not_wait_for_a_write_in_progress() {
    let root = tempfile::tempdir().unwrap();
    let (log, _) = log_in(&root);
    log.record(&incident(true, IncidentKind::Crash, 1_000));
    let log = std::sync::Arc::new(log);
    let (done, finished) = std::sync::mpsc::channel();

    let writing = log.lock_writers();
    let reader = {
        let log = std::sync::Arc::clone(&log);
        std::thread::spawn(move || {
            let seen = (log.pending(), log.last());
            done.send(seen).unwrap();
        })
    };
    let seen = finished.recv_timeout(Duration::from_secs(5));
    drop(writing);
    reader.join().unwrap();

    let (pending, last) = seen.expect("the reads wait for no writer");
    assert_eq!(pending.map(|problem| problem.id), Some(1_000));
    assert_eq!(last.map(|problem| problem.id), Some(1_000));
}

#[test]
fn writers_on_many_threads_leave_every_line_whole_and_in_the_order_of_their_ids() {
    let root = tempfile::tempdir().unwrap();
    let (log, path) = log_in(&root);
    let log = std::sync::Arc::new(log);

    let threads: Vec<_> = (0..8)
        .map(|_| {
            let log = std::sync::Arc::clone(&log);
            std::thread::spawn(move || {
                for _ in 0..10 {
                    log.record(&incident(false, IncidentKind::Error, 1_000));
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }

    let ids: Vec<u64> = lines(&path)
        .iter()
        .map(|line| line["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids.len(), 80);
    assert!(ids.windows(2).all(|pair| pair[1] == pair[0] + 1), "{ids:?}");
}
