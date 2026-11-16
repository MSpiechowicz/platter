//! The evaluation set of the reading (`reading.json`): 60 natural questions, English and Polish, with the
//! reading each must come to (intent, kind, subject). 40 are the set the words were tuned on, 20 are held out
//! and only scored. The same file drives the real-model score in `real.rs`.

use platter_core::Os;
use platter_rules::RuleSet;
use serde::Deserialize;

use super::super::reading::{Reading, read};
use super::*;

#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    pub id: String,
    pub set: String,
    pub question: String,
    pub intent: String,
    pub kind: String,
    pub subject: String,
}

#[derive(Deserialize)]
struct Fixture {
    questions: Vec<Item>,
}

pub fn items() -> Vec<Item> {
    let fixture: Fixture = serde_json::from_str(include_str!("reading.json")).expect("reading.json is valid");

    fixture.questions
}

/// Why `reading` is not what `item` expects; `None` when it is. A folder name is right when it holds the
/// expected one (`Faugus folder` for `Faugus`).
pub fn miss(item: &Item, reading: &Reading) -> Option<String> {
    let subject = reading.subject.symbol();
    let subject_ok = match item.subject.strip_prefix("name:") {
        Some(name) => subject
            .strip_prefix("name:")
            .is_some_and(|said| said.to_lowercase().contains(&name.to_lowercase())),
        None => subject == item.subject,
    };

    let got = (
        reading.query.intent.as_str(),
        reading.query.kind.as_str(),
        subject.as_str(),
    );
    (got.0 != item.intent || got.1 != item.kind || !subject_ok).then(|| {
        format!(
            "{} {:?}: expected {} / {} / {}, read {} / {} / {}",
            item.id, item.question, item.intent, item.kind, item.subject, got.0, got.1, got.2
        )
    })
}

/// How many of each set are right, and the misses.
#[derive(Debug, Default)]
pub struct Score {
    pub tune: (usize, usize),
    pub heldout: (usize, usize),
    pub misses: Vec<String>,
}

impl Score {
    pub fn report(&self) -> String {
        format!(
            "tune {}/{}, held out {}/{}",
            self.tune.0, self.tune.1, self.heldout.0, self.heldout.1
        )
    }
}

pub fn score(read_question: impl Fn(&str) -> Reading) -> Score {
    let mut score = Score::default();

    for item in items() {
        let reading = read_question(&item.question);
        let counts = if item.set == "tune" {
            &mut score.tune
        } else {
            &mut score.heldout
        };

        counts.1 += 1;
        match miss(&item, &reading) {
            None => counts.0 += 1,
            Some(why) => score.misses.push(format!("[{}] {why}", item.set)),
        }
    }

    score
}

/// The reading without a model.
pub fn words_only(question: &str) -> Reading {
    read(question, None, &RuleSet::builtin(), Os::Linux)
}

#[test]
fn the_evaluation_set_has_sixty_questions_forty_to_tune_on_and_twenty_held_out() {
    let items = items();

    assert!(items.len() >= 60);
    assert_eq!(items.iter().filter(|item| item.set == "tune").count(), 40);
    assert_eq!(items.iter().filter(|item| item.set == "heldout").count(), 20);
    for item in &items {
        assert!(Intent::from_word(&item.intent).is_some(), "{}", item.id);
        assert!(Kind::from_word(&item.kind).is_some(), "{}", item.id);
    }

    let questions: Vec<&str> = items.iter().map(|item| item.question.as_str()).collect();
    for owner in [
        "which folder does take the biggest amount of data?",
        "can i safely delete the docker images",
        "czy mogę usunąć obrazy dockera?",
        "write me a poem",
    ] {
        assert!(questions.contains(&owner), "{owner}");
    }
    assert!(
        items.iter().any(|item| item.question.contains("pycache")),
        "a typo or a short question is in the set"
    );
}

#[test]
fn the_words_alone_read_every_question_that_names_no_folder_by_a_bare_name() {
    let score = score(words_only);

    // A bare folder name (`Projekty`, `Faugus`) is what the model is for; everything else is the words'.
    let only_names: Vec<&String> = score
        .misses
        .iter()
        .filter(|miss| !miss.contains("name:"))
        .collect();
    assert!(
        only_names.is_empty(),
        "{}\n{}",
        score.report(),
        only_names
            .iter()
            .map(|miss| miss.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    println!("without a model: {}", score.report());
}
