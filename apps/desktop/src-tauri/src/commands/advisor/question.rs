//! `ask_scan`: a free question about a finished scan (A5.3; docs/architecture.md, "Questions about a scan").
//!
//! The words of the question are read first ([`reading`]): a clear intent word, a kind, a media category, a
//! typed path, a personal folder or a tool of the rules each decide what they can, and the model only fills
//! what they leave open (and is not asked at all when they settle the intent and the subject). The backend
//! resolves the subject in the finished tree ([`resolve`]: a tool of the rules, a personal folder as the user
//! has it, a path, a name; a subject that fits two things is asked back, never guessed), computes the rows
//! and the [`FactSheet`] from one list, and Platter writes the answer from the sheet with the template: no
//! word of the answer is the model's. A named subject is strict: nothing outside it is answered. A verdict
//! comes only from the guard, the rules, the user's own rules or an earlier model label (see [`Lens::row`]).
//! The question is text only, never a path. Everything that can name a folder is looked up in the tree and
//! nowhere else. Each question is also written to a local log ([`super::log`]) unless the user switched that
//! off: with how the words and the model read it, and the sentences about the reading that are kept out of
//! the answer's text (`Collected::meta`: the rule that covers a folder, how many folders matched).

use std::cell::OnceCell;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use platter_ai::{
    By, FactRow, FactSheet, FactTotal, FileFact, Intent, Kind, MediaKind, OUTDATED_NOTE, Place, ScanQuestion,
    Segment, TotalKind, Units, WellKnown, clean_question, format_size, quote_name, render, template_segments,
};
use platter_core::dto::{AgeBucket, ScanId, Verdict, VerdictTotals};
use platter_core::query::{MAX_ENTRY_MATCHES, find_entries, find_folders, summary};
use platter_core::{Category, ModelLabel, ModelVerdict, NodeId, NodeKind, Os, Tree};
use platter_rules::{Matcher, Rule, RuleMatches, RuleSet, matches, scan_is_kept, scan_root_is_claimed};
use serde::{Deserialize, Serialize};
use tauri::State;

use self::reading::{Subject, read};
use self::record::Trace;
use self::resolve::{Choice, ChoiceKey, Clarify, Resolved};
use super::{GUARD_DETAIL, ModelRead, Review, clamp};
use crate::commands::suggestions;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;
use crate::user_rules;

/// Caches: the rules of the built-in table whose folders the model's `cache` kind means.
pub const CACHE_FAMILY: [&str; 15] = [
    "apt-cache",
    "dnf-cache",
    "npm-cache",
    "bun-cache",
    "pnpm-store",
    "yarn-cache",
    "cargo-registry",
    "gradle-cache",
    "nuget-packages",
    "go-modules",
    "steam-shader-cache",
    "pacman-cache",
    "app-cache",
    "maven-repository",
    "python-cache",
];

/// Build output and dependency folders: what the `build` kind means.
pub const BUILD_FAMILY: [&str; 4] = [
    "rust-target",
    "rust-target-recent",
    "node-modules",
    "node-modules-recent",
];

/// The categories the `media` kind means.
const MEDIA: [Category; 3] = [Category::Video, Category::Image, Category::Audio];

/// Most rows an answer shows (a question's limit is at most this).
const MAX_ROWS: usize = 10;
/// Seconds in a day, for the age of a file.
const SECONDS_PER_DAY: u64 = 86_400;
/// Most notes an answer carries.
const MAX_NOTES: usize = 6;
/// Longest note, in characters.
const NOTE_CHARS: usize = 240;

const KEPT_SCAN_NOTE: &str = "This scan starts inside a folder you keep, so nothing in it is suggested.";
const LABELLED_BY_MODEL: &str = "Labelled by the local model";
/// What to do about it, when the model has labelled nothing (the answer itself says that nothing is labelled).
/// The note of a question the model was asked to read and could not (it ran out of time or wrote no query),
/// when the words name nothing either: the answer is about the whole scan, and says so.
pub(crate) const UNREAD_NOTE: &str =
    "Platter could not tell which part of the scan you meant, so this answer covers the whole scan.";
const NO_LABELS_NOTE: &str = "Use “Label folders” in the Advisor view.";
const ENGLISH_NOTE: &str = "Answers are in English.";
const NOT_FROM_HERE_NOTE: &str = "Platter does not delete anything from here. Use Cleanup to choose.";

// ---- Wire types ----

/// How sizes are written in the answer's text. The only type of this command the web view sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SizeUnitsDto {
    #[serde(rename = "GB")]
    Gb,
    #[serde(rename = "GiB")]
    Gib,
}

impl From<SizeUnitsDto> for Units {
    fn from(units: SizeUnitsDto) -> Self {
        match units {
            SizeUnitsDto::Gb => Self::Decimal,
            SizeUnitsDto::Gib => Self::Binary,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanIntentDto {
    Largest,
    Cleanable,
    Labelled,
    Explain,
    Find,
    General,
    Offtopic,
}

impl From<Intent> for ScanIntentDto {
    fn from(intent: Intent) -> Self {
        match intent {
            Intent::Largest => Self::Largest,
            Intent::Cleanable => Self::Cleanable,
            Intent::Labelled => Self::Labelled,
            Intent::Explain => Self::Explain,
            Intent::Find => Self::Find,
            Intent::General => Self::General,
            Intent::Offtopic => Self::Offtopic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScanKindDto {
    Any,
    Cache,
    Build,
    Media,
}

impl From<Kind> for ScanKindDto {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Any => Self::Any,
            Kind::Cache => Self::Cache,
            Kind::Build => Self::Build,
            Kind::Media => Self::Media,
        }
    }
}

/// Who made a row's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RowSourceDto {
    Guard,
    Rule,
    /// One of the user's own rules.
    User,
    Model,
}

impl From<RowSourceDto> for By {
    fn from(source: RowSourceDto) -> Self {
        match source {
            RowSourceDto::Guard => Self::Guard,
            RowSourceDto::Rule => Self::Rule,
            RowSourceDto::User => Self::User,
            RowSourceDto::Model => Self::Model,
        }
    }
}

/// The folder (or file) the question's scope was resolved to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeDto {
    pub node: NodeId,
    pub path: String,
    /// The scope is a file. Left out for a folder, so an answer about a folder reads as it always did.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub file: bool,
}

/// What the backend made of the question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnderstoodDto {
    pub intent: ScanIntentDto,
    pub kind: ScanKindDto,
    pub limit: u8,
    /// The folder the scope was resolved to; left out for the whole scan, and when it was not found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeDto>,
    /// The text of a scope that is not in the scan (or, for `find`, the name looked for).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_text: Option<String>,
}

/// One folder (or file) of the answer, computed by the backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRowDto {
    pub node: NodeId,
    pub path: String,
    pub allocated: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<RowSourceDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The row is a file, not a folder. Left out for a folder.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub file: bool,
}

/// A piece of the answer's text. A path of the scan (a row or the scope) comes as its own segment, with
/// the verdict Platter itself gives that folder (never the model's), so a reader can show it as a verdict
/// chip. `verdict` and `source` are left out when nobody has an opinion on the folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AnswerSegmentDto {
    Text {
        text: String,
    },
    Path {
        node: NodeId,
        /// The path as the answer says it: the display path, a long one cut with `…` in the middle.
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        verdict: Option<Verdict>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<RowSourceDto>,
    },
}

/// One way to read a question that could mean more than one thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChoiceDto {
    /// What `pick` of the next `ask_scan` must be to take this one.
    pub id: u8,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
}

/// A question asked back: the words fit more than one folder or tool, and Platter does not guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarifyDto {
    pub prompt: String,
    pub choices: Vec<ChoiceDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanAnswerDto {
    pub scan: ScanId,
    pub understood: UnderstoodDto,
    /// The whole answer as text, written by Platter from the scan, every path of the scan in “ ” quotes (a
    /// “ ” or `"` in a name is shown as ‟ or 〃): the text of `segments` with each path quoted.
    pub answer: String,
    /// The answer in order: text, and the paths of the scan as segments that point at their folder.
    pub segments: Vec<AnswerSegmentDto>,
    /// At most 10.
    pub rows: Vec<ScanRowDto>,
    /// Matches that are not among the rows.
    pub more: u64,
    pub notes: Vec<String>,
    /// Set when the question could mean more than one thing: `answer` is then the prompt, the rows are
    /// empty, and asking again with the `pick` of one choice gives the answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clarify: Option<ClarifyDto>,
}

// ---- The command ----

/// Answers a free question about a finished scan: the model reads it, the backend finds the rows and the
/// facts, Platter writes the answer from them. A few seconds of work (the model's), so off the async
/// runtime. See docs/architecture.md, "Questions about a scan".
#[tauri::command]
pub async fn ask_scan(
    state: State<'_, AppState>,
    scan: ScanId,
    question: String,
    units: SizeUnitsDto,
    pick: Option<u8>,
) -> CommandResult<ScanAnswerDto> {
    answer_scan_picking(&state, scan, question, units, pick).await
}

/// [`answer_scan_picking`] for a question that is not a choice of one asked back.
#[cfg(test)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub async fn answer_scan(
    state: &AppState,
    scan: ScanId,
    question: String,
    units: SizeUnitsDto,
) -> CommandResult<ScanAnswerDto> {
    answer_scan_picking(state, scan, question, units, None).await
}

/// The refusals come in this order: a blank or too long question (`invalid`); an unknown or running scan
/// (`notFound`); a broken rules file (`invalid`); labelling, another model job or another question, the
/// model switched off, not on disk or unavailable (`invalid`). Once the question is taken, reading it can end
/// in an error too (`invalid`): the model was stopped before it read the question (switched off), or it could
/// not be loaded (not enough memory, a file that does not check out). A model that only runs out of time or
/// writes something unreadable gives the general question, and the words of the question still decide what
/// they can (see [`read`]): a question whose intent and subject the words settle is answered without the
/// model. A `pick` is the id of a choice of the same question asked back before; it is read again from the same
/// scan and question, and one that fits no choice any more is `invalid`.
pub async fn answer_scan_picking(
    state: &AppState,
    scan: ScanId,
    question: String,
    units: SizeUnitsDto,
    pick: Option<u8>,
) -> CommandResult<ScanAnswerDto> {
    let question = clean_question(&question).map_err(|error| CommandError::invalid(refusal_text(error)))?;
    let tree = state.scans.tree(scan)?;
    // With rules that cannot be read nothing says which folders the user keeps, so nothing is described to
    // the model or listed.
    let user = user_rules::current(&state.user_rules).await?;
    user.usable()?;
    // Held until the answer is made: no other question takes the model, and no labelling begins, meanwhile.
    let ask = state.advisor.begin_ask()?;

    let labels = state.scans.labels(scan)?;
    let enabled = state.advisor.enabled();
    let advisor = Arc::clone(&state.advisor);
    let scans = Arc::clone(&state.scans);
    let platform = Arc::clone(&state.platform);
    // The log is written when the user's switch is on as the answer is made.
    let log = Arc::clone(&state.question_log);
    let asked_back = Arc::clone(&state.asked_back);

    // The model takes seconds and the rules walk the tree: all of it is off the async runtime.
    tauri::async_runtime::spawn_blocking(move || {
        let _ask = ask;
        let started = Instant::now();
        let review = Review::new(&tree, &platform, user.rules.clone());

        // The words first; the model only when they leave something open.
        let os = review.env().os;
        let mut reading = read(&question, None, review.rules(), os);
        let mut model = None;
        if !reading.complete {
            let by_model = advisor.interpret_traced(&question)?;

            reading = read(&question, Some(by_model.query()), review.rules(), os);
            model = Some(by_model);
        }
        let trace = Trace {
            scan,
            question: &question,
            pick,
            model,
            stage_a: started.elapsed(),
        };
        let query = reading.query.clone();

        let totals = match query.intent {
            Intent::General => Some(
                suggestions::suggestions(&scans, &platform, scan, enabled, &user, Units::Decimal)?.totals,
            ),
            _ => None,
        };

        // Read once the question is read, so a cleanup that ended meanwhile is not missed.
        let outdated = scans.is_outdated(scan);
        let dirs = review
            .env()
            .home
            .as_deref()
            .map(user_dirs_besides_the_environment)
            .unwrap_or_default();
        let asked = Asked {
            query: &query,
            units: units.into(),
            outdated,
            subject: reading.subject.clone(),
            media: reading.media,
            polish: reading.polish,
            everything: reading.everything,
            pick,
            // A pick must be a choice this backend showed for this question.
            shown: pick.map(|_| asked_back.shown(scan, &question)),
            unread: matches!(trace.model, Some(ModelRead::TimedOut | ModelRead::Unreadable)),
            dirs,
        };
        let labels = labels.as_deref().map_or(&[][..], |set| &set.labels[..]);
        let collected = collect(&review, labels, totals, &asked)?;

        // A question asked back is the prompt and nothing else.
        let written = match &collected.clarify {
            Some(clarify) => vec![Segment::Text(clarify.prompt.clone())],
            None => template_segments(&collected.sheet),
        };
        let answer = render(&written);
        let segments = segment_dtos(written, &collected);
        if let Some(clarify) = &collected.clarify {
            asked_back.remember(scan, &question, clarify.choices.iter().map(Choice::key).collect());
        }

        log.append_if(
            &record::entry(&trace, &reading, &collected, &answer, started.elapsed()),
            || advisor.log_questions(),
        );

        Ok(ScanAnswerDto {
            scan,
            understood: collected.understood,
            answer,
            segments,
            rows: collected.rows,
            more: collected.more,
            notes: collected.notes,
            clarify: collected.clarify.map(|clarify| ClarifyDto {
                prompt: clarify.prompt,
                choices: clarify
                    .choices
                    .into_iter()
                    .enumerate()
                    .map(|(id, choice)| ChoiceDto {
                        id: id as u8,
                        label: choice.label,
                        path: choice.path,
                        node: choice.node,
                    })
                    .collect(),
            }),
        })
    })
    .await?
}

/// The answer's pieces as the web view gets them: a path is bound to the folder it names, with that
/// folder's own verdict. A path that cannot be bound (it never happens for a sheet made by [`collect`])
/// stays in the text, in quotes.
fn segment_dtos(segments: Vec<Segment>, collected: &Collected) -> Vec<AnswerSegmentDto> {
    segments
        .into_iter()
        .map(|segment| {
            let Segment::Path { path, place } = &segment else {
                return AnswerSegmentDto::Text {
                    text: render(&[segment]),
                };
            };

            let folder = match place {
                Place::Row(index) => collected
                    .rows
                    .get(*index)
                    .map(|row| (row.node, row.verdict, row.source)),
                Place::Scope => collected
                    .scope_row
                    .as_ref()
                    .map(|scope| (scope.node, scope.verdict, scope.source)),
            };

            match folder {
                Some((node, verdict, source)) => AnswerSegmentDto::Path {
                    node,
                    path: path.clone(),
                    verdict,
                    source,
                },
                None => AnswerSegmentDto::Text {
                    text: render(&[segment]),
                },
            }
        })
        .collect()
}

/// The refusal for a question that cannot be asked. Says nothing of its text.
fn refusal_text(error: platter_ai::QuestionError) -> &'static str {
    match error {
        platter_ai::QuestionError::Empty => "the question is empty",
        platter_ai::QuestionError::TooLong => "the question is longer than 500 characters",
    }
}

// ---- Collecting rows and facts ----

/// What is asked: the reading of the question (see [`read`]) and what the answer needs of it.
struct Asked<'a> {
    query: &'a ScanQuestion,
    units: Units,
    outdated: bool,
    /// What the question names.
    subject: Subject,
    /// The media category of a `media` kind question.
    media: Option<MediaKind>,
    /// The question is Polish: the answer says it is in English.
    polish: bool,
    /// The question wants everything cleaned: the answer says Platter does not do that from here.
    everything: bool,
    /// The id of a choice of the same question asked back before.
    pick: Option<u8>,
    /// The choices the user was shown when this question was asked back, as far as the backend knows
    /// (empty: none). `None` in tests that pick without having asked.
    shown: Option<Vec<ChoiceKey>>,
    /// The model was asked to read the question and could not (it ran out of time or wrote no query).
    unread: bool,
    /// Downloads, Templates and Public, where this machine has them (the environment lists the others).
    dirs: Vec<(WellKnown, PathBuf)>,
}

impl<'a> Asked<'a> {
    /// A query whose scope text is the only subject: a path when it looks like one, else a name.
    #[cfg(test)]
    fn of(query: &'a ScanQuestion, units: Units, outdated: bool) -> Self {
        let scope = query.scope.trim();
        let subject = if scope.is_empty() {
            Subject::None
        } else if scope == "~" || scope.starts_with("~/") || scope.starts_with('/') {
            Subject::Path(scope.to_owned())
        } else {
            Subject::Name(scope.to_owned())
        };

        Self {
            query,
            units,
            outdated,
            subject,
            media: None,
            polish: false,
            everything: false,
            pick: None,
            shown: None,
            unread: false,
            dirs: Vec::new(),
        }
    }
}

/// The answer's data, computed by the backend: the rows the user sees and the sheet the answer is written
/// from come from the same list.
struct Collected {
    understood: UnderstoodDto,
    /// The folder the question was about (not for `find`), with the verdict [`Lens::row`] gives it.
    scope_row: Option<ScopeRow>,
    rows: Vec<ScanRowDto>,
    more: u64,
    notes: Vec<String>,
    /// Sentences about how the question was read (the rule that covers the folder, how many folders
    /// matched): not part of the answer, only of the log.
    meta: Vec<String>,
    sheet: FactSheet,
    /// The question asked back, when the words fit more than one thing.
    clarify: Option<Clarify>,
}

/// The scope as a folder of the answer.
struct ScopeRow {
    node: NodeId,
    verdict: Option<Verdict>,
    source: Option<RowSourceDto>,
}

/// Where the question's scope points in the finished tree.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scope {
    /// No scope: the whole scan.
    Whole,
    Folder {
        node: NodeId,
        path: String,
    },
    /// One file, named by its path or by its exact name. Answered by one row, whatever the intent.
    File {
        node: NodeId,
        path: String,
    },
    /// The folders that the rules named by the scope text matched in this scan (more than one), largest
    /// first, and what to call them.
    Rules {
        title: String,
        nodes: Vec<NodeId>,
    },
    /// A scope text that is no file or folder of this scan. `absent` says what the rules call it and where they look
    /// for it, when a rule knows it.
    Missing {
        text: String,
        absent: Option<Absent>,
    },
}

/// What a rule with a known place is called by the question, and where that place is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Absent {
    /// The words of the question that named the rule (`docker` of `docker images`).
    named: String,
    hint: String,
}

/// A folder or file of the answer before it is shown.
struct Row {
    node: NodeId,
    verdict: Option<Verdict>,
    source: Option<RowSourceDto>,
    note: Option<String>,
    /// The words of whoever decided the folder: the rule's reason or the guard's.
    why: Option<String>,
    /// Platter could not read the folder, so its size is unknown.
    unreadable: bool,
}

/// What one intent found, before it is turned into rows and a sheet.
#[derive(Default)]
struct Found {
    rows: Vec<Row>,
    /// Matches that are not among the rows.
    more: u64,
    scope_size: Option<u64>,
    files: Option<u64>,
    totals: Vec<FactTotal>,
    notes: Vec<String>,
    /// Sentences for the log only (see [`Collected::meta`]).
    meta: Vec<String>,
    context_paths: Vec<String>,
    /// The rows are the thing the question named, not what lies inside it.
    subject: bool,
}

/// The finished tree with what the rules, the guard and the model's earlier labels say about it.
struct Lens<'a> {
    review: &'a Review<'a>,
    tree: &'a Tree,
    labels: &'a [ModelLabel],
    /// Every node each rule matched, worked out when first needed.
    matched: OnceCell<Vec<RuleMatches>>,
    /// Downloads, Templates and Public, where this machine has them (the environment lists the others).
    dirs: &'a [(WellKnown, PathBuf)],
    /// The media category a `media` question is about.
    media: Option<MediaKind>,
    /// How sizes are written in the notes.
    units: Units,
}

fn collect(
    review: &Review<'_>,
    labels: &[ModelLabel],
    totals: Option<VerdictTotals>,
    asked: &Asked<'_>,
) -> CommandResult<Collected> {
    let lens = Lens::new(review, labels)
        .with_dirs(&asked.dirs)
        .with_media(asked.media)
        .with_units(asked.units);
    let query = asked.query;
    // A find of a family of folders is a question about the biggest of them.
    let intent = if query.intent == Intent::Find && query.kind != Kind::Any {
        Intent::Largest
    } else {
        query.intent
    };

    // `find` looks for a name, so a name is not a folder to resolve; an overview is of the whole scan.
    let resolved = match (intent, &asked.subject) {
        // A choice was asked for this question before, but this reading asks nothing back.
        (Intent::General | Intent::Offtopic, _) | (Intent::Find, Subject::Name(_) | Subject::None)
            if asked.pick.is_some() =>
        {
            return Err(CommandError::invalid(resolve::STALE_CHOICE));
        }
        (Intent::General | Intent::Offtopic, _) => Resolved::whole(),
        (Intent::Find, Subject::Name(_) | Subject::None) => Resolved::whole(),
        _ => lens.resolve(&asked.subject, asked.pick, asked.shown.as_deref())?,
    };
    if let Some(clarify) = resolved.clarify {
        return Ok(clarified(asked, intent, clarify));
    }

    let mut scope = resolved.scope;
    let mut notes = Vec::new();
    let mut meta = Vec::new();
    if asked.outdated {
        notes.push(OUTDATED_NOTE.to_owned());
    }
    if asked.unread && asked.subject == Subject::None && intent != Intent::Offtopic {
        notes.push(UNREAD_NOTE.to_owned());
    }
    let place_notes = resolved.notes;
    let mut context_paths = Vec::new();

    // A reason is explained for one folder: of several that a rule matched, the largest.
    if let Scope::Rules { title, nodes } = &scope
        && intent == Intent::Explain
    {
        meta.push(format!(
            "{} folders match {}. The largest is explained.",
            nodes.len(),
            quoted(title)
        ));
        scope = lens.folder(nodes[0]);
    }

    let mut found = match (&scope, intent) {
        (_, Intent::Offtopic) => Found::default(),
        (Scope::Missing { .. }, _) => Found::default(),
        (Scope::File { node, .. }, _) => lens.file(*node),
        (_, Intent::Find) => match &asked.subject {
            Subject::Name(name) => lens.find(name, query.limit),
            Subject::None => Found::default(),
            _ => lens.find_scope(&scope, query.limit),
        },
        (scope, Intent::Largest) => lens.largest(scope, query.kind, query.limit)?,
        (scope, Intent::Cleanable) => lens.cleanable(scope, query.kind, query.limit),
        (scope, Intent::Labelled) => lens.labelled(scope, query.limit),
        (scope, Intent::Explain) => lens.explain(scope, query.limit)?,
        (_, Intent::General) => lens.general(totals, query.limit)?,
    };

    if let Scope::Missing { text, absent } = &scope {
        let shown = absent
            .as_ref()
            .map_or(text.as_str(), |absent| absent.named.as_str());

        notes.push(not_in_scan(
            shown,
            absent.as_ref().map(|absent| absent.hint.as_str()),
        ));
        // The user's own words may be a path; the answer may repeat them.
        context_paths.push(shown.to_owned());
    }
    notes.append(&mut found.notes);
    meta.append(&mut found.meta);
    notes.extend(place_notes);
    if intent == Intent::Find && matches!(asked.subject, Subject::None) {
        notes.push("No folder name was given to look for.".to_owned());
    } else if intent == Intent::Find && found.rows.is_empty() && matches!(asked.subject, Subject::Name(_)) {
        let absent = lens.absent_for(&query.scope);
        let shown = absent
            .as_ref()
            .map_or(query.scope.as_str(), |absent| absent.named.as_str());

        notes.push(not_in_scan(
            shown,
            absent.as_ref().map(|absent| absent.hint.as_str()),
        ));
        context_paths.push(shown.to_owned());
    }
    if asked.everything && intent == Intent::Cleanable {
        notes.push(NOT_FROM_HERE_NOTE.to_owned());
    }
    if asked.polish && intent != Intent::Offtopic {
        notes.push(ENGLISH_NOTE.to_owned());
    }
    notes.truncate(MAX_NOTES);
    let notes: Vec<String> = notes.iter().map(|note| cut(note, NOTE_CHARS)).collect();

    let (understood_scope, scope_text) = match (&scope, intent) {
        (_, Intent::Find) => (None, Some(query.scope.clone()).filter(|text| !text.is_empty())),
        (Scope::Folder { node, path }, _) => (
            Some(ScopeDto {
                node: *node,
                path: path.clone(),
                file: false,
            }),
            None,
        ),
        (Scope::File { node, path }, _) => (
            Some(ScopeDto {
                node: *node,
                path: path.clone(),
                file: true,
            }),
            None,
        ),
        (Scope::Missing { text, .. }, _) => (None, Some(text.clone())),
        (Scope::Whole | Scope::Rules { .. }, _) => (None, None),
    };
    let understood = UnderstoodDto {
        intent: intent.into(),
        kind: query.kind.into(),
        limit: query.limit,
        scope: understood_scope,
        scope_text,
    };

    let rows: Vec<ScanRowDto> = found
        .rows
        .iter()
        .map(|row| ScanRowDto {
            node: row.node,
            path: review.display(row.node),
            allocated: clamp(lens.tree.allocated(row.node)),
            verdict: row.verdict,
            source: row.source,
            note: row.note.clone(),
            file: lens.tree.kind(row.node) == NodeKind::File,
        })
        .collect();

    let mut sheet = FactSheet::new(intent, asked.units);
    sheet.kind = query.kind;
    match &scope {
        Scope::Folder { path, .. } if intent != Intent::Find => sheet.scope = path.clone(),
        Scope::Rules { title, .. } => sheet.set = Some(title.clone()),
        _ => {}
    }
    if intent == Intent::Find {
        sheet.name = query.scope.clone();
    }
    if query.kind == Kind::Media {
        sheet.category = asked.media.map(|media| media.as_str().to_owned());
    }
    if let Scope::File { node, .. } = &scope {
        sheet.file = Some(lens.file_fact(*node));
    }
    sheet.scope_size = found.scope_size;
    sheet.files = found.files;
    sheet.subject = found.subject;
    sheet.rows = rows
        .iter()
        .zip(&found.rows)
        .map(|(shown, row)| FactRow {
            path: shown.path.clone(),
            size: shown.allocated,
            verdict: row.verdict,
            by: row.source.map(By::from),
            note: row.note.clone(),
            why: row.why.clone(),
            unreadable: row.unreadable,
        })
        .collect();
    sheet.totals = found.totals;
    // The answer says an outdated scan by itself.
    sheet.notes = notes
        .iter()
        .filter(|note| *note != OUTDATED_NOTE)
        .cloned()
        .collect();
    sheet.outdated = asked.outdated;
    sheet.more = found.more;
    sheet.context_paths = [context_paths, found.context_paths].concat();

    // The scope is a folder of the answer like any row, so its verdict is the one `row` gives.
    let scope_row = match &scope {
        Scope::Folder { node, .. } if intent != Intent::Find => {
            let scoped = lens.row(*node);

            Some(ScopeRow {
                node: *node,
                verdict: scoped.verdict,
                source: scoped.source,
            })
        }
        _ => None,
    };

    Ok(Collected {
        understood,
        scope_row,
        rows,
        more: found.more,
        notes,
        meta,
        sheet,
        clarify: None,
    })
}

/// The answer to a question that could mean more than one thing: the question asked back, and nothing that
/// would answer it.
fn clarified(asked: &Asked<'_>, intent: Intent, clarify: Clarify) -> Collected {
    let query = asked.query;

    Collected {
        understood: UnderstoodDto {
            intent: intent.into(),
            kind: query.kind.into(),
            limit: query.limit,
            scope: None,
            scope_text: Some(query.scope.clone()).filter(|text| !text.is_empty()),
        },
        scope_row: None,
        rows: Vec::new(),
        more: 0,
        notes: Vec::new(),
        meta: Vec::new(),
        sheet: FactSheet::new(intent, asked.units),
        clarify: Some(clarify),
    }
}

/// A text of the user's disk or of the user's words, set apart in “ ” quotes. A quote inside it is shown as
/// `‟` or `〃`, so the text cannot close the quotes it is in.
fn quoted(text: &str) -> String {
    quote_name(text)
}

/// `No file or folder called “x” is in this scan`, and where the rules look for what `x` names, when they know.
fn not_in_scan(text: &str, hint: Option<&str>) -> String {
    match hint {
        Some(hint) => format!(
            "No file or folder called {} is in this scan ({hint})",
            quoted(text)
        ),
        None => format!("No file or folder called {} is in this scan", quoted(text)),
    }
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

impl<'a> Lens<'a> {
    fn new(review: &'a Review<'a>, labels: &'a [ModelLabel]) -> Self {
        Self {
            review,
            tree: review.tree,
            labels,
            matched: OnceCell::new(),
            dirs: &[],
            media: None,
            units: Units::Decimal,
        }
    }

    fn with_dirs(mut self, dirs: &'a [(WellKnown, PathBuf)]) -> Self {
        self.dirs = dirs;
        self
    }

    fn with_media(mut self, media: Option<MediaKind>) -> Self {
        self.media = media;
        self
    }

    fn with_units(mut self, units: Units) -> Self {
        self.units = units;
        self
    }

    /// Every node each rule matched (the same pass as the explainer's, made once).
    fn matched(&self) -> &[RuleMatches] {
        self.matched
            .get_or_init(|| matches(self.tree, self.review.env(), self.review.rules()))
    }

    // ---- Scope ----

    /// `node` as the scope: the folder, with its display path.
    fn folder(&self, node: NodeId) -> Scope {
        Scope::Folder {
            node,
            path: self.review.display(node),
        }
    }

    /// `node` as the scope when it is a file: the file, with its display path.
    fn file_scope(&self, node: NodeId) -> Scope {
        Scope::File {
            node,
            path: self.review.display(node),
        }
    }

    /// One folder is that folder; more are the folders of a rule.
    fn scope_of(&self, title: String, nodes: Vec<NodeId>) -> Scope {
        match nodes.as_slice() {
            [node] => self.folder(*node),
            _ => Scope::Rules { title, nodes },
        }
    }

    /// The folders that the rules the text names matched in this scan, largest first; or where such a rule
    /// looks, when it has a known place and nothing was matched. `None` when no rule has the words, or
    /// one has them and has neither a match nor a place to name.
    ///
    /// The words of the text (see [`scope_words`]) are scored against each rule's id, its title and a few
    /// names it goes by. The rules with the most words win, and of those the ones that fit closest (the
    /// share of their own words the text has), so `desktop` is the Desktop folder and not Docker Desktop.
    /// Rules of equal standing all count (`docker` is Docker data and the Docker Desktop disk image).
    fn by_rule(&self, text: &str) -> Option<ByRule> {
        let words = scope_words(text);
        if words.is_empty() {
            return None;
        }

        let named = rules_for_words(self.review.rules(), self.review.env().os, &words);

        self.by_named_rules(&named, text)
    }

    /// The folders of the rules `named` matched in this scan, largest first, with what to call them; where
    /// they look, when they have one place and nothing was matched. `text` is what the user said.
    fn by_named_rules(&self, named: &[&Rule], text: &str) -> Option<ByRule> {
        if named.is_empty() {
            return None;
        }

        let mut nodes: Vec<NodeId> = Vec::new();
        let mut matching: Vec<&Rule> = Vec::new();
        // The folders each rule matched, to tell the tools it names apart.
        let mut per_rule: Vec<(&Rule, Vec<NodeId>)> = Vec::new();
        for found in self.matched() {
            let Some(rule) = named.iter().find(|rule| rule.id == found.id) else {
                continue;
            };
            let folders: Vec<NodeId> = found
                .nodes
                .iter()
                .copied()
                .filter(|&node| self.is_dir(node))
                .collect();

            if !folders.is_empty() {
                matching.push(rule);
                per_rule.push((rule, folders.clone()));
                nodes.extend(folders);
            }
        }

        // A rule's place that Platter could not read is not matched (nothing is known of it), but it is
        // there: it is the folder asked about, and the answer says it could not be read.
        for rule in named {
            if !matching.iter().any(|matched| matched.id == rule.id)
                && let Some(node) = self.unreadable_place(rule)
            {
                matching.push(rule);
                per_rule.push((rule, vec![node]));
                nodes.push(node);
            }
        }
        self.by_size(&mut nodes);
        nodes.dedup();

        if nodes.is_empty() {
            let places: Vec<String> = named.iter().filter_map(|rule| place_hint(rule)).take(2).collect();

            return (!places.is_empty()).then(|| {
                ByRule::Absent(Absent {
                    named: named_words(text, named),
                    hint: places.join(". "),
                })
            });
        }

        // One rule's folders are called by its title; the folders of several by what the user said.
        let title = match matching.as_slice() {
            [rule] => rule.title.to_owned(),
            _ if text.chars().all(char::is_alphabetic) => capitalised(text),
            _ => text.to_owned(),
        };

        Some(ByRule::Found {
            title,
            nodes,
            located: named.iter().any(|rule| place_hint(rule).is_some()),
            notes: self.outside_notes(named),
            tools: self.tools_of(per_rule),
        })
    }

    /// For each of the rules `named` whose one fixed place lies outside the scanned folder: where it is and
    /// how to include it (`“/var/lib/docker” (Docker data) is outside this scan. Scan / to include it.`).
    /// Asked only when folders of the tool were found, so the answer does not look complete without it.
    fn outside_notes(&self, named: &[&Rule]) -> Vec<String> {
        let root = self.tree.root_path();

        named
            .iter()
            .filter_map(|rule| match rule.matcher {
                Matcher::Fixed(path) if !Path::new(path).starts_with(root) => Some(format!(
                    "{} ({}) is outside this scan. Scan / to include it.",
                    quoted(path),
                    rule.title
                )),
                _ => None,
            })
            .collect()
    }

    /// The rules that matched, grouped by tool: the first word of a rule's id (`docker-data` and
    /// `docker-desktop-vm` are Docker), each with its folders, largest first.
    fn tools_of(&self, per_rule: Vec<(&Rule, Vec<NodeId>)>) -> Vec<Tool> {
        let mut tools: Vec<(&str, Tool)> = Vec::new();

        for (rule, nodes) in per_rule {
            let family = rule.id.split('-').next().unwrap_or(rule.id);

            match tools.iter_mut().find(|(known, _)| *known == family) {
                Some((_, tool)) => tool.nodes.extend(nodes),
                None => tools.push((
                    family,
                    Tool {
                        title: rule.title.to_owned(),
                        nodes,
                    },
                )),
            }
        }

        tools
            .into_iter()
            .map(|(_, mut tool)| {
                self.by_size(&mut tool.nodes);
                tool.nodes.dedup();
                tool
            })
            .collect()
    }

    /// The folder at the one place `rule` looks in, when the scan holds it and could not read it.
    fn unreadable_place(&self, rule: &Rule) -> Option<NodeId> {
        let place = match rule.matcher {
            Matcher::Fixed(path) => path.to_owned(),
            Matcher::Home(path) => format!("~/{path}"),
            _ => return None,
        };
        let found = find_folders(self.tree, self.review.home(), &place, 1)
            .ok()?
            .into_iter()
            .next()?;

        (found.path == place && self.tree.is_unreadable(found.node)).then_some(found.node)
    }

    /// What the rules call `text` and where they look for it, when this scan does not hold it.
    fn absent_for(&self, text: &str) -> Option<Absent> {
        match self.by_rule(text) {
            Some(ByRule::Absent(absent)) => Some(absent),
            _ => None,
        }
    }

    /// The scope as a node: a resolved folder, or the root for the whole scan (and, where a scope of
    /// several folders is not handled, the largest of them).
    fn node_of(&self, scope: &Scope) -> NodeId {
        match scope {
            Scope::Folder { node, .. } | Scope::File { node, .. } => *node,
            Scope::Rules { nodes, .. } => nodes[0],
            _ => self.tree.root(),
        }
    }

    /// The folders the scope covers: the root for the whole scan, none for a scope that is not there.
    fn roots(&self, scope: &Scope) -> Vec<NodeId> {
        match scope {
            Scope::Whole => vec![self.tree.root()],
            Scope::Folder { node, .. } | Scope::File { node, .. } => vec![*node],
            Scope::Rules { nodes, .. } => nodes.clone(),
            Scope::Missing { .. } => Vec::new(),
        }
    }

    fn is_given(scope: &Scope) -> bool {
        matches!(scope, Scope::Folder { .. })
    }

    /// Whether `node` is `scope` or lies below it.
    fn within(&self, node: NodeId, scope: NodeId) -> bool {
        let mut current = Some(node);

        while let Some(at) = current {
            if at == scope {
                return true;
            }
            current = self.tree.parent(at);
        }
        false
    }

    fn is_dir(&self, node: NodeId) -> bool {
        self.tree.contains(node) && self.tree.kind(node) == NodeKind::Dir
    }

    // ---- Verdicts ----

    /// A folder as a row, with its standing in the order `ask_advisor` uses: the guard, then a rule that
    /// covers it (the built-in table or the user's own), then a label the model made earlier. Nothing else
    /// ever gives a row a verdict, and only a rule gives `safe`.
    fn row(&self, node: NodeId) -> Row {
        let unreadable = self.tree.is_unreadable(node);

        if let Some(detail) = self.review.guard(node) {
            return Row {
                node,
                verdict: Some(Verdict::Keep),
                source: Some(RowSourceDto::Guard),
                note: Some(GUARD_DETAIL.to_owned()),
                why: Some(detail),
                unreadable,
            };
        }

        if let Some(covering) = self.review.explainer.covering(node) {
            let source = match covering.source {
                platter_core::dto::SuggestionSource::User => RowSourceDto::User,
                _ => RowSourceDto::Rule,
            };

            return Row {
                node,
                verdict: Some(covering.verdict),
                source: Some(source),
                note: Some(covering.title.to_owned()),
                why: Some(covering.reason),
                unreadable,
            };
        }

        if let Some(label) = self.labels.iter().find(|label| label.node == node) {
            return Row {
                node,
                verdict: Some(verdict_of(label.verdict)),
                source: Some(RowSourceDto::Model),
                note: Some(LABELLED_BY_MODEL.to_owned()),
                why: None,
                unreadable,
            };
        }

        Row {
            node,
            verdict: None,
            source: None,
            note: None,
            why: None,
            unreadable,
        }
    }

    /// The one row of a file the question named: its standing, whatever the intent.
    fn file(&self, node: NodeId) -> Found {
        let mut found = Found::default();
        self.rows_of(&[node], 1, &mut found);

        found
    }

    /// What the answer says of a file besides its standing: how long ago it was changed, by the scan's own
    /// clock. A file with no modification time (0 or less) has no age.
    fn file_fact(&self, node: NodeId) -> FileFact {
        let changed = self.tree.mtime(node);
        let age_days = (changed > 0)
            .then(|| self.tree.scanned_at().saturating_sub(changed).max(0) as u64 / SECONDS_PER_DAY);

        FileFact { age_days }
    }

    /// The title of a rule of this scan's rule set.
    fn title_of(&self, id: &str) -> String {
        self.review
            .rules()
            .rules()
            .iter()
            .find(|rule| rule.id == id)
            .map_or_else(|| id.to_owned(), |rule| rule.title.to_owned())
    }

    fn size(&self, node: NodeId) -> u64 {
        self.tree.allocated(node)
    }

    /// The rows of `nodes`, in order, at most `limit`; the rest is `more`.
    fn rows_of(&self, nodes: &[NodeId], limit: u8, found: &mut Found) {
        let shown = usize::from(limit).min(MAX_ROWS);

        found.rows = nodes.iter().take(shown).map(|&node| self.row(node)).collect();
        found.more = nodes.len().saturating_sub(shown) as u64;
    }

    /// The summary's largest folders, each lifted to the folder a rule matched above it, when that lies
    /// inside the scope: the folder the rules know (`/var/lib/docker`, `~/Projekty/app/target`), not a part
    /// of it (a layer of it). Largest first.
    fn lifted(&self, nodes: Vec<NodeId>, scope: NodeId) -> Vec<NodeId> {
        let mut lifted: Vec<NodeId> = nodes
            .into_iter()
            .map(|node| match self.review.explainer.covering(node) {
                Some(covering)
                    if covering.at != node
                        && covering.at != scope
                        && self.within(covering.at, scope)
                        && self.is_dir(covering.at) =>
                {
                    covering.at
                }
                _ => node,
            })
            .collect();
        self.by_size(&mut lifted);
        lifted.dedup();

        lifted
    }

    /// `nodes`, largest first, equal sizes by id.
    fn by_size(&self, nodes: &mut [NodeId]) {
        nodes.sort_unstable_by_key(|&node| (Reverse(self.size(node)), node));
    }

    // ---- The intents ----

    fn largest(&self, scope: &Scope, kind: Kind, limit: u8) -> CommandResult<Found> {
        // The folders of a rule are the answer themselves, largest first, with what they take together.
        if let Scope::Rules { title, nodes } = scope {
            let mut found = Found::default();
            let bytes: u64 = nodes.iter().map(|&node| self.size(node)).sum();
            found.totals.push(total(title, bytes, nodes.len() as u64));
            self.rows_of(nodes, limit, &mut found);

            return Ok(found);
        }

        match kind {
            Kind::Any => self.largest_any(scope, limit),
            Kind::Cache => Ok(self.largest_family(scope, &CACHE_FAMILY, "All caches", limit)),
            Kind::Build => Ok(self.largest_family(
                scope,
                &BUILD_FAMILY,
                "All build output and dependency folders",
                limit,
            )),
            Kind::Media => Ok(self.largest_media(scope, limit)),
        }
    }

    /// The scope's largest folders, from `summary(...).largest_dirs`.
    fn largest_any(&self, scope: &Scope, limit: u8) -> CommandResult<Found> {
        let node = self.node_of(scope);
        let summary = summary(self.tree, node)?;
        let nodes = self.lifted(summary.largest_dirs.iter().map(|dir| dir.id).collect(), node);

        let mut found = Found {
            scope_size: Some(self.size(node)),
            files: Some(self.tree.files(node)),
            ..Found::default()
        };
        self.rows_of(&nodes, limit, &mut found);

        Ok(found)
    }

    /// The folders of one rule family under the scope, largest first, with the family's total.
    fn largest_family(&self, scope: &Scope, family: &[&str], total_label: &str, limit: u8) -> Found {
        let node = self.node_of(scope);
        let mut found = Found::default();

        // The scope itself lies in the family: it is the one row.
        if let Some(covering) = self.review.explainer.covering(node)
            && family.contains(&covering.id)
        {
            found.rows.push(self.row(node));
            found.totals.push(total(total_label, self.size(node), 1));
            return found;
        }

        let mut nodes: Vec<NodeId> = self
            .matched()
            .iter()
            .filter(|rule| family.contains(&rule.id))
            .flat_map(|rule| rule.nodes.iter().copied())
            .filter(|&matched| self.is_dir(matched) && self.within(matched, node))
            .collect();
        self.by_size(&mut nodes);

        if !nodes.is_empty() {
            let bytes = nodes.iter().map(|&matched| self.size(matched)).sum();
            found.totals.push(total(total_label, bytes, nodes.len() as u64));
        }
        if nodes.is_empty() && scan_is_kept(self.tree, self.review.env(), self.review.rules()) {
            found.notes.push(KEPT_SCAN_NOTE.to_owned());
        }
        self.rows_of(&nodes, limit, &mut found);

        found
    }

    /// A walk down from the scope's children: the first folder of a media category is a row and is not
    /// entered. The note is its category.
    fn largest_media(&self, scope: &Scope, limit: u8) -> Found {
        let node = self.node_of(scope);
        let wanted = self.media_categories();
        let mut hits: Vec<NodeId> = Vec::new();
        let mut pending: Vec<NodeId> = self
            .tree
            .children(node)
            .filter(|&child| self.is_dir(child))
            .collect();

        while let Some(current) = pending.pop() {
            if wanted.contains(&self.tree.category(current)) {
                if self.size(current) > 0 {
                    hits.push(current);
                }
                continue;
            }
            pending.extend(self.tree.children(current).filter(|&child| self.is_dir(child)));
        }
        self.by_size(&mut hits);

        let mut found = Found::default();
        for &category in &wanted {
            let of_category: Vec<NodeId> = hits
                .iter()
                .copied()
                .filter(|&hit| self.tree.category(hit) == category)
                .collect();
            if !of_category.is_empty() {
                let bytes = of_category.iter().map(|&hit| self.size(hit)).sum();
                found
                    .totals
                    .push(total(category_name(category), bytes, of_category.len() as u64));
            }
        }

        self.rows_of(&hits, limit, &mut found);
        for row in &mut found.rows {
            row.note = Some(category_name(self.tree.category(row.node)).to_owned());
        }

        found
    }

    /// The categories a `media` question is about: the one it names (`videos`), else all three.
    fn media_categories(&self) -> Vec<Category> {
        match self.media {
            Some(MediaKind::Video) => vec![Category::Video],
            Some(MediaKind::Image) => vec![Category::Image],
            Some(MediaKind::Audio) => vec![Category::Audio],
            None => MEDIA.to_vec(),
        }
    }

    /// What can be cleaned: the matches that are not `keep` and the labels the suggestions list, `safe`
    /// first, then `review`, largest first. Information only; nothing is ever deleted here.
    ///
    /// A match counts, as a row and in the totals, with the verdict [`Lens::row`] gives it, so the two
    /// cannot disagree. What the guard leaves alone (the folder that was scanned, a protected place, an
    /// unreadable folder) is neither a row nor in a total.
    fn cleanable(&self, scope: &Scope, kind: Kind, limit: u8) -> Found {
        let node = self.node_of(scope);
        let mut found = Found::default();

        if scan_is_kept(self.tree, self.review.env(), self.review.rules()) {
            found.notes.push(KEPT_SCAN_NOTE.to_owned());
            return found;
        }

        // The folders of a rule are what the question names: each is a row with the verdict it has, kept
        // and left alone by the guard too, so the answer can say yes or no about every one.
        if let Scope::Rules { nodes, .. } = scope {
            found.subject = true;
            self.rows_of(nodes, limit, &mut found);

            return found;
        }

        // A scope (or a whole scan) that a rule covers is that rule's: the one row, and for a scope the
        // answer says yes or no about it, a kept folder and one the guard leaves alone too. The whole scan
        // that is itself covered is only said to be kept or left alone.
        if let Some(covering) = self.review.explainer.covering(node) {
            let row = self.row(node);
            let named = Self::is_given(scope);
            found.subject = named;

            if covering.verdict == Verdict::Keep {
                if named {
                    found.rows.push(row);
                } else {
                    found.notes.push(format!(
                        "{} is kept by {}.",
                        quoted(&self.review.display(node)),
                        covering.title
                    ));
                }
            } else if let Some(detail) = self.review.guard(node) {
                // A folder that could not be read says so in its row (and in the answer).
                if !(named && row.unreadable) {
                    found.notes.push(detail);
                }
                if named {
                    found.rows.push(row);
                }
            } else if self.in_kind(kind, node, Some(covering.id)) {
                if let Some(verdict) = row.verdict {
                    self.add_verdict_total(&mut found, verdict, self.size(node), 1);
                }
                found.rows.push(row);
            }
            return found;
        }

        // The folder that was scanned is never a suggestion, also when a rule would claim it: say so, as the
        // guard does, rather than answer with nothing.
        if node == self.tree.root() && scan_root_is_claimed(self.tree, self.review.env(), self.review.rules())
        {
            if let Some(detail) = self.review.guard(node) {
                found.notes.push(detail);
            }

            return found;
        }

        // A scope that Platter could not read is not covered by any rule, but the answer is about it.
        if Self::is_given(scope) && self.tree.is_unreadable(node) {
            found.subject = true;
            found.rows.push(self.row(node));

            return found;
        }

        let mut candidates: Vec<(NodeId, Verdict)> = Vec::new();
        for rule in self.matched() {
            if rule.verdict == Verdict::Keep {
                continue;
            }
            for &matched in &rule.nodes {
                if self.within(matched, node) && self.in_kind(kind, matched, Some(rule.id)) {
                    self.offer(matched, &mut candidates);
                }
            }
        }
        // The model's labels are no rule: they only belong to the kinds that name no rule.
        if matches!(kind, Kind::Any | Kind::Media) {
            for label in self.review.listed_labels(self.labels) {
                if self.within(label.node, node) && self.in_kind(kind, label.node, None) {
                    self.offer(label.node, &mut candidates);
                }
            }
        }

        // A match found twice (by two rules, or as a rule's match and as a label) counts once. Only folders
        // are rows; matched files (old downloads, disk images) count in the totals, and are counted once.
        candidates
            .sort_unstable_by_key(|&(matched, verdict)| (verdict, Reverse(self.size(matched)), matched));
        candidates.dedup_by_key(|&mut (matched, _)| matched);
        let skipped = candidates
            .iter()
            .filter(|(matched, _)| !self.is_dir(*matched))
            .count();

        let mut by_verdict: Vec<(Verdict, Vec<NodeId>)> = Vec::new();
        for verdict in [Verdict::Safe, Verdict::Review] {
            let folders: Vec<NodeId> = candidates
                .iter()
                .filter(|(matched, candidate)| *candidate == verdict && self.is_dir(*matched))
                .map(|&(matched, _)| matched)
                .collect();
            let all: Vec<NodeId> = candidates
                .iter()
                .filter(|(_, candidate)| *candidate == verdict)
                .map(|&(matched, _)| matched)
                .collect();
            if !all.is_empty() {
                let bytes = all.iter().map(|&matched| self.size(matched)).sum();
                self.add_verdict_total(&mut found, verdict, bytes, all.len() as u64);
            }
            by_verdict.push((verdict, folders));
        }

        // The rows are the largest folders, the limit shared between what is safe and what needs a look, so
        // the answer names both (the biggest to review are often the ones that matter most).
        let limit = usize::from(limit).min(MAX_ROWS);
        let safe = by_verdict[0].1.len();
        let review = by_verdict[1].1.len();
        let (take_safe, take_review) = if safe == 0 || review == 0 {
            (limit.min(safe), limit.min(review))
        } else {
            let review_share = review.min((limit / 2).max(1));
            let safe_share = safe.min(limit - review_share.min(limit.saturating_sub(1)));

            (safe_share, review.min(limit - safe_share))
        };
        let mut listed: Vec<NodeId> = Vec::new();
        listed.extend(by_verdict[0].1.iter().take(take_safe));
        listed.extend(by_verdict[1].1.iter().take(take_review));

        found.rows = listed.iter().map(|&node| self.row(node)).collect();
        found.more = (candidates.len() - listed.len()) as u64;
        // What is not listed is told by a name that stands for it (`__pycache__`).
        for (verdict, folders) in &by_verdict {
            let unlisted: Vec<NodeId> = folders
                .iter()
                .copied()
                .filter(|node| !listed.contains(node))
                .collect();
            let example = self.common_name(&unlisted);
            if let Some(total) = found
                .totals
                .iter_mut()
                .find(|total| total.kind == TotalKind::Verdict(*verdict))
            {
                total.example = example;
            }
        }
        if skipped > 0 {
            found.notes.push(format!(
                "{skipped} matching files are counted in the totals but not listed. Only folders are listed."
            ));
        }

        // Nothing to clean in the folder: where Platter stands on it.
        if found.rows.is_empty() && found.totals.is_empty() && Self::is_given(scope) {
            self.standing_of(node, &mut found);
        }

        found
    }

    /// The name most of `nodes` have (the first of the largest wins a tie), when more than one has it.
    fn common_name(&self, nodes: &[NodeId]) -> Option<String> {
        let mut counts: HashMap<Vec<u8>, usize> = HashMap::new();
        let mut order: Vec<Vec<u8>> = Vec::new();

        for &node in nodes {
            let name = self.tree.name(node).to_vec();
            let count = counts.entry(name.clone()).or_insert(0);
            if *count == 0 {
                order.push(name);
            }
            *count += 1;
        }
        let (name, count) = order
            .into_iter()
            .map(|name| {
                let count = counts[&name];
                (name, count)
            })
            .max_by_key(|(_, count)| *count)?;

        (count > 1).then(|| String::from_utf8_lossy(&name).into_owned())
    }

    /// What a folder that nothing in it can be cleaned is: its size, and who decides about it.
    fn standing_of(&self, node: NodeId, found: &mut Found) {
        found.scope_size = Some(self.size(node));
        found.files = Some(self.tree.files(node));

        let row = self.row(node);
        if row.source == Some(RowSourceDto::Model) {
            found
                .notes
                .push("The local model labelled it for review. Deleting it is your call.".to_owned());
            return;
        }

        // What its rules keep inside it.
        let kept: Vec<(String, u64)> = self
            .review
            .explainer
            .inside(node)
            .into_iter()
            .filter(|(id, _)| {
                self.review
                    .rules()
                    .rules()
                    .iter()
                    .any(|rule| rule.id == *id && rule.verdict == Verdict::Keep)
            })
            .take(2)
            .map(|(id, bytes)| (self.title_of(id), bytes))
            .collect();
        if kept.is_empty() {
            found
                .notes
                .push("Platter has no rule for it. Deleting it is your call.".to_owned());
            return;
        }

        let named: Vec<String> = kept
            .iter()
            .map(|(title, bytes)| format!("{} ({})", quoted(title), format_size(*bytes, self.units)))
            .collect();
        found.notes.push(format!(
            "Platter keeps part of it, such as {}. Deleting the rest is your call.",
            named.join(" and ")
        ));
    }

    /// Adds `node` to the cleanable candidates with the verdict its row has: only what is `safe` or `review`
    /// by a rule, the user or the model, and that the guard does not leave alone.
    fn offer(&self, node: NodeId, candidates: &mut Vec<(NodeId, Verdict)>) {
        let row = self.row(node);

        if row.source == Some(RowSourceDto::Guard) {
            return;
        }
        if let Some(verdict @ (Verdict::Safe | Verdict::Review)) = row.verdict {
            candidates.push((node, verdict));
        }
    }

    /// Whether a match belongs to the question's kind. `rule` is the rule that matched, when one did.
    fn in_kind(&self, kind: Kind, node: NodeId, rule: Option<&str>) -> bool {
        match (kind, rule) {
            (Kind::Any, _) => true,
            (Kind::Cache, Some(id)) => CACHE_FAMILY.contains(&id),
            (Kind::Build, Some(id)) => BUILD_FAMILY.contains(&id),
            (Kind::Cache | Kind::Build, None) => false,
            (Kind::Media, _) => self.media_categories().contains(&self.tree.category(node)),
        }
    }

    fn add_verdict_total(&self, found: &mut Found, verdict: Verdict, bytes: u64, count: u64) {
        match found
            .totals
            .iter_mut()
            .find(|total| total.kind == TotalKind::Verdict(verdict))
        {
            Some(total) => {
                total.bytes = Some(total.bytes.unwrap_or(0).saturating_add(bytes));
                total.count = Some(total.count.unwrap_or(0) + count);
            }
            None => found
                .totals
                .push(FactTotal::of_verdict(verdict, bytes, Some(count))),
        }
    }

    /// The labels the model made earlier, under the scope: its verdict and reason, as it said them.
    fn labelled(&self, scope: &Scope, limit: u8) -> Found {
        let roots = self.roots(scope);
        let mut found = Found::default();

        let mut labelled: Vec<&ModelLabel> = self
            .labels
            .iter()
            .filter(|label| {
                self.is_dir(label.node) && roots.iter().any(|&root| self.within(label.node, root))
            })
            .collect();
        labelled.sort_unstable_by_key(|label| (Reverse(self.size(label.node)), label.node));
        labelled.dedup_by_key(|label| label.node);

        // The answer says that nothing is labelled; the note says what to do about it.
        if self.labels.is_empty() {
            found.notes.push(NO_LABELS_NOTE.to_owned());
        }

        for verdict in [ModelVerdict::Review, ModelVerdict::Keep] {
            let of_verdict: Vec<&&ModelLabel> =
                labelled.iter().filter(|each| each.verdict == verdict).collect();
            if !of_verdict.is_empty() {
                let bytes = of_verdict.iter().map(|each| self.size(each.node)).sum();
                found.totals.push(FactTotal::of_verdict(
                    verdict_of(verdict),
                    bytes,
                    Some(of_verdict.len() as u64),
                ));
            }
        }

        let shown = usize::from(limit).min(MAX_ROWS);
        // The answer lists the folders with their verdicts only. What the model said is for the log.
        found.rows = labelled
            .iter()
            .take(shown)
            .map(|label| Row {
                node: label.node,
                verdict: Some(verdict_of(label.verdict)),
                source: Some(RowSourceDto::Model),
                note: None,
                why: None,
                unreadable: false,
            })
            .collect();
        found.meta.extend(
            labelled
                .iter()
                .take(shown)
                .filter(|label| !label.reason.trim().is_empty())
                .map(|label| {
                    format!(
                        "The local model's reason for {}: {}",
                        quoted(&self.review.display(label.node)),
                        label.reason
                    )
                }),
        );
        found.more = labelled.len().saturating_sub(shown) as u64;

        found
    }

    /// Why a folder is big: what it is made of, how old it is, what the rules matched inside it and where
    /// it stands. The rows are its largest folders.
    fn explain(&self, scope: &Scope, limit: u8) -> CommandResult<Found> {
        let node = self.node_of(scope);
        let summary = summary(self.tree, node)?;
        let size = self.size(node);

        let mut found = Found {
            scope_size: Some(size),
            files: Some(self.tree.files(node)),
            ..Found::default()
        };

        // The largest three parts, without one that rounds to nothing.
        for stat in summary
            .by_category
            .iter()
            .take(3)
            .filter(|stat| percent(stat.allocated, size) > 0)
        {
            found.totals.push(FactTotal {
                kind: TotalKind::Category,
                percent: Some(percent(stat.allocated, size)),
                ..FactTotal::plain(category_name(stat.category), Some(stat.allocated), None)
            });
        }

        let older: u64 = summary
            .by_age
            .iter()
            .filter(|stat| matches!(stat.bucket, AgeBucket::TwoYears | AgeBucket::Older))
            .map(|stat| stat.allocated)
            .sum();
        found.totals.push(FactTotal {
            kind: TotalKind::Older,
            percent: Some(percent(older, size)),
            ..FactTotal::plain("Older than a year", Some(older), None)
        });

        for (id, bytes) in self.review.explainer.inside(node).into_iter().take(3) {
            found.totals.push(FactTotal {
                kind: TotalKind::Rule,
                ..FactTotal::plain(&self.title_of(id), Some(bytes), None)
            });
        }

        // Where the scope stands is for the log: its verdict is the chip of its path in the answer.
        if Self::is_given(scope)
            && let Some(standing) = self.standing(node)
        {
            found.meta.push(standing);
        }

        let nodes = self.lifted(summary.largest_dirs.iter().map(|dir| dir.id).collect(), node);
        self.rows_of(&nodes, limit, &mut found);

        Ok(found)
    }

    /// Where the scope stands, in the order the advisor decides: the guard, a rule, an earlier label.
    fn standing(&self, node: NodeId) -> Option<String> {
        let row = self.row(node);
        let verdict = match row.verdict? {
            Verdict::Safe => "safe",
            Verdict::Review => "review",
            Verdict::Keep => "keep",
        };

        Some(match row.source? {
            RowSourceDto::Guard => "This folder is left alone by the safety guard.".to_owned(),
            RowSourceDto::Rule | RowSourceDto::User => format!(
                "This folder is covered by the rule {} (verdict {verdict}).",
                quoted(&row.note.unwrap_or_default())
            ),
            RowSourceDto::Model => format!("The local model labelled this folder {}.", quoted(verdict)),
        })
    }

    /// Folders and files with a name, each with its standing: the folders of the rule that has a known place
    /// under that name (`docker` is `/var/lib/docker`, not every folder called docker), else from
    /// `find_entries`.
    fn find(&self, name: &str, limit: u8) -> Found {
        let mut found = Found::default();

        if let Some(ByRule::Found {
            nodes, located: true, ..
        }) = self.by_rule(name)
        {
            self.rows_of(&nodes, limit, &mut found);
            return found;
        }

        let matched =
            find_entries(self.tree, self.review.home(), name, MAX_ENTRY_MATCHES).unwrap_or_default();
        let nodes: Vec<NodeId> = matched.iter().map(|each| each.node).collect();

        self.rows_of(&nodes, limit, &mut found);
        found
    }

    /// The folder (or folders of a rule) a `find` is about, each with its standing.
    fn find_scope(&self, scope: &Scope, limit: u8) -> Found {
        let mut found = Found::default();

        match scope {
            Scope::Folder { node, .. } | Scope::File { node, .. } => {
                self.rows_of(&[*node], limit, &mut found)
            }
            Scope::Rules { nodes, .. } => self.rows_of(nodes, limit, &mut found),
            Scope::Whole | Scope::Missing { .. } => {}
        }

        found
    }

    /// An unclear question about the disk: the whole scan's size, its largest folders and what the
    /// suggestions total.
    fn general(&self, totals: Option<VerdictTotals>, limit: u8) -> CommandResult<Found> {
        let root = self.tree.root();
        let summary = summary(self.tree, root)?;
        let nodes = self.lifted(summary.largest_dirs.iter().map(|dir| dir.id).collect(), root);

        let mut found = Found {
            scope_size: Some(self.size(root)),
            files: Some(self.tree.files(root)),
            ..Found::default()
        };
        self.rows_of(&nodes, limit, &mut found);

        // A verdict is told only when something has it: a "safe" total says the sheet has safe folders.
        if let Some(totals) = totals {
            for (verdict, bytes) in [
                (Verdict::Safe, totals.safe),
                (Verdict::Review, totals.review),
                (Verdict::Keep, totals.keep),
            ] {
                if bytes > 0 {
                    found.totals.push(FactTotal::of_verdict(verdict, bytes, None));
                }
            }
        }
        if !self.labels.is_empty() {
            found.totals.push(FactTotal::plain(
                "Folders the model labelled",
                None,
                Some(self.labels.len() as u64),
            ));
        }

        Ok(found)
    }
}

fn total(label: &str, bytes: u64, count: u64) -> FactTotal {
    FactTotal::plain(label, Some(bytes), Some(count))
}

// ---- What the rules know by name ----

/// The folders of one tool of the rules in this scan.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tool {
    title: String,
    nodes: Vec<NodeId>,
}

/// What the rules know of a scope text.
enum ByRule {
    /// The folders the rules matched in this scan, largest first, and what to call them. `located`: a rule
    /// among them has a known place, so the name means that place.
    Found {
        title: String,
        nodes: Vec<NodeId>,
        located: bool,
        /// The places the named rules look in, outside this scan (see [`Lens::outside_notes`]).
        notes: Vec<String>,
        /// The folders of each tool that matched (rules of one family, such as `docker-data` and
        /// `docker-desktop-vm`, are one tool), with the first rule's title.
        tools: Vec<Tool>,
    },
    /// A rule with a known place names it, and that place is not in this scan.
    Absent(Absent),
}

/// Words that name no folder.
const STOP_WORDS: [&str; 27] = [
    "a", "all", "an", "and", "any", "are", "can", "do", "does", "for", "from", "in", "is", "it", "me", "my",
    "of", "on", "or", "that", "the", "these", "this", "those", "to", "you", "your",
];

/// Words so common in the titles of the rules that on their own they name no rule (`cache` is in most of
/// them).
const GENERIC_WORDS: [&str; 19] = [
    "app",
    "apps",
    "build",
    "cache",
    "data",
    "dir",
    "directory",
    "file",
    "files",
    "folder",
    "folders",
    "old",
    "output",
    "package",
    "recent",
    "space",
    "system",
    "temp",
    "tmp",
];

/// Other names a rule goes by, besides the words of its id and title.
const ALIASES: [(&str, &[&str]); 9] = [
    ("docker-data", &["container", "image", "volume"]),
    ("docker-desktop-vm", &["container", "image", "volume"]),
    ("python-cache", &["pip", "pycache", "python"]),
    ("trash", &["bin", "recycle", "wastebasket"]),
    ("journal", &["log"]),
    ("coredumps", &["crash", "dump"]),
    ("btrfs-snapshots", &["snapper", "rollback"]),
    ("steam-library", &["game"]),
    ("pacman-cache", &["package", "pkg"]),
];

/// The lower-case words of `text` (letters and digits, so `node_modules` is `node` and `module`), a plural
/// `s` cut off so `images` and `image` are one word.
fn words_of(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| {
            match word.strip_suffix('s') {
                Some(stem) if stem.chars().count() > 2 && !stem.ends_with('s') => stem,
                _ => word,
            }
            .to_owned()
        })
        .collect()
}

/// The words of a scope text that can name a rule: no stop word, no word every rule has, at least two
/// characters.
fn scope_words(text: &str) -> Vec<String> {
    let skipped: Vec<String> = STOP_WORDS
        .iter()
        .chain(&GENERIC_WORDS)
        .flat_map(|word| words_of(word))
        .collect();
    let mut words: Vec<String> = words_of(text)
        .into_iter()
        .filter(|word| word.chars().count() >= 2 && !skipped.contains(word))
        .collect();
    words.dedup();

    words
}

/// The words of a scope text that can name a folder, the longest first: no stop word, at least two
/// characters, and not the whole text.
fn name_words(text: &str) -> Vec<String> {
    // One word is the text itself, which was tried whole.
    if text.split_whitespace().nth(1).is_none() {
        return Vec::new();
    }

    let mut words: Vec<String> = text
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_'))
        .filter(|word| {
            word.chars().count() >= 2 && *word != text && !STOP_WORDS.contains(&word.to_lowercase().as_str())
        })
        .map(str::to_owned)
        .collect();
    words.sort_by_key(|word| Reverse(word.chars().count()));
    words.dedup();

    words
}

/// The rules that have the most of `words` among their own words (id, title, other names), and of those the
/// ones that fit closest. Rules of equal standing are all named. Nothing is looked up in a scan.
fn rules_for_words<'r>(rules: &'r RuleSet, os: Os, words: &[String]) -> Vec<&'r Rule> {
    let scored: Vec<(&Rule, usize, f64)> = rules
        .rules()
        .iter()
        .filter(|rule| !matches!(rule.matcher, Matcher::User(_)))
        .filter(|rule| rule.os.is_none_or(|only| only == os))
        .filter_map(|rule| {
            let (hits, own) = rule_hits(rule, words);

            (hits > 0).then(|| (rule, hits, hits as f64 / own.max(1) as f64))
        })
        .collect();
    let Some(most) = scored.iter().map(|(_, hits, _)| *hits).max() else {
        return Vec::new();
    };
    let best_fit = scored
        .iter()
        .filter(|(_, hits, _)| *hits == most)
        .map(|(_, _, fit)| *fit)
        .fold(0.0, f64::max);

    scored
        .into_iter()
        .filter(|(_, hits, fit)| *hits == most && *fit * 2.0 >= best_fit)
        .map(|(rule, _, _)| rule)
        .collect()
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();

    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// How many of `words` a rule has, and how many words it has in all (in its id, its title and its other
/// names).
fn rule_hits(rule: &Rule, words: &[String]) -> (usize, usize) {
    let known: HashSet<String> = title_words(rule).chain(alias_words(rule)).collect();

    (
        words.iter().filter(|word| known.contains(*word)).count(),
        known.len(),
    )
}

/// The words of a rule's id and title.
fn title_words(rule: &Rule) -> impl Iterator<Item = String> {
    words_of(&rule.id.replace('-', " "))
        .into_iter()
        .chain(words_of(rule.title))
}

/// The words of the other names a rule goes by.
fn alias_words(rule: &Rule) -> impl Iterator<Item = String> {
    ALIASES
        .iter()
        .filter(|(id, _)| *id == rule.id)
        .flat_map(|(_, names)| names.iter().flat_map(|name| words_of(name)))
}

/// The words of `text`, as written, that name one of `rules`: by the id or title when any word does (`docker`
/// of `docker images`), else by another name they go by. The whole text when none does.
fn named_words(text: &str, rules: &[&Rule]) -> String {
    let titles: HashSet<String> = rules.iter().flat_map(|rule| title_words(rule)).collect();
    let aliases: HashSet<String> = rules.iter().flat_map(|rule| alias_words(rule)).collect();
    let written: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .collect();
    let by = |known: &HashSet<String>| -> Vec<&str> {
        written
            .iter()
            .copied()
            .filter(|word| words_of(word).iter().any(|stem| known.contains(stem)))
            .collect()
    };

    let named = by(&titles);
    let named = if named.is_empty() { by(&aliases) } else { named };

    if named.is_empty() {
        return text.to_owned();
    }

    named.join(" ")
}

/// Where the rule looks, when it has one place: `“Docker data” lives in /var/lib/docker, so scan a folder
/// that holds it`.
fn place_hint(rule: &Rule) -> Option<String> {
    match rule.matcher {
        Matcher::Fixed(path) => Some(format!(
            "“{}” lives in {path}, so scan a folder that holds it",
            rule.title
        )),
        Matcher::Home(path) => Some(format!(
            "“{}” lives in ~/{path}, so scan your home folder",
            rule.title
        )),
        _ => None,
    }
}

/// `user-dirs.dirs` is a few hundred bytes; anything much larger is not that file.
const USER_DIRS_MAX_BYTES: u64 = 64 * 1024;

/// The personal folders that the environment does not list (Downloads, Templates, Public), as this machine's
/// `user-dirs.dirs` sets them (XDG), read only; a folder it does not set is not listed (it is then the
/// usual one in the home).
fn user_dirs_besides_the_environment(home: &Path) -> Vec<(WellKnown, PathBuf)> {
    use std::io::Read as _;

    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let Ok(file) = std::fs::File::open(config.join("user-dirs.dirs")) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if file.take(USER_DIRS_MAX_BYTES).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }

    parse_other_user_dirs(&String::from_utf8_lossy(&bytes), home)
        .into_iter()
        .map(|(role, path)| (role, std::fs::canonicalize(&path).unwrap_or(path)))
        .collect()
}

/// Reads `XDG_DOWNLOAD_DIR="$HOME/..."`, `XDG_TEMPLATES_DIR` and `XDG_PUBLICSHARE_DIR`: double-quoted, and
/// either absolute or below `$HOME`; anything else is ignored, as the XDG tools do.
fn parse_other_user_dirs(text: &str, home: &Path) -> Vec<(WellKnown, PathBuf)> {
    let mut found: Vec<(WellKnown, PathBuf)> = Vec::new();

    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let role = match key.trim() {
            "XDG_DOWNLOAD_DIR" => WellKnown::Downloads,
            "XDG_TEMPLATES_DIR" => WellKnown::Templates,
            "XDG_PUBLICSHARE_DIR" => WellKnown::Public,
            _ => continue,
        };
        let Some(value) = value
            .trim()
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .map(|value| value.replace('\\', ""))
        else {
            continue;
        };

        let path = if let Some(rest) = value.strip_prefix("$HOME/") {
            home.join(rest)
        } else if value.starts_with('/') {
            PathBuf::from(value)
        } else {
            continue;
        };
        // The home itself is how the desktop says "no such folder".
        if path == home || home.starts_with(&path) || path.parent().is_none() {
            continue;
        }

        found.retain(|(kind, _)| *kind != role);
        found.push((role, path));
    }

    found
}

fn verdict_of(verdict: ModelVerdict) -> Verdict {
    match verdict {
        ModelVerdict::Review => Verdict::Review,
        ModelVerdict::Keep => Verdict::Keep,
    }
}

/// `part` of `whole` in whole percent, rounded, but never 100 for less than the whole: 99.6% reads 99. A share
/// under half a percent is 0, which the sentences write as "Less than 1%" when the bytes are not zero.
pub(super) fn percent(part: u64, whole: u64) -> u64 {
    if whole == 0 {
        return 0;
    }

    let rounded = ((u128::from(part) * 100 + u128::from(whole) / 2) / u128::from(whole)) as u64;
    if part < whole { rounded.min(99) } else { rounded }
}

fn category_name(category: Category) -> &'static str {
    match category {
        Category::Video => "Video",
        Category::Image => "Images",
        Category::Audio => "Audio",
        Category::Document => "Documents",
        Category::Archive => "Archives",
        Category::Code => "Code",
        Category::Build => "Build output",
        Category::Package => "Packages",
        Category::App => "Applications",
        Category::DiskImage => "Disk images",
        Category::Data => "Data",
        Category::Other => "Other",
    }
}

mod reading;
mod record;
mod resolve;
#[cfg(test)]
mod tests;

/// The questions asked back lately, with the choices the user was shown for each. A `pick` is checked
/// against them: the web view sends only the id, and the backend reads the question again, so the choice it
/// finds under that id must be the one that was shown.
#[derive(Debug, Default)]
pub struct AskedBack {
    recent: std::sync::Mutex<std::collections::VecDeque<(ScanId, String, Vec<ChoiceKey>)>>,
}

impl AskedBack {
    /// How many questions asked back are kept (the newest first to stay).
    const KEPT: usize = 8;

    fn remember(&self, scan: ScanId, question: &str, choices: Vec<ChoiceKey>) {
        let mut recent = self
            .recent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recent.retain(|(at, asked, _)| !(*at == scan && asked == question));
        recent.push_front((scan, question.to_owned(), choices));
        recent.truncate(Self::KEPT);
    }

    /// The choices shown for `question` of `scan`, or none when it was not asked back lately.
    fn shown(&self, scan: ScanId, question: &str) -> Vec<ChoiceKey> {
        let recent = self
            .recent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recent
            .iter()
            .find(|(at, asked, _)| *at == scan && asked == question)
            .map(|(_, _, choices)| choices.clone())
            .unwrap_or_default()
    }
}
