//! What a person asking in their own words gets, on seeded trees and without a model: a tool (docker, steam),
//! a personal folder by any of its names, a name that fits two folders, media, a folder with nothing to clean,
//! and the sentences of the answer. The words decide whatever they can; these tests give the reading no
//! model at all, or a model that is wrong, and the answer must still be right.

use super::*;

/// Both Docker places, a Steam library, caches, Trash and the seed: what "docker" must be told apart from.
fn docker_disk() -> FakeFs {
    let mut fs = seeded();
    fs.file("/var/lib/docker/overlay2/l/layer", 8 * GIB)
        .file("/home/u/.docker/desktop/vms/DockerDesktop/disk.raw", 12 * GIB)
        .file("/home/u/.docker/config.json", 1024)
        .file("/home/u/.local/share/Trash/files/old.iso", 2 * GIB)
        .file("/home/u/.npm/_cacache/x", GIB);

    fs
}

fn text_of(collected: &Collected) -> String {
    render(&template_segments(&collected.sheet))
}

fn words_only(setup: &Setup, question: &str) -> Collected {
    setup
        .collect_read(question, None, None)
        .expect("the question is collected")
}

// ---- A tool is the tool and nothing else ----

#[test]
fn docker_is_both_docker_places_and_nothing_else_for_every_intent() {
    let setup = Setup::new(docker_disk());

    for question in [
        "can i safely delete the docker images",
        "czy mogę usunąć obrazy dockera?",
        "how big is my docker data?",
        "where is docker?",
        "why is docker so big?",
    ] {
        let collected = words_only(&setup, question);
        let text = text_of(&collected);

        let mut shown: Vec<&str> = paths(&collected);
        shown.sort_unstable();
        // Why is it big: the biggest part of the largest Docker folder, which is inside Docker.
        if question.starts_with("why") {
            assert!(
                shown.iter().all(|path| path.starts_with("~/.docker/desktop/vms")),
                "{question}: {shown:?}"
            );
            continue;
        }

        assert_eq!(
            shown,
            ["/var/lib/docker", "~/.docker/desktop/vms"],
            "{question}: {text}"
        );
        for other in ["Trash", ".npm", "pacman", ".cache", "Steam", "node_modules"] {
            assert!(!text.contains(other), "{question}: {text}");
        }
        assert!(
            collected
                .sheet
                .totals
                .iter()
                .all(|total| matches!(total.label.as_str(), "Docker" | "Review")
                    || total.kind == TotalKind::Verdict(Verdict::Review)),
            "{question}: {:?}",
            collected.sheet.totals
        );
    }
}

#[test]
fn can_i_delete_the_docker_images_is_one_answer_about_both_docker_places() {
    let setup = Setup::new(docker_disk());

    let collected = words_only(&setup, "can i safely delete the docker images");

    assert_eq!(
        text_of(&collected),
        "No, not without a look: “~/.docker/desktop/vms” (13 GB) and “/var/lib/docker” (8.6 GB) need your \
         review."
    );
    assert!(collected.clarify.is_none());
}

#[test]
fn how_big_is_my_docker_data_leads_with_what_docker_takes() {
    let setup = Setup::new(docker_disk());

    let text = text_of(&words_only(&setup, "how big is my docker data?"));

    assert!(
        text.starts_with("Docker: 21 GB in 2 items. The biggest folder among the “Docker” folders is “~/.docker/desktop/vms” (13 GB), then “/var/lib/docker” (8.6 GB)."),
        "{text}"
    );
}

#[test]
fn a_tool_that_is_not_in_the_scan_says_only_that_and_where_it_lives() {
    let setup = Setup::new(seeded());

    for question in [
        "how big is my docker data?",
        "can I delete the docker containers?",
    ] {
        let collected = words_only(&setup, question);

        assert!(collected.rows.is_empty(), "{question}");
        assert!(collected.sheet.totals.is_empty(), "{question}");
        let text = text_of(&collected);
        assert!(
            text.starts_with(
                "No file or folder called “docker” is in this scan (“Docker data” lives in /var/lib/docker,"
            ),
            "{question}: {text}"
        );
        assert!(
            !text.contains("cache") && !text.contains("Trash"),
            "{question}: {text}"
        );
    }
}

#[test]
fn a_tool_with_a_fixed_place_outside_a_home_scan_says_where_the_rest_is() {
    let note = "“/var/lib/docker” (Docker data) is outside this scan. Scan / to include it.";

    // A home scan holds the Docker Desktop disk, and Docker's own folder is outside it.
    let home = Setup::rooted(docker_disk(), "/home/u");
    for question in [
        "how big is my docker data?",
        "can i safely delete the docker images",
    ] {
        let collected = words_only(&home, question);
        let text = text_of(&collected);

        assert!(text.contains("~/.docker/desktop/vms"), "{question}: {text}");
        assert!(
            collected.rows.iter().all(|row| row.path != "/var/lib/docker"),
            "{question}"
        );
        assert!(text.ends_with(note), "{question}: {text}");
        assert_eq!(
            collected
                .notes
                .iter()
                .filter(|shown| shown.as_str() == note)
                .count(),
            1,
            "{question}"
        );
    }

    // A scan of everything holds both places: nothing is outside it.
    let whole = Setup::new(docker_disk());
    assert!(!text_of(&words_only(&whole, "how big is my docker data?")).contains("is outside this scan"));

    // A tool not found at all says where it lives instead (the existing answer), not this note.
    let none = Setup::rooted(seeded(), "/home/u");
    let nothing = text_of(&words_only(&none, "how big is my docker data?"));
    assert!(
        nothing.starts_with("No file or folder called “docker” is in this scan"),
        "{nothing}"
    );
    assert!(!nothing.contains("is outside this scan"), "{nothing}");
}

#[test]
fn an_unreadable_docker_place_is_listed_and_says_it_could_not_be_read() {
    let mut fs = seeded();
    fs.unreadable_dir("/var/lib/docker");
    let setup = Setup::new(fs);

    let text = text_of(&words_only(&setup, "where is docker?"));

    assert_eq!(
        text,
        "Matches for “docker”: “/var/lib/docker” (could not be read)."
    );
    let verdict = text_of(&words_only(&setup, "can I delete docker?"));
    assert_eq!(
        verdict,
        "No: “/var/lib/docker” is left alone by Platter. Platter could not read it, so its size is unknown."
    );
}

#[test]
fn steam_and_rust_are_their_rules_folders_by_any_wording() {
    let setup = Setup::new(seeded());

    let steam = words_only(&setup, "how much space does steam use?");
    assert!(
        text_of(&steam)
            .starts_with("Steam: 4.6 GB in 2 items. The biggest folder among the “Steam” folders is "),
        "{}",
        text_of(&steam)
    );
    assert_eq!(
        paths(&steam),
        [
            "~/.local/share/Steam/steamapps/common",
            "~/.local/share/Steam/steamapps/shadercache"
        ]
    );

    let rust = words_only(&setup, "where are my rust build folders?");
    assert_eq!(paths(&rust), ["~/Projekty/app/target", "~/Projekty/live/target"]);
    assert_eq!(rust.understood.intent, ScanIntentDto::Find);
    assert!(
        text_of(&rust).starts_with("Matches for “rust”: "),
        "{}",
        text_of(&rust)
    );
}

#[test]
fn a_model_that_reads_the_wrong_scope_cannot_turn_a_tool_into_a_folder_name() {
    let setup = Setup::new(docker_disk());
    // "obrazy" is a Polish folder name (Pictures), and the model took it for the subject.
    let wrong = query(Intent::Cleanable, "obrazy", Kind::Media, 5);

    let collected = setup
        .collect_read("czy mogę usunąć obrazy dockera?", Some(wrong), None)
        .unwrap();

    assert_eq!(paths(&collected), ["~/.docker/desktop/vms", "/var/lib/docker"]);
    assert_eq!(collected.understood.kind, ScanKindDto::Any);
}

// ---- Personal folders go through the user's own ----

fn polish_home() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/Pobrane/big.iso", 700 * MIB)
        .file("/home/u/Pobrane/notes.txt", MIB)
        .file("/home/u/Dokumenty/work/a.odt", 4 * MIB)
        .file("/home/u/.cache/platter-review4/t/home/me/Downloads/x", 16 * 1024)
        .file("/home/u/Documents/Codex/a.md", 4096);

    fs
}

fn polish_dirs() -> Vec<(WellKnown, PathBuf)> {
    vec![(WellKnown::Downloads, PathBuf::from("/home/u/Pobrane"))]
}

#[test]
fn downloads_is_the_users_own_downloads_folder_and_a_namesake_elsewhere_is_never_it() {
    let setup = Setup::new(polish_home());

    for question in [
        "can I delete my Downloads?",
        "can I delete my old downloads?",
        "czy mogę usunąć Pobrane",
    ] {
        let collected = setup
            .collect_read_with(question, None, None, polish_dirs())
            .unwrap();
        let text = text_of(&collected);

        assert!(
            collected.clarify.is_none(),
            "{question}: the user dir wins, nothing is asked"
        );
        assert_eq!(collected.sheet.scope, "~/Pobrane", "{question}: {text}");
        assert!(!text.contains(".cache"), "{question}: {text}");
    }

    // An English name for a folder that has another name here is told.
    let collected = setup
        .collect_read_with("can I delete my Downloads?", None, None, polish_dirs())
        .unwrap();
    assert!(
        collected
            .notes
            .contains(&"There is no “~/Downloads”. Your downloads folder is “~/Pobrane”.".to_owned()),
        "{:?}",
        collected.notes
    );
    let polish = setup
        .collect_read_with("czy mogę usunąć Pobrane", None, None, polish_dirs())
        .unwrap();
    assert!(
        !polish.notes.iter().any(|note| note.contains("There is no")),
        "the user used the folder's own name: {:?}",
        polish.notes
    );
}

#[test]
fn a_file_in_the_users_own_downloads_folder_is_found_by_its_path_and_by_its_name() {
    let setup = Setup::new(polish_home());

    let by_path = setup
        .collect_read_with("how big is ~/Pobrane/big.iso", None, None, polish_dirs())
        .unwrap();
    let by_name = setup
        .collect_read_with(
            "how big is big.iso?",
            Some(query(Intent::Largest, "big.iso", Kind::Any, 5)),
            None,
            polish_dirs(),
        )
        .unwrap();

    for collected in [by_path, by_name] {
        assert_eq!(paths(&collected), ["~/Pobrane/big.iso"]);
        assert!(collected.rows[0].file);
        assert!(
            text_of(&collected).starts_with("“~/Pobrane/big.iso” is a file of 734 MB."),
            "{}",
            text_of(&collected)
        );
    }
}

#[test]
fn a_personal_folder_that_is_not_in_the_scan_is_missing_and_never_a_namesake() {
    // A scan of a part of the home that does not hold Pobrane, but holds a folder called Downloads.
    let mut fs = FakeFs::new();
    fs.file("/home/u/.cache/fixture/Downloads/x", 16 * 1024)
        .file("/home/u/Projekty/a", MIB);
    let setup = Setup::new(fs);

    let collected = setup
        .collect_read_with("can I delete my Downloads?", None, None, polish_dirs())
        .unwrap();

    assert!(collected.rows.is_empty());
    assert_eq!(
        text_of(&collected),
        "No file or folder called “Downloads” is in this scan (your downloads folder is “~/Pobrane”)."
    );
}

#[test]
fn documents_and_pictures_follow_the_environments_own_folders() {
    let mut fs = polish_home();
    fs.file("/home/u/Obrazy/a.png", MIB);
    let tree = scan_fake(&fs, false);
    let env = Environment {
        user_dirs: vec![
            (
                platter_core::UserDir::Documents,
                PathBuf::from("/home/u/Dokumenty"),
            ),
            (platter_core::UserDir::Pictures, PathBuf::from("/home/u/Obrazy")),
        ],
        ..home_env()
    };
    let review = Review::for_environment(&tree, env, &fs, Vec::new());
    let lens = Lens::new(&review, &[]);

    let folder = |question: &str| {
        let reading = read(question, None, review.rules(), Os::Linux);
        match lens.resolve(&reading.subject, None).unwrap().scope {
            Scope::Folder { path, .. } => path,
            other => format!("{other:?}"),
        }
    };

    assert_eq!(folder("what is in my Documents folder?"), "~/Dokumenty");
    assert_eq!(folder("what is in my Dokumenty folder?"), "~/Dokumenty");
    assert_eq!(folder("can I delete my Pictures?"), "~/Obrazy");
}

#[test]
fn an_explicit_path_beats_a_personal_folder_name_and_says_where_the_users_folder_is() {
    let mut fs = polish_home();
    fs.file("/home/u/Dokumenty/b", MIB);
    let tree = scan_fake(&fs, false);
    let env = Environment {
        user_dirs: vec![(
            platter_core::UserDir::Documents,
            PathBuf::from("/home/u/Dokumenty"),
        )],
        ..home_env()
    };
    let review = Review::for_environment(&tree, env, &fs, Vec::new());
    let asked_dirs = polish_dirs();
    let q = query(Intent::Cleanable, "", Kind::Any, 5);
    let reading = read("is ~/Documents safe to delete?", None, review.rules(), Os::Linux);
    let asked = Asked {
        query: &reading.query,
        units: Units::Decimal,
        outdated: false,
        subject: reading.subject.clone(),
        media: None,
        polish: false,
        everything: false,
        pick: None,
        dirs: asked_dirs,
    };
    let _ = q;

    let collected = collect(&review, &[], None, &asked).unwrap();

    assert_eq!(
        collected.sheet.scope, "~/Documents",
        "the path typed is the folder"
    );
    assert_eq!(
        text_of(&collected),
        "“~/Documents” holds 4.1 KB in 1 file. Platter has no rule for it. Deleting it is your call. Your \
         documents folder is “~/Dokumenty”."
    );
}

// ---- A name that fits two folders is asked back ----

fn two_backups() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/work/backup/x", 300 * MIB)
        .file("/home/u/phone/backup/y", 250 * MIB)
        .file("/home/u/tiny/backup/z", MIB)
        .file("/home/u/Projekty/a", MIB);

    fs
}

#[test]
fn a_name_that_fits_two_folders_of_a_similar_size_is_asked_back_with_both_and_no_whole_scan() {
    let setup = Setup::new(two_backups());
    let model = query(Intent::Cleanable, "backup", Kind::Any, 5);

    let collected = setup
        .collect_read("can I delete the backup folder?", Some(model), None)
        .unwrap();

    let clarify = collected.clarify.expect("asked back");
    assert_eq!(
        clarify.prompt,
        "I found more than one folder called “backup”. Which one do you mean?"
    );
    assert_eq!(
        clarify
            .choices
            .iter()
            .map(|choice| choice.label.as_str())
            .collect::<Vec<_>>(),
        ["~/work/backup", "~/phone/backup"],
        "the tiny one is no peer, and there is no choice of the whole scan: a folder was named"
    );
    assert!(collected.rows.is_empty() && collected.sheet.rows.is_empty());
    assert!(
        clarify
            .choices
            .iter()
            .all(|choice| choice.node.is_some() && choice.path.is_some())
    );
}

#[test]
fn a_pick_takes_the_choice_of_the_same_question_and_a_stale_pick_is_refused() {
    let setup = Setup::new(two_backups());
    let model = || Some(query(Intent::Cleanable, "backup", Kind::Any, 5));
    let ask = |pick: Option<u8>| setup.collect_read("can I delete the backup folder?", model(), pick);

    let second = ask(Some(1)).unwrap();
    assert!(second.clarify.is_none());
    assert_eq!(second.sheet.scope, "~/phone/backup");
    assert_eq!(
        second.understood.scope.as_ref().map(|scope| scope.path.as_str()),
        Some("~/phone/backup")
    );
    assert_eq!(ask(Some(0)).unwrap().sheet.scope, "~/work/backup");

    // A choice that is not there, and a pick for a question that is not asked back.
    for refused in [
        ask(Some(2)),
        setup.collect_read("can I delete ~/work/backup?", None, Some(0)),
    ] {
        let error = refused.err().expect("refused");
        assert_eq!(
            serde_json::to_value(&error).unwrap()["message"],
            "that choice is no longer available, so ask again"
        );
    }
}

#[test]
fn a_name_with_one_clear_largest_folder_is_not_asked_back() {
    let mut fs = two_backups();
    fs.file("/home/u/big/backup/w", 5 * GIB);
    let setup = Setup::new(fs);

    let collected = setup
        .collect_read(
            "can I delete the backup folder?",
            Some(query(Intent::Cleanable, "backup", Kind::Any, 5)),
            None,
        )
        .unwrap();

    assert!(collected.clarify.is_none());
    assert_eq!(collected.sheet.scope, "~/big/backup");
}

#[test]
fn two_unrelated_tools_that_fit_the_words_are_asked_back() {
    let mut fs = seeded();
    fs.file("/home/u/go/pkg/mod/github.com/x/y.go", 3 * GIB);
    let setup = Setup::new(fs);
    let model = query(Intent::Cleanable, "modules", Kind::Any, 5);

    let collected = setup
        .collect_read("can I delete the modules?", Some(model), None)
        .unwrap();

    let clarify = collected.clarify.expect("two different tools fit");
    let labels: Vec<&str> = clarify
        .choices
        .iter()
        .map(|choice| choice.label.as_str())
        .collect();
    assert!(labels.contains(&"node_modules"), "{labels:?}");
    assert!(labels.contains(&"Go module cache"), "{labels:?}");
    assert!(collected.rows.is_empty());

    // Picking one answers about that tool alone.
    let go = clarify
        .choices
        .iter()
        .position(|choice| choice.label == "Go module cache")
        .unwrap();
    let picked = setup
        .collect_read(
            "can I delete the modules?",
            Some(query(Intent::Cleanable, "modules", Kind::Any, 5)),
            Some(go as u8),
        )
        .unwrap();
    assert_eq!(paths(&picked), ["~/go/pkg/mod"]);
}

// ---- Media ----

fn media_disk() -> FakeFs {
    let mut fs = seeded();
    fs.file("/home/u/Pictures/trip/a.jpg", 50 * MIB)
        .file("/home/u/Pictures/b.png", 10 * MIB)
        .file("/home/u/Music/album/a.mp3", 40 * MIB);

    fs
}

#[test]
fn how_much_do_my_videos_take_leads_with_the_video_total_and_lists_video_folders_only() {
    let setup = Setup::new(media_disk());

    for question in ["how much do my videos take?", "ile miejsca zajmują filmy?"] {
        let collected = words_only(&setup, question);
        let text = text_of(&collected);

        assert_eq!(paths(&collected), ["~/Videos"], "{question}");
        assert!(
            text.starts_with("Video folders in this scan take 3.3 GB in total (1 folder). The biggest video folder in this scan is “~/Videos” (3.3 GB)."),
            "{question}: {text}"
        );
        assert!(
            !text.contains("Pictures") && !text.contains("Music") && !text.contains("Images"),
            "{text}"
        );
    }

    let photos = text_of(&words_only(&setup, "how large are my photos?"));
    assert!(photos.starts_with("Image folders in this scan take "), "{photos}");
    assert!(
        photos.contains("“~/Pictures”") && !photos.contains("Videos"),
        "{photos}"
    );

    let music = words_only(&setup, "how much does my music take up?");
    assert_eq!(paths(&music), ["~/Music"]);
    assert!(
        text_of(&music).starts_with("Audio folders in this scan take "),
        "{}",
        text_of(&music)
    );
}

#[test]
fn a_media_category_that_is_not_in_the_scan_says_so() {
    let setup = Setup::new(polish_home());

    let text = text_of(&words_only(&setup, "how much do my videos take?"));

    assert_eq!(text, "No video folders were found in this scan.");
}

// ---- A folder with nothing to clean ----

#[test]
fn a_folder_with_nothing_to_clean_says_what_it_holds_and_where_platter_stands() {
    let mut fs = seeded();
    fs.file("/home/u/Notes/n.txt", 4 * MIB)
        .file("/home/u/Games/Steam/steamapps/common/g/data", 2 * GIB);
    let setup = Setup::new(fs);

    let none = text_of(&words_only(&setup, "is ~/Notes safe to delete?"));
    assert_eq!(
        none,
        "“~/Notes” holds 4.2 MB in 1 file. Platter has no rule for it. Deleting it is your call."
    );

    // A rule keeps something inside it (Steam's games).
    let mut fs = FakeFs::new();
    fs.file("/home/u/.local/share/Steam/steamapps/common/g/data", 2 * GIB)
        .file("/home/u/Projekty/a", MIB);
    let setup = Setup::new(fs);
    let kept = text_of(&words_only(
        &setup,
        "can I delete ~/.local/share/Steam/steamapps?",
    ));
    assert!(
        kept.contains(
            "Platter keeps part of it, such as “Steam library” (2.1 GB). Deleting the rest is your call."
        ),
        "{kept}"
    );
    assert!(!kept.contains("Nothing to clean"), "{kept}");
}

// ---- The sentences ----

fn many_caches() -> FakeFs {
    let mut fs = FakeFs::new();
    fs.file("/home/u/.npm/_cacache/x", 5 * GIB)
        .file("/home/u/.bun/install/cache/x", 4 * GIB)
        .file("/home/u/.local/share/Trash/files/x", 3 * GIB)
        .file("/home/u/.cache/huggingface/m.bin", 8 * GIB)
        .file("/var/cache/pacman/pkg/a.zst", 2 * GIB)
        .file("/home/u/.docker/desktop/vms/vm/disk.raw", 20 * GIB);
    for number in 0..40 {
        fs.file(&format!("/home/u/Projekty/p{number}/m/__pycache__/x.pyc"), 30_000);
    }

    fs
}

#[test]
fn a_generic_answer_collapses_the_small_items_and_names_the_biggest_to_review() {
    let setup = Setup::new(many_caches());

    let collected = words_only(&setup, "what can I clean up?");
    let text = text_of(&collected);

    assert!(
        text.starts_with(
            "Safe to clean: 13 GB in 43 items, the largest being “~/.npm” (5.4 GB), “~/.bun/install/cache” \
             (4.3 GB), “~/.local/share/Trash” (3.2 GB) and 40 smaller items such as “__pycache__”. For you to \
             review: 32 GB in 3 items, the largest being “~/.docker/desktop/vms” (21 GB), “~/.cache” (8.6 GB) \
             and 1 more item."
        ),
        "{text}"
    );
    assert!(
        !text.contains("not listed"),
        "the count of unlisted matches is `more`, not text: {text}"
    );
    assert!(collected.more > 0);
    assert_eq!(
        collected.rows.len(),
        5,
        "the limit is shared between safe and review"
    );
}

#[test]
fn the_rows_of_a_limited_subject_answer_say_how_many_more_match() {
    let mut fs = FakeFs::new();
    for number in 0..8 {
        fs.file(&format!("/home/u/Projekty/p{number}/package.json"), 1);
        fs.file(&format!("/home/u/Projekty/p{number}/package-lock.json"), 1);
        fs.file(
            &format!("/home/u/Projekty/p{number}/node_modules/m/x"),
            (10 + number) * MIB,
        );
    }
    let setup = Setup::new(fs);

    let text = text_of(&words_only(&setup, "can I delete node_modules?"));

    assert!(
        text.starts_with("Yes: “~/Projekty/p7/node_modules” (18 MB), "),
        "{text}"
    );
    assert!(
        text.contains(" are safe to clean. Only the largest 5 are named here, and 3 more match."),
        "{text}"
    );
}

#[test]
fn a_polish_question_and_a_wish_to_clean_everything_each_get_one_short_note() {
    let setup = Setup::new(seeded());

    let polish = words_only(&setup, "co mogę usunąć?");
    assert_eq!(
        polish
            .notes
            .iter()
            .filter(|note| *note == "Answers are in English.")
            .count(),
        1
    );
    assert!(
        text_of(&polish).ends_with(" Answers are in English."),
        "{}",
        text_of(&polish)
    );
    assert!(!polish.notes.iter().any(|note| note.contains("Cleanup")));

    let everything = words_only(&setup, "clean everything");
    assert_eq!(
        everything
            .notes
            .iter()
            .filter(|note| *note == "Platter does not delete anything from here. Use Cleanup to choose.")
            .count(),
        1
    );
    assert!(!everything.notes.iter().any(|note| note.contains("English")));

    let plain = words_only(&setup, "what can I clean up?");
    assert!(
        !plain
            .notes
            .iter()
            .any(|note| note.contains("English") || note.contains("Cleanup")),
        "{:?}",
        plain.notes
    );
}

#[test]
fn a_question_without_a_word_about_disks_is_off_topic_whatever_the_model_read() {
    let setup = Setup::new(seeded());

    for model in [
        query(Intent::General, "", Kind::Any, 5),
        query(Intent::Largest, "~/Projekty", Kind::Any, 5),
        query(Intent::Cleanable, "", Kind::Cache, 5),
    ] {
        let collected = setup.collect_read("write me a poem", Some(model), None).unwrap();

        assert_eq!(collected.understood.intent, ScanIntentDto::Offtopic);
        assert!(collected.rows.is_empty());
        assert_eq!(text_of(&collected), OFFTOPIC_TEXT);
    }
}

#[test]
fn the_words_win_over_a_model_that_reads_the_wrong_intent_and_scope() {
    let setup = Setup::new(seeded());
    let wrong = query(Intent::Explain, "~/Videos", Kind::Cache, 9);

    let collected = setup
        .collect_read(
            "which folder does take the biggest amount of data?",
            Some(wrong),
            None,
        )
        .unwrap();

    assert_eq!(collected.understood.intent, ScanIntentDto::Largest);
    assert_eq!(
        collected.understood.scope, None,
        "the model's scope is not in the question"
    );
    assert_eq!(collected.understood.kind, ScanKindDto::Any);
}

#[test]
fn what_the_model_did_not_label_is_said_once_with_the_note_for_what_to_do() {
    let setup = Setup::new(seeded());

    let collected = words_only(&setup, "what did the model label?");

    assert_eq!(collected.notes, ["Use “Label folders” in the Advisor view."]);
    assert_eq!(
        text_of(&collected),
        "The model has not labelled any folder in this scan. Use “Label folders” in the Advisor view."
    );
}
