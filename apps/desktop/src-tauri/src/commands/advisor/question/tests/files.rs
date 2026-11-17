//! A file as the subject of a question: a typed path, a bare name, a name that is a file and a folder, and
//! every intent about one file. Without a model unless a test gives the reading one.

use super::*;

/// The seed with a disk image in Downloads (old, so rules cover it), a file below a folder a rule covers and
/// one no rule covers, two files called `notes` and a folder called `notes`, and a path with white space.
fn with_files() -> FakeFs {
    let mut fs = seeded();
    fs.file("/home/u/Downloads/old.iso", 2 * GIB)
        .set_mtime("/home/u/Downloads/old.iso", NOW - 400 * DAY)
        .file("/home/u/Documents/notes", 5 * MIB)
        .file("/home/u/Projekty/notes/a", MIB)
        .file("/home/u/My Files/a b.txt", 3 * MIB)
        .set_mtime("/home/u/My Files/a b.txt", NOW - 3 * DAY)
        .file("/etc/big/q", 300 * MIB);

    fs
}

fn text_of(collected: &Collected) -> String {
    render(&template_segments(&collected.sheet))
}

fn typed(setup: &Setup, question: &str) -> Collected {
    setup
        .collect_read(question, None, None)
        .expect("the question is collected")
}

const OLD_ISO: &str = "~/Downloads/old.iso";
const OLD_ISO_TEXT: &str = "“~/Downloads/old.iso” is a file of 2.1 GB, last changed 1 year ago.";
const OLD_DOWNLOADS: &str = "It needs your review (“Old downloads”). 3 files in Downloads not modified for \
     365 days or more. These are personal files, so only you can tell whether you still need them.";

fn one_file(collected: &Collected, path: &str) {
    assert_eq!(paths(collected), [path]);
    assert!(collected.rows[0].file, "the row is a file");
    assert!(collected.clarify.is_none());
    assert!(collected.notes.is_empty(), "{:?}", collected.notes);
}

// ---- A typed path ----

#[test]
fn a_typed_path_to_a_file_is_that_file_with_what_the_covering_rule_says() {
    let setup = Setup::new(with_files());

    let collected = typed(&setup, "how big is ~/Downloads/old.iso");

    one_file(&collected, OLD_ISO);
    let scope = collected.understood.scope.as_ref().expect("a scope");
    assert_eq!((scope.path.as_str(), scope.file), (OLD_ISO, true));
    assert_eq!(scope.node, collected.rows[0].node);
    let row = &collected.rows[0];
    assert_eq!(row.verdict, Some(Verdict::Review));
    assert_eq!(row.source, Some(RowSourceDto::Rule));
    assert_eq!(row.note.as_deref(), Some("Old downloads"));
    assert_eq!(text_of(&collected), format!("{OLD_ISO_TEXT} {OLD_DOWNLOADS}"));
}

#[test]
fn the_path_of_the_answer_is_bound_to_the_row_of_the_file() {
    let setup = Setup::new(with_files());

    let collected = typed(&setup, "how big is ~/Downloads/old.iso");
    let segments = template_segments(&collected.sheet);

    assert_eq!(
        segments[0],
        Segment::Path {
            path: OLD_ISO.to_owned(),
            place: platter_ai::Place::Row(0)
        }
    );
    assert!(!render(&segments).contains(';'), "{}", render(&segments));
}

#[test]
fn a_file_below_a_folder_a_rule_covers_has_that_rule_and_one_no_rule_covers_is_the_users_call() {
    let setup = Setup::new(with_files());

    let safe = typed(&setup, "how big is ~/.local/share/Steam/steamapps/shadercache/s");
    assert_eq!(
        text_of(&safe),
        "“~/.local/share/Steam/steamapps/shadercache/s” is a file of 315 MB. It is safe to clean (“Steam \
         shader cache”)."
    );
    assert_eq!(safe.rows[0].verdict, Some(Verdict::Safe));

    let review = typed(&setup, "how big is ~/.cache/huggingface/m.bin");
    assert!(
        text_of(&review).starts_with(
            "“~/.cache/huggingface/m.bin” is a file of 1.1 GB. It needs your review (“Application caches”)."
        ),
        "{}",
        text_of(&review)
    );

    let nobody = typed(&setup, "how big is ~/Backups/phone/img");
    assert_eq!(
        text_of(&nobody),
        "“~/Backups/phone/img” is a file of 524 MB. No rule covers it, so deleting it is your call."
    );
    assert_eq!(nobody.rows[0].verdict, None);
    assert_eq!(nobody.rows[0].source, None);
}

#[test]
fn a_file_in_a_protected_place_is_left_alone_by_the_guard() {
    let setup = Setup::new(with_files());

    let collected = typed(&setup, "how big is /etc/big/q");

    assert_eq!(
        text_of(&collected),
        "“/etc/big/q” is a file of 315 MB. Platter leaves it alone. This is a protected place, or inside one \
         (/etc). Platter does not clean it."
    );
    assert_eq!(collected.rows[0].verdict, Some(Verdict::Keep));
    assert_eq!(collected.rows[0].source, Some(RowSourceDto::Guard));
}

#[test]
fn a_file_the_model_labelled_says_so_and_a_user_rule_on_it_is_kept() {
    let setup = Setup::new(with_files());
    let labels = [label(
        &setup.tree,
        "/home/u/Backups/phone/img",
        ModelVerdict::Review,
        "the model's own words",
    )];

    let collected = setup.collect(
        &query(Intent::Explain, "~/Backups/phone/img", Kind::Any, 5),
        &labels,
    );

    let text = text_of(&collected);
    assert_eq!(
        text,
        "“~/Backups/phone/img” is a file of 524 MB. The local model marked it for review."
    );
    assert!(!text.contains("the model's own words"));
}

#[test]
fn the_age_is_the_scans_time_minus_the_modification_time_and_left_out_without_one() {
    let setup = Setup::new(with_files());

    let recent = typed(&setup, "how big is \"~/My Files/a b.txt\"");
    assert!(
        text_of(&recent).contains("last changed 3 days ago."),
        "{}",
        text_of(&recent)
    );

    let unknown = typed(&setup, "how big is ~/Backups/phone/img");
    assert!(!text_of(&unknown).contains("last changed"));
    assert_eq!(setup.tree.mtime(unknown.rows[0].node), 0);
}

#[test]
fn a_quoted_path_with_white_space_is_one_file() {
    let setup = Setup::new(with_files());

    let collected = typed(&setup, "how big is \"~/My Files/a b.txt\"?");

    one_file(&collected, "~/My Files/a b.txt");
    assert_eq!(
        text_of(&collected),
        "“~/My Files/a b.txt” is a file of 3.1 MB, last changed 3 days ago. No rule covers it, so deleting it \
         is your call."
    );
}

#[test]
fn a_path_that_is_neither_a_folder_nor_a_file_of_the_scan_says_so() {
    let setup = Setup::new(with_files());

    for question in [
        "how big is ~/Downloads/nope.iso",
        "how big is ~/Downloads/old.iso/inside",
    ] {
        let collected = typed(&setup, question);
        let path = question.strip_prefix("how big is ").unwrap();

        assert!(collected.rows.is_empty(), "{question}");
        assert_eq!(
            text_of(&collected),
            format!("No file or folder called “{path}” is in this scan."),
            "{question}"
        );
    }
}

// ---- Every intent ----

#[test]
fn every_intent_but_the_overview_answers_for_the_file_with_one_row() {
    let setup = Setup::new(with_files());
    let expected = format!("{OLD_ISO_TEXT} {OLD_DOWNLOADS}");

    for intent in [
        Intent::Largest,
        Intent::Cleanable,
        Intent::Labelled,
        Intent::Explain,
        Intent::Find,
    ] {
        let collected = setup.collect(&query(intent, OLD_ISO, Kind::Any, 5), &[]);

        one_file(&collected, OLD_ISO);
        assert_eq!(text_of(&collected), expected, "{intent:?}");
        assert_eq!(collected.sheet.rows.len(), 1, "{intent:?}");
    }

    for intent in [Intent::General, Intent::Offtopic] {
        let collected = setup.collect(&query(intent, OLD_ISO, Kind::Any, 5), &[]);

        assert!(collected.rows.iter().all(|row| !row.file), "{intent:?}");
        assert!(collected.understood.scope.is_none(), "{intent:?}");
    }
}

#[test]
fn a_question_about_what_can_be_cleaned_in_a_file_is_answered_for_the_file() {
    let setup = Setup::new(with_files());

    let collected = typed(&setup, "can I delete ~/Downloads/old.iso?");

    one_file(&collected, OLD_ISO);
    assert!(text_of(&collected).starts_with(OLD_ISO_TEXT));
}

// ---- A bare name ----

#[test]
fn a_bare_name_that_is_one_file_is_that_file_whatever_the_case() {
    let setup = Setup::new(with_files());

    for (question, scope) in [
        ("how big is old.iso?", "old.iso"),
        ("how big is OLD.ISO", "OLD.ISO"),
    ] {
        let collected = setup
            .collect_read(question, Some(query(Intent::Largest, scope, Kind::Any, 5)), None)
            .unwrap();

        one_file(&collected, OLD_ISO);
        assert_eq!(
            text_of(&collected),
            format!("{OLD_ISO_TEXT} {OLD_DOWNLOADS}"),
            "{question}"
        );
    }
}

#[test]
fn an_exact_file_beats_folders_that_only_start_with_the_name_or_hold_it() {
    let mut fs = with_files();
    fs.file("/home/u/Backups/old.iso.d/xx", 900 * MIB)
        .file("/home/u/Backups/my-old.iso-copies/y", 800 * MIB);
    let setup = Setup::new(fs);

    let collected = setup
        .collect_read(
            "how big is old.iso?",
            Some(query(Intent::Largest, "old.iso", Kind::Any, 5)),
            None,
        )
        .unwrap();

    one_file(&collected, OLD_ISO);
}

#[test]
fn a_file_is_found_by_its_exact_name_only() {
    let setup = Setup::new(with_files());

    // `old` is a piece of the file's name, so no file, and no folder of that name either.
    let collected = setup
        .collect_read(
            "how big is old?",
            Some(query(Intent::Largest, "old", Kind::Any, 5)),
            None,
        )
        .unwrap();

    assert!(collected.rows.is_empty(), "{:?}", paths(&collected));
    assert_eq!(
        text_of(&collected),
        "No file or folder called “old” is in this scan."
    );
}

fn ask_notes(setup: &Setup, pick: Option<u8>) -> CommandResult<Collected> {
    setup.collect_read(
        "how big is notes?",
        Some(query(Intent::Largest, "notes", Kind::Any, 5)),
        pick,
    )
}

#[test]
fn a_name_that_is_a_file_and_a_folder_is_asked_back_with_the_kind_of_each_choice() {
    let setup = Setup::new(with_files());

    let collected = ask_notes(&setup, None).unwrap();

    let clarify = collected.clarify.as_ref().expect("asked back");
    assert_eq!(
        clarify.prompt,
        "I found more than one file or folder called “notes”. Which one do you mean?"
    );
    let labels: Vec<&str> = clarify
        .choices
        .iter()
        .map(|choice| choice.label.as_str())
        .collect();
    assert_eq!(labels, ["~/Projekty/notes (folder)", "~/Documents/notes (file)"]);
    assert_eq!(
        clarify.choices[0].path.as_deref(),
        Some("~/Projekty/notes"),
        "the path of a choice has no kind in it"
    );
    assert_eq!(
        clarify.choices[1].node,
        Some(path_in(&setup.tree, "/home/u/Documents/notes"))
    );
    assert!(collected.rows.is_empty());
}

#[test]
fn picking_the_file_of_a_mixed_question_answers_for_the_file_and_the_folder_for_the_folder() {
    let setup = Setup::new(with_files());

    let file = ask_notes(&setup, Some(1)).unwrap();
    one_file(&file, "~/Documents/notes");
    assert!(text_of(&file).starts_with("“~/Documents/notes” is a file of 5.2 MB."));

    let folder = ask_notes(&setup, Some(0)).unwrap();
    assert_eq!(folder.sheet.scope, "~/Projekty/notes");
    assert!(folder.sheet.file.is_none());
    assert!(folder.understood.scope.as_ref().is_some_and(|scope| !scope.file));

    let stale = ask_notes(&setup, Some(2)).err().expect("refused");
    assert_eq!(
        serde_json::to_value(&stale).unwrap()["message"],
        "that choice is no longer available, so ask again"
    );
}

#[test]
fn files_of_one_name_and_a_similar_size_are_asked_back_and_a_much_smaller_one_is_not() {
    let mut fs = FakeFs::new();
    fs.file("/home/u/a/photo.raw", 100 * MIB)
        .file("/home/u/b/photo.raw", 90 * MIB)
        .file("/home/u/c/photo.raw", 10 * MIB)
        .file("/home/u/d/solo.raw", 100 * MIB)
        .file("/home/u/e/solo.raw", MIB);
    let setup = Setup::new(fs);
    let ask = |name: &str, pick| {
        setup
            .collect_read(
                &format!("how big is {name}?"),
                Some(query(Intent::Largest, name, Kind::Any, 5)),
                pick,
            )
            .unwrap()
    };

    let several = ask("photo.raw", None);
    let clarify = several.clarify.as_ref().expect("asked back");
    assert_eq!(
        clarify.prompt,
        "I found more than one file called “photo.raw”. Which one do you mean?"
    );
    let labels: Vec<&str> = clarify
        .choices
        .iter()
        .map(|choice| choice.label.as_str())
        .collect();
    assert_eq!(
        labels,
        ["~/a/photo.raw", "~/b/photo.raw"],
        "the small one is no choice"
    );

    let picked = ask("photo.raw", Some(1));
    one_file(&picked, "~/b/photo.raw");

    let largest = ask("solo.raw", None);
    one_file(&largest, "~/d/solo.raw");
}

#[test]
fn a_small_file_is_still_asked_back_when_many_folders_share_its_name() {
    let mut fs = FakeFs::new();
    for place in 0..25 {
        fs.file(&format!("/home/u/p{place:02}/cache/x"), (50 + place) * MIB);
    }
    fs.file("/home/u/Documents/cache", MIB);
    let setup = Setup::new(fs);

    let collected = setup
        .collect_read(
            "how big is cache?",
            Some(query(Intent::Largest, "cache", Kind::Any, 5)),
            None,
        )
        .unwrap();

    let clarify = collected.clarify.expect("asked back");
    assert_eq!(
        clarify.prompt,
        "I found more than one file or folder called “cache”. Which one do you mean?"
    );
    assert!(
        clarify
            .choices
            .iter()
            .any(|choice| choice.label == "~/Documents/cache (file)"),
        "{:?}",
        clarify.choices
    );
}

#[test]
fn at_most_four_choices_are_offered_and_at_least_one_of_each_kind() {
    let mut fs = FakeFs::new();
    for place in ["a", "b", "c", "d", "e", "f"] {
        fs.file(&format!("/home/u/{place}/xx"), 10 * MIB);
    }
    for place in ["g", "h", "i", "j"] {
        fs.file(&format!("/home/u/{place}/xx/inner"), 10 * MIB);
    }
    let setup = Setup::new(fs);

    let collected = setup
        .collect_read(
            "how big is xx?",
            Some(query(Intent::Largest, "xx", Kind::Any, 5)),
            None,
        )
        .unwrap();

    let clarify = collected.clarify.expect("asked back");
    let labels: Vec<&str> = clarify
        .choices
        .iter()
        .map(|choice| choice.label.as_str())
        .collect();
    assert_eq!(labels.len(), 4, "{labels:?}");
    assert_eq!(
        labels.iter().filter(|label| label.ends_with("(folder)")).count(),
        3
    );
    assert_eq!(labels.iter().filter(|label| label.ends_with("(file)")).count(), 1);
    assert!(
        labels[..3].iter().all(|label| label.ends_with("(folder)")),
        "folders first"
    );
}

// ---- Finding ----

#[test]
fn finding_a_name_lists_files_with_the_rows_marked_as_files() {
    let setup = Setup::new(with_files());

    let collected = setup
        .collect_read(
            "where is old.iso?",
            Some(query(Intent::Find, "old.iso", Kind::Any, 5)),
            None,
        )
        .unwrap();

    assert_eq!(paths(&collected), [OLD_ISO]);
    assert!(collected.rows[0].file);
    assert_eq!(collected.rows[0].verdict, Some(Verdict::Review));
    assert_eq!(
        text_of(&collected),
        "Matches for “old.iso”: “~/Downloads/old.iso” (2.1 GB)."
    );

    let mixed = setup
        .collect_read(
            "find notes on my disk",
            Some(query(Intent::Find, "notes", Kind::Any, 5)),
            None,
        )
        .unwrap();
    let kinds: Vec<(&str, bool)> = mixed
        .rows
        .iter()
        .map(|row| (row.path.as_str(), row.file))
        .collect();
    assert_eq!(kinds, [("~/Documents/notes", true), ("~/Projekty/notes", false)]);
}

#[test]
fn the_words_of_a_question_with_a_file_name_make_it_one_about_the_disk() {
    let setup = Setup::new(with_files());
    let review = setup.review(Vec::new());

    let reading = read("where is old.iso?", None, review.rules(), Os::Linux);

    assert_eq!(reading.query.intent, Intent::Find);
    assert_ne!(reading.query.intent, Intent::Offtopic);

    // The model still turns down what is no question about the disk.
    let refused = read(
        "write me a poem, e.g. about spring.",
        Some(query(Intent::Offtopic, "", Kind::Any, 5)),
        review.rules(),
        Os::Linux,
    );
    assert_eq!(refused.query.intent, Intent::Offtopic);
}

#[test]
fn a_missing_file_name_is_said_with_the_new_wording() {
    let setup = Setup::new(with_files());

    let collected = setup
        .collect_read(
            "where is gone.iso?",
            Some(query(Intent::Find, "gone.iso", Kind::Any, 5)),
            None,
        )
        .unwrap();

    assert!(collected.rows.is_empty());
    assert_eq!(
        text_of(&collected),
        "No file or folder called “gone.iso” is in this scan."
    );
}
