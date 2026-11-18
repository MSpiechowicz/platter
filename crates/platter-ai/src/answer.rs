//! The answer to a free question, written by Platter from the facts.
//!
//! The model only reads the question (see the `interpret` module). The backend collects the rows and the
//! [`FactSheet`] from one list of folders, and [`template_segments`] writes the answer from the sheet
//! alone: every word of it is Platter's, so nothing a model says can reach the reader. The only text that
//! is not Platter's is a name of the user's disk (a path, a rule title the user wrote, a label's reason the
//! model gave earlier); each is set apart in “ ” quotes.
//!
//! An answer is a list of [`Segment`]s: text, and every path of the sheet as a segment of its own that
//! points at its row, so a reader can show Platter's own verdict next to the folder. A path that the sheet
//! shows for more than one folder (two names that read the same, a real name that looks like a cut path)
//! points at none: it is text, in quotes.

use platter_core::dto::Verdict;
// The size writer lives in the core (the rule reasons use it too); these paths stay for the crate's users.
pub use platter_core::units::{Units, format_size};

use crate::facts::is_invisible;
use crate::question::{Intent, Kind};

/// Longest path written into an answer, in characters (the full path stays in the rows).
pub const SHEET_PATH_CHARS: usize = 160;
/// Most rows in a sheet.
pub const SHEET_ROWS: usize = 10;
/// Most notes in a sheet.
pub const SHEET_NOTES: usize = 6;
/// Longest note, in characters.
pub const NOTE_CHARS: usize = 240;
/// Most rows an answer names in one sentence; the others are counted.
pub const NAMED_ROWS: usize = 5;

/// What the question is told when it is not about the scan.
pub const OFFTOPIC_TEXT: &str = "I can only answer questions about this scan: what takes space, what can be cleaned, why a folder is big, and what the model labelled.";

/// The note of a scan that is from before a cleanup. The sheet says it with [`FactSheet::outdated`].
pub const OUTDATED_NOTE: &str = "This scan is from before a cleanup. Rescan for current figures.";

/// A count with a comma between the thousands: `120,345`.
fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3);

    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }

    text
}

/// `1 item`, `5 items`.
fn items(count: u64) -> String {
    format!("{} {}", grouped(count), if count == 1 { "item" } else { "items" })
}

/// What a path of an answer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The row of the sheet with this index (rows are listed in the same order everywhere).
    Row(usize),
    /// The folder the question was about.
    Scope,
}

/// A piece of an answer: plain text, or a path of the sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    /// A path as the sheet shows it (a long one cut, with `…` in the middle), bound to what it is.
    Path {
        path: String,
        place: Place,
    },
}

/// The substitutes for the quotes that set a text apart, inside the text.
const OPEN_SUBSTITUTE: char = '‟';
const CLOSE_SUBSTITUTE: char = '〃';

/// A name set apart in the text, so that a folder name with a full stop or a command in it cannot be read
/// as part of the sentence: `“~/Projekty/app”`. A `“`, `”` or `"` in the name is shown as `‟` or `〃`, so
/// the name cannot close the quote it is in.
pub fn quote_name(path: &str) -> String {
    format!("“{}”", without_quotes(path))
}

/// `path` with every `“`, `”` or `"` in it shown as `‟` or `〃`: the text [`quote_name`] sets in quotes.
fn without_quotes(path: &str) -> String {
    path.chars()
        .map(|c| match c {
            '“' => OPEN_SUBSTITUTE,
            '”' | '"' => CLOSE_SUBSTITUTE,
            other => other,
        })
        .collect()
}

/// Adds `text` to the segments, to the text before it when there is one.
pub(crate) fn push_text(segments: &mut Vec<Segment>, text: &str) {
    if text.is_empty() {
        return;
    }

    match segments.last_mut() {
        Some(Segment::Text(last)) => last.push_str(text),
        _ => segments.push(Segment::Text(text.to_owned())),
    }
}

/// The segments as one text: every path in “ ” quotes (see [`quote_name`]).
pub fn render(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|segment| match segment {
            Segment::Text(text) => text.clone(),
            Segment::Path { path, .. } => quote_name(path),
        })
        .collect()
}

/// Who made a row's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    /// The safety guard: the folder is left alone.
    Guard,
    /// A rule of the built-in table.
    Rule,
    /// One of the user's own rules.
    User,
    /// A label the local model made earlier.
    Model,
}

/// One row of the sheet: a folder, with what the backend knows about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRow {
    /// The folder's display path (home written as `~`).
    pub path: String,
    /// Bytes on disk. Not known (and 0 here) for a folder that could not be read.
    pub size: u64,
    pub verdict: Option<Verdict>,
    pub by: Option<By>,
    /// By who made the verdict: a rule's title (`Rule`, `User`), the guard's reason (`Guard`) or the model's
    /// reason (`Model`).
    pub note: Option<String>,
    /// The rule's own words on the folder, when a rule covers it.
    pub why: Option<String>,
    /// Whether Platter could not read the folder, so its size is unknown.
    pub unreadable: bool,
}

impl FactRow {
    /// A row with a path and a size and nothing else.
    pub fn new(path: &str, size: u64) -> Self {
        Self {
            path: path.to_owned(),
            size,
            verdict: None,
            by: None,
            note: None,
            why: None,
            unreadable: false,
        }
    }
}

/// What is known of the file a question was about (see [`FactSheet::file`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFact {
    /// Whole days from the file's last change to the time of the scan. `None` when the scan has no
    /// modification time for the file: the answer then leaves its age out.
    pub age_days: Option<u64>,
}

/// What a figure over many folders is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotalKind {
    /// A figure with a label of its own: a family of folders, a media category, a count of labels.
    Plain,
    /// What a verdict totals.
    Verdict(Verdict),
    /// A category's share of the folder (`explain`).
    Category,
    /// The share of the folder that is older than a year (`explain`).
    Older,
    /// What a rule matched inside the folder, labelled by its title (`explain`).
    Rule,
}

/// A figure over many folders: a verdict's total, a category's share, a family's size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactTotal {
    pub label: String,
    pub bytes: Option<u64>,
    pub count: Option<u64>,
    pub kind: TotalKind,
    /// A share of the folder's size, in whole per cent (`Category` and `Older`).
    pub percent: Option<u64>,
    /// A name that stands for the many small items of a verdict's total that are not listed (`__pycache__`).
    pub example: Option<String>,
}

impl FactTotal {
    /// A figure with a label, a size and a count.
    pub fn plain(label: &str, bytes: Option<u64>, count: Option<u64>) -> Self {
        Self {
            label: label.to_owned(),
            bytes,
            count,
            kind: TotalKind::Plain,
            percent: None,
            example: None,
        }
    }

    /// What a verdict totals.
    pub fn of_verdict(verdict: Verdict, bytes: u64, count: Option<u64>) -> Self {
        Self {
            label: verdict_label(verdict).to_owned(),
            bytes: Some(bytes),
            count,
            kind: TotalKind::Verdict(verdict),
            percent: None,
            example: None,
        }
    }
}

/// How a verdict's total is called.
fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Safe => "Safe to clean",
        Verdict::Review => "For you to review",
        Verdict::Keep => "Kept",
    }
}

/// Everything an answer may say, in the form it is written. Built by the backend from one list of folders;
/// the same list gives the rows the user sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSheet {
    pub intent: Intent,
    /// The family of folders asked about (`largest` and `cleanable`).
    pub kind: Kind,
    pub units: Units,
    /// The folder asked about, as a display path; empty for the whole scan.
    pub scope: String,
    /// The title of the rule whose folders (more than one) the question is about, when the scope was
    /// resolved by a rule and not as one folder.
    pub set: Option<String>,
    /// The name looked for (`find`).
    pub name: String,
    /// The media category asked about (`video`, `image`, `audio`), when the question is about one.
    pub category: Option<String>,
    pub scope_size: Option<u64>,
    pub files: Option<u64>,
    /// The rows are the very thing the question named (a folder a rule covers, the folders of a rule), not
    /// what lies inside a folder: the answer says yes or no about them (`cleanable`).
    pub subject: bool,
    pub rows: Vec<FactRow>,
    pub totals: Vec<FactTotal>,
    /// What else the reader should know, each a sentence of its own, written into the answer as it is.
    pub notes: Vec<String>,
    /// The scan is from before a cleanup: the answer says [`OUTDATED_NOTE`].
    pub outdated: bool,
    /// Folders that matched but are not among the rows.
    pub more: u64,
    /// Paths that are not rows, that the notes may still speak of.
    pub context_paths: Vec<String>,
    /// The question was about one file, which is the only row: the answer says what it is and where Platter
    /// stands on it, whatever the intent.
    pub file: Option<FileFact>,
}

impl FactSheet {
    /// A sheet with only an intent and units; the rest is filled in.
    pub fn new(intent: Intent, units: Units) -> Self {
        Self {
            intent,
            kind: Kind::Any,
            units,
            scope: String::new(),
            set: None,
            name: String::new(),
            category: None,
            scope_size: None,
            files: None,
            subject: false,
            rows: Vec::new(),
            totals: Vec::new(),
            notes: Vec::new(),
            outdated: false,
            more: 0,
            context_paths: Vec::new(),
            file: None,
        }
    }
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// `path` at most `chars` characters long: a longer one keeps its beginning and its end and has `…` in
/// the middle, so a folder is not taken for the longer one it is cut from, nor for a sibling, and the end,
/// which tells the folders of one parent apart, stays. The mark does not make the text unique: a real
/// folder's name can hold `…` too, and can read exactly like a cut path. [`Known`] therefore tells the
/// folders apart by their paths before the cut, and a text that is shown for more than one folder is bound
/// to none of them.
fn cut_path(path: &str, chars: usize) -> String {
    let length = path.chars().count();
    if length <= chars {
        return path.to_owned();
    }

    let kept = chars - 1;
    let head = kept.div_ceil(2);
    let tail = kept - head;
    let mut cut: String = path.chars().take(head).collect();
    cut.push('…');
    cut.extend(path.chars().skip(length - tail));

    cut
}

/// The sheet as it is written: at most 10 rows and 6 notes, paths cut to 160 characters and notes to 240.
fn shown(sheet: &FactSheet) -> FactSheet {
    let mut sheet = sheet.clone();

    sheet.more += sheet.rows.len().saturating_sub(SHEET_ROWS) as u64;
    sheet.rows.truncate(SHEET_ROWS);
    sheet.notes.truncate(SHEET_NOTES);
    sheet.scope = cut_path(&sheet.scope, SHEET_PATH_CHARS);
    for row in &mut sheet.rows {
        row.path = cut_path(&row.path, SHEET_PATH_CHARS);
    }
    for note in &mut sheet.notes {
        *note = cut(note, NOTE_CHARS);
    }
    sheet.context_paths = sheet
        .context_paths
        .iter()
        .map(|path| cut_path(path, SHEET_PATH_CHARS))
        .collect();

    sheet
}

// ---------------------------------------------------------------------------------------------------
// Paths

/// `text` with every invisible character a space and white space made one space.
fn plain(text: &str) -> String {
    let visible: String = text
        .chars()
        .map(|c| if is_invisible(c) { ' ' } else { c })
        .collect();

    visible.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A path the sheet holds.
struct KnownPath {
    /// The path as [`quote_name`] shows it: its own quotes as `‟` or `〃`. Two folders whose paths read the
    /// same this way are one text, so `ambiguous`.
    text: String,
    /// The path as the sheet holds it, for the path segment.
    real: String,
    /// The ways the path can stand in a text: `text`, and each path as the sheet holds it when that differs.
    forms: Vec<String>,
    /// The rows shown with this text (none for the scope and the context paths).
    rows: Vec<usize>,
    /// Whether it is the scope.
    scope: bool,
    /// Whether the sheet shows this text for more than one folder: two rows whose names read the same once
    /// invisible characters and white space are gone, a row and a scope or a context path that are other
    /// folders, or a real folder whose name is the cut form (with `…`) of a longer path. It is bound to no
    /// row.
    ambiguous: bool,
}

impl KnownPath {
    /// What the path is in an answer: a row, the scope, or neither (a context path, or a text that is
    /// shown for more than one folder).
    fn place(&self) -> Option<Place> {
        if self.ambiguous {
            return None;
        }

        match self.rows.first() {
            Some(&row) => Some(Place::Row(row)),
            None if self.scope => Some(Place::Scope),
            None => None,
        }
    }
}

/// A folder of the sheet as the answer shows it.
struct Listed {
    /// The path as shown (cut to 160 characters when longer), as the answer is read.
    text: String,
    /// The path before any cut: what makes two folders the same folder.
    identity: String,
    row: Option<usize>,
    scope: bool,
}

/// A folder behind one shown text.
struct Folder {
    identity: String,
    row: Option<usize>,
    scope: bool,
}

/// The paths an answer may bind to a row, longest first (so a path with a longer one beginning the same way
/// is found as the longer). Not `~` or `/` alone: they name nothing in particular.
///
/// A text is one folder only when one folder has it. A cut path is marked with `…`, but a real folder's name
/// can hold `…` too, and invisible characters, white space and a lossy reading of a name that is no UTF-8
/// make two names read the same. The folders are told apart by their path before any cut (the scope and a
/// context path are the row whose path they are), so a text with more than one folder behind it is
/// `ambiguous`.
struct Known {
    paths: Vec<KnownPath>,
}

impl Known {
    /// `original` is the sheet as the backend made it, `sheet` the same as it is written ([`shown`]).
    fn new(original: &FactSheet, sheet: &FactSheet) -> Self {
        let mut listed: Vec<Listed> = Vec::new();
        let mut add = |shown: &str, original: &str, row: Option<usize>, scope: bool| {
            listed.push(Listed {
                text: plain(shown.trim_end_matches('/')),
                identity: original.trim_end_matches('/').to_owned(),
                row,
                scope,
            });
        };

        for (index, row) in sheet.rows.iter().enumerate() {
            let before = original
                .rows
                .get(index)
                .map_or(row.path.as_str(), |row| row.path.as_str());
            add(&row.path, before, Some(index), false);
        }
        if !sheet.scope.is_empty() {
            add(&sheet.scope, &original.scope, None, true);
        }
        for (index, path) in sheet.context_paths.iter().enumerate() {
            let before = original.context_paths.get(index).unwrap_or(path);
            add(path, before, None, false);
        }

        // Per text (as `quote_name` shows it): the ways it is written, and the folders behind it.
        let mut grouped: Vec<(String, String, Vec<String>, Vec<Folder>)> = Vec::new();
        for item in listed {
            if item.text.chars().count() < 2 {
                continue;
            }

            let key = without_quotes(&item.text);
            let at = match grouped.iter().position(|(text, ..)| *text == key) {
                Some(at) => at,
                None => {
                    grouped.push((key.clone(), item.text.clone(), vec![key], Vec::new()));
                    grouped.len() - 1
                }
            };
            let (_, _, forms, folders) = &mut grouped[at];
            if !forms.contains(&item.text) {
                forms.push(item.text.clone());
            }

            // A row is a folder of its own; the scope and a context path are the folder they say.
            let same = match item.row {
                Some(_) => None,
                None => folders.iter_mut().find(|folder| folder.identity == item.identity),
            };
            match same {
                Some(folder) => folder.scope |= item.scope,
                None => folders.push(Folder {
                    identity: item.identity,
                    row: item.row,
                    scope: item.scope,
                }),
            }
        }

        let mut paths: Vec<KnownPath> = grouped
            .into_iter()
            .map(|(text, real, mut forms, folders)| KnownPath {
                real,
                // Longest first, like the paths themselves.
                forms: {
                    forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
                    forms
                },
                rows: folders.iter().filter_map(|folder| folder.row).collect(),
                scope: folders.iter().any(|folder| folder.scope),
                ambiguous: folders.len() > 1,
                text,
            })
            .collect();

        let longest = |path: &KnownPath| path.forms.iter().map(String::len).max().unwrap_or(0);
        paths.sort_by(|a, b| longest(b).cmp(&longest(a)).then_with(|| a.text.cmp(&b.text)));
        Self { paths }
    }

    /// Whether the sheet shows `path` (a path as the backend wrote it) for more than one folder.
    fn is_ambiguous(&self, path: &str) -> bool {
        let text = without_quotes(&plain(path.trim_end_matches('/')));

        self.paths
            .iter()
            .any(|known| known.ambiguous && known.text == text)
    }

    /// The longest known path that starts at byte `at` of `text` as a whole word: its index and length.
    fn match_at(&self, text: &str, at: usize) -> Option<(usize, usize)> {
        if !starts_a_word(text, at) {
            return None;
        }

        // A path is found as it is, or as `quote_name` wrote it (its own quotes shown as `‟` or `〃`).
        self.paths.iter().enumerate().find_map(|(index, path)| {
            path.forms.iter().find_map(|form| {
                let end = at + form.len();

                (text[at..].starts_with(form.as_str()) && ends_a_word(text, end))
                    .then_some((index, form.len()))
            })
        })
    }

    /// `text` in pieces: plain text, and every known path in it as a path segment. A path in quotes loses
    /// them (the answer writes its own “ ” around every path); a path that is bound to no folder stays in
    /// the text, in quotes.
    fn segments_of(&self, text: &str) -> Vec<Segment> {
        let text = plain(text);
        let mut segments: Vec<Segment> = Vec::new();
        let mut loose = String::new();
        let mut at = 0;

        while let Some(c) = text[at..].chars().next() {
            let Some((index, length)) = self.match_at(&text, at) else {
                loose.push(c);
                at += c.len_utf8();
                continue;
            };
            let path = &self.paths[index];
            at += length;

            let closer = loose.chars().next_back().and_then(closer_of);
            if let (Some(closer), Some(next)) = (closer, text[at..].chars().next())
                && next == closer
            {
                loose.pop();
                at += next.len_utf8();
            }

            match path.place() {
                Some(place) => {
                    push_text(&mut segments, &loose);
                    loose.clear();
                    segments.push(Segment::Path {
                        path: path.real.clone(),
                        place,
                    });
                }
                None => loose.push_str(&quote_name(&path.text)),
            }
        }
        push_text(&mut segments, &loose);

        segments
    }
}

fn starts_a_word(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{' | '"' | '\'' | '`' | '“' | '‘'))
}

fn ends_a_word(text: &str, at: usize) -> bool {
    let mut after = text[at..].chars();

    match after.next() {
        None => true,
        Some('.') => after.next().is_none_or(char::is_whitespace),
        Some(c) => {
            c.is_whitespace()
                || matches!(
                    c,
                    ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '"' | '\'' | '`' | '”' | '’'
                )
        }
    }
}

/// The quote that closes `open`, for a path a note puts in quotes.
fn closer_of(open: char) -> Option<char> {
    match open {
        '“' => Some('”'),
        '‘' => Some('’'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------------
// The text

/// The answer written from the sheet alone, as text: the text of [`template_segments`], every path in
/// “ ” quotes.
pub fn template(sheet: &FactSheet) -> String {
    render(&template_segments(sheet))
}

/// The answer written from the sheet alone, in pieces. The first sentence answers the question (see the
/// writer of each intent), then come the figures the sheet holds and its notes, each written as it is.
pub fn template_segments(sheet: &FactSheet) -> Vec<Segment> {
    if sheet.intent == Intent::Offtopic {
        return vec![Segment::Text(OFFTOPIC_TEXT.to_owned())];
    }

    let original = sheet;
    let sheet = shown(original);
    let known = Known::new(original, &sheet);
    let writer = Writer {
        sheet: &sheet,
        known: &known,
    };

    let mut sentences = writer.body();
    // A note may be the reason there is nothing to list; then it says so by itself. What the model has not
    // labelled is said here, and its note only says what to do about it.
    if sentences.is_empty() && (sheet.notes.is_empty() || sheet.intent == Intent::Labelled) {
        sentences.push(writer.nothing());
    }
    for note in &sheet.notes {
        sentences.push(writer.note(note));
    }
    if sheet.outdated {
        sentences.push(writer.note(OUTDATED_NOTE));
    }

    let mut segments: Vec<Segment> = Vec::new();
    for (at, sentence) in sentences.into_iter().enumerate() {
        if at > 0 {
            push_text(&mut segments, " ");
        }
        for part in sentence {
            match part {
                Segment::Text(text) => push_text(&mut segments, &text),
                path => segments.push(path),
            }
        }
    }

    segments
}

/// What a verdict says of a folder the question named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Safe,
    Review,
    Kept,
}

/// A row of the sheet with its index.
type Indexed<'a> = (usize, &'a FactRow);

/// One sentence, built piece by piece.
struct Line<'a> {
    parts: Vec<Segment>,
    known: &'a Known,
    units: Units,
    /// A row that could not be read says `(could not be read)` where its size would be.
    mark_unreadable: bool,
}

impl Line<'_> {
    fn text(&mut self, text: &str) {
        push_text(&mut self.parts, text);
    }

    /// A text of the user's disk or of the model (a rule title, a reason), set apart in quotes.
    fn quoted(&mut self, text: &str) {
        push_text(&mut self.parts, &quote_name(text));
    }

    /// A folder's path: a path segment bound to its place, or, when the sheet shows that text for more than
    /// one folder, text in quotes that is bound to none of them.
    fn path(&mut self, path: &str, place: Place) {
        if self.known.is_ambiguous(path) {
            self.quoted(path);
            return;
        }

        self.parts.push(Segment::Path {
            path: path.to_owned(),
            place,
        });
    }

    fn size(&mut self, bytes: u64) {
        let text = format_size(bytes, self.units);

        self.text(&text);
    }

    /// Text of the sheet (a note) with the paths it holds as segments, so that a path inside it is shown
    /// like any other.
    fn sheet_text(&mut self, text: &str) {
        for segment in self.known.segments_of(text) {
            match segment {
                Segment::Text(text) => self.text(&text),
                path => self.parts.push(path),
            }
        }
    }

    /// A row as the answer names it: its path, then its size in brackets, or `(could not be read)` for a
    /// folder whose size is unknown.
    fn row(&mut self, index: usize, row: &FactRow) {
        self.path(&row.path, Place::Row(index));
        if row.unreadable {
            if self.mark_unreadable {
                self.text(" (could not be read)");
            }
            return;
        }

        self.text(" (");
        self.size(row.size);
        self.text(")");
    }

    /// The first rows as a list, `a, b and c`, then how many more there are (`more` are not rows).
    fn rows(&mut self, rows: &[Indexed<'_>], more: u64) {
        let named = rows.len().min(NAMED_ROWS);
        let left = (rows.len() - named) as u64 + more;
        let count = named + usize::from(left > 0);

        for (at, (index, row)) in rows.iter().take(named).enumerate() {
            self.separator(at, count);
            self.row(*index, row);
        }
        if left > 0 {
            self.separator(named, count);
            self.text(&format!("{} more", grouped(left)));
        }
    }

    /// What goes before the item `at` of `count`: nothing, a comma or `and`.
    fn separator(&mut self, at: usize, count: usize) {
        if at == 0 {
            return;
        }

        self.text(if at + 1 == count { " and " } else { ", " });
    }

    fn done(self) -> Vec<Segment> {
        self.parts
    }
}

/// The written form of the one intent the sheet is for.
struct Writer<'a> {
    sheet: &'a FactSheet,
    known: &'a Known,
}

impl Writer<'_> {
    fn line(&self) -> Line<'_> {
        Line {
            parts: Vec::new(),
            known: self.known,
            units: self.sheet.units,
            mark_unreadable: true,
        }
    }

    /// A sentence that is only text.
    fn said(&self, text: &str) -> Vec<Segment> {
        let mut line = self.line();
        line.text(text);

        line.done()
    }

    /// A note of the sheet, as it is.
    fn note(&self, note: &str) -> Vec<Segment> {
        let note = note.trim();
        let mut line = self.line();

        line.sheet_text(note);
        if !note.ends_with(['.', '!', '?']) {
            line.text(".");
        }

        line.done()
    }

    fn rows(&self) -> Vec<Indexed<'_>> {
        self.sheet.rows.iter().enumerate().collect()
    }

    fn rows_with(&self, verdict: Verdict) -> Vec<Indexed<'_>> {
        self.rows()
            .into_iter()
            .filter(|(_, row)| row.verdict == Some(verdict))
            .collect()
    }

    fn total_of(&self, kind: TotalKind) -> Option<&FactTotal> {
        self.sheet.totals.iter().find(|total| total.kind == kind)
    }

    /// Where the question was about: ` in “~/Projekty”`, ` in this scan` or ` among the “Docker data”
    /// folders`.
    fn in_scope(&self, line: &mut Line<'_>) {
        if !self.sheet.scope.is_empty() {
            line.text(" in ");
            line.path(&self.sheet.scope, self.scope_place());
        } else if let Some(set) = &self.sheet.set {
            line.text(" among the ");
            line.quoted(set);
            line.text(" folders");
        } else {
            line.text(" in this scan");
        }
    }

    /// The folder asked about, as the subject of a sentence: `This scan` or `“~/Projekty”`.
    fn subject(&self, line: &mut Line<'_>) {
        if self.sheet.scope.is_empty() {
            line.text("This scan");
        } else {
            line.path(&self.sheet.scope, self.scope_place());
        }
    }

    /// A path segment bound to the row with the scope's path when there is one, to the scope otherwise.
    fn scope_place(&self) -> Place {
        self.sheet
            .rows
            .iter()
            .position(|row| row.path == self.sheet.scope)
            .map_or(Place::Scope, Place::Row)
    }

    /// `Subject holds 31 GB in 120,345 files.`
    fn holds(&self) -> Option<Vec<Segment>> {
        let size = self.sheet.scope_size?;
        let mut line = self.line();

        self.subject(&mut line);
        line.text(" holds ");
        line.size(size);
        if let Some(files) = self.sheet.files {
            line.text(&format!(
                " in {} {}",
                grouped(files),
                if files == 1 { "file" } else { "files" }
            ));
        }
        line.text(".");

        Some(line.done())
    }

    /// `Label: 12 GB in 4 items.`
    fn total(&self, total: &FactTotal) -> Option<Vec<Segment>> {
        let mut line = self.line();

        line.text(&total.label);
        line.text(": ");
        match (total.bytes, total.count) {
            // A total can count files too (a matched file is counted though it is not listed), and the sheet
            // does not say which: they are "items", never "folders".
            (Some(bytes), Some(count)) => {
                line.size(bytes);
                line.text(&format!(" in {}", items(count)));
            }
            (Some(bytes), None) => line.size(bytes),
            (None, Some(count)) => line.text(&grouped(count)),
            (None, None) => return None,
        }
        line.text(".");

        Some(line.done())
    }

    fn body(&self) -> Vec<Vec<Segment>> {
        if let Some(file) = &self.sheet.file
            && let Some(row) = self.sheet.rows.first()
        {
            return self.file(file, row);
        }

        match self.sheet.intent {
            Intent::Largest => self.largest(),
            Intent::Cleanable => self.cleanable(),
            Intent::Labelled => self.labelled(),
            Intent::Explain => self.explain(),
            Intent::Find => self.find(),
            Intent::General => self.general(),
            Intent::Offtopic => Vec::new(),
        }
    }

    /// The sentence for a question that found nothing and has no note to say why.
    fn nothing(&self) -> Vec<Segment> {
        let mut line = self.line();

        match self.sheet.intent {
            Intent::Largest => {
                match &self.sheet.category {
                    Some(category) => line.text(&format!("No {category} folders were found")),
                    None => line.text(&format!("No {} were found", family(self.sheet.kind).1)),
                }
                self.in_scope(&mut line);
            }
            Intent::Cleanable => {
                line.text("Nothing to clean was found");
                self.in_scope(&mut line);
            }
            Intent::Labelled => {
                line.text("The model has not labelled any folder");
                self.in_scope(&mut line);
            }
            Intent::Explain => {
                line.text("There is nothing to report");
                self.in_scope(&mut line);
            }
            Intent::Find => line.text("No matching folder was found"),
            Intent::General | Intent::Offtopic => line.text("No folders were found in this scan"),
        }
        line.text(".");

        line.done()
    }

    // ---- The intents ----

    /// `The biggest folder in “~/Projekty” is “a” (7.9 GB), then “b” (3.3 GB) and “c” (1 GB).`
    ///
    /// About the whole scan the biggest folders come first and the size of the scan follows. About one
    /// thing (a folder, the folders of a rule, a media category) its size comes first, because a question
    /// about it (`how big is my docker data?`) is a question about that.
    fn largest(&self) -> Vec<Vec<Segment>> {
        let rows = self.rows();
        let about_one_thing =
            !self.sheet.scope.is_empty() || self.sheet.set.is_some() || self.sheet.category.is_some();
        let mut sentences = Vec::new();

        let mut sizes: Vec<Vec<Segment>> = Vec::new();
        match &self.sheet.category {
            Some(category) => sizes.extend(self.media_total(category)),
            None => {
                sizes.extend(self.holds());
                for total in &self.sheet.totals {
                    sizes.extend(self.total(total));
                }
            }
        }
        if about_one_thing {
            sentences.append(&mut sizes);
        }

        if let Some((first, rest)) = rows.split_first() {
            let mut line = self.line();

            line.text(&format!("The biggest {}", self.family_name().0));
            self.in_scope(&mut line);
            line.text(" is ");
            line.row(first.0, first.1);
            if !rest.is_empty() {
                // The first row is named on its own, so one less is named after it.
                let named = rest.len().min(NAMED_ROWS - 1);

                line.text(", then ");
                line.rows(&rest[..named], (rest.len() - named) as u64);
            }
            line.text(".");
            sentences.push(line.done());
        }

        sentences.append(&mut sizes);

        sentences
    }

    /// The family of folders asked about, in the singular and the plural: a media category when there is one.
    fn family_name(&self) -> (String, String) {
        match &self.sheet.category {
            Some(category) => (format!("{category} folder"), format!("{category} folders")),
            None => {
                let (one, many) = family(self.sheet.kind);

                (one.to_owned(), many.to_owned())
            }
        }
    }

    /// `Video folders in this scan take 2.9 GB in total (33 folders).`
    fn media_total(&self, category: &str) -> Option<Vec<Segment>> {
        let total = self.sheet.totals.first()?;
        let bytes = total.bytes?;
        let mut line = self.line();

        line.text(&capitalised(category));
        line.text(" folders");
        self.in_scope(&mut line);
        line.text(" take ");
        line.size(bytes);
        line.text(" in total");
        if let Some(count) = total.count {
            line.text(&format!(
                " ({} {})",
                grouped(count),
                if count == 1 { "folder" } else { "folders" }
            ));
        }
        line.text(".");

        Some(line.done())
    }

    fn cleanable(&self) -> Vec<Vec<Segment>> {
        if self.sheet.subject && !self.sheet.rows.is_empty() {
            return self.verdict_of_subject();
        }

        let mut sentences = Vec::new();
        let mut first = true;

        for verdict in [Verdict::Safe, Verdict::Review, Verdict::Keep] {
            let of_verdict = self.rows_with(verdict);
            let total = self.total_of(TotalKind::Verdict(verdict));
            if of_verdict.is_empty() && total.is_none() {
                continue;
            }

            let mut line = self.line();
            line.text(verdict_label(verdict));
            if first {
                self.in_scope_named(&mut line);
                first = false;
            }
            line.text(":");
            self.listed(&mut line, &of_verdict, total);
            sentences.push(line.done());
        }

        // Nothing to clean in a folder: what it is, so the answer is about the folder and not about nothing.
        // The notes say where Platter stands on it.
        if sentences.is_empty() && !self.sheet.scope.is_empty() {
            sentences.extend(self.holds());
        }

        sentences
    }

    /// ` in “scope”` when the question named a scope; nothing for the whole scan (the lists say it all).
    fn in_scope_named(&self, line: &mut Line<'_>) {
        if !self.sheet.scope.is_empty() || self.sheet.set.is_some() {
            self.in_scope(line);
        }
    }

    /// ` 6.4 GB in 3 items, the largest being “a” (4 GB) and “b” (2 GB).`
    ///
    /// The rows that are a small part of the total are not named one by one: they are counted with the items
    /// that are not rows (`and 4,674 smaller items such as “__pycache__”`).
    fn listed(&self, line: &mut Line<'_>, rows: &[Indexed<'_>], total: Option<&FactTotal>) {
        let counted = total.and_then(|total| Some((total.bytes?, total.count?)));

        match counted {
            // A folder that is the whole total needs no total.
            Some((_, 1)) if rows.len() == 1 => {
                line.text(" ");
                line.rows(rows, 0);
            }
            Some((bytes, count)) if !rows.is_empty() => {
                line.text(" ");
                line.size(bytes);
                line.text(&format!(" in {}", items(count)));

                // A row is named when it is at least 2% of the total; the first one always is.
                let named: Vec<Indexed<'_>> = rows
                    .iter()
                    .copied()
                    .enumerate()
                    .filter(|(at, (_, row))| *at == 0 || u128::from(row.size) * 50 >= u128::from(bytes))
                    .map(|(_, indexed)| indexed)
                    .take(NAMED_ROWS)
                    .collect();
                let left = count.saturating_sub(named.len() as u64);
                let left_bytes = bytes.saturating_sub(named.iter().map(|(_, row)| row.size).sum::<u64>());
                // What is left is small when the average of it is under 2% of the total.
                let small = left > 0 && u128::from(left_bytes) * 50 < u128::from(bytes) * u128::from(left);

                line.text(if left > 0 {
                    ", the largest being "
                } else {
                    ", namely "
                });
                let listed = named.len() + usize::from(left > 0);
                for (at, (index, row)) in named.iter().enumerate() {
                    line.separator(at, listed);
                    line.row(*index, row);
                }
                if left > 0 {
                    line.separator(named.len(), listed);
                    line.text(&format!(
                        "{} {} {}",
                        grouped(left),
                        if small { "smaller" } else { "more" },
                        if left == 1 { "item" } else { "items" }
                    ));
                    let example = total.and_then(|total| total.example.clone()).or_else(|| {
                        rows.iter()
                            .find(|indexed| !named.iter().any(|named| named.0 == indexed.0))
                            .map(|(_, row)| base_name(&row.path))
                    });
                    if let Some(example) = example.filter(|example| small && !example.is_empty()) {
                        line.text(" such as ");
                        line.quoted(&example);
                    }
                }
            }
            Some((bytes, count)) => {
                line.text(" ");
                line.size(bytes);
                line.text(&format!(" in {}", items(count)));
            }
            None => {
                line.text(" ");
                line.rows(rows, 0);
            }
        }
        line.text(".");
    }

    /// The answer to a question about a thing itself: `Yes: “a” (4 GB) is safe to clean (Rust build output).`
    fn verdict_of_subject(&self) -> Vec<Vec<Segment>> {
        let rows = self.rows();
        // A row with no verdict is no one's to call safe: it is for a look.
        let group_of = |row: &FactRow| match row.verdict {
            Some(Verdict::Safe) => Group::Safe,
            Some(Verdict::Keep) => Group::Kept,
            _ => Group::Review,
        };
        let groups: Vec<(Group, Vec<Indexed<'_>>)> = [Group::Safe, Group::Review, Group::Kept]
            .into_iter()
            .map(|group| {
                let of_group: Vec<Indexed<'_>> = rows
                    .iter()
                    .copied()
                    .filter(|(_, row)| group_of(row) == group)
                    .collect();

                (group, of_group)
            })
            .filter(|(_, of_group)| !of_group.is_empty())
            .collect();

        let mut line = self.line();
        line.mark_unreadable = false;
        line.text(match groups.as_slice() {
            [(Group::Safe, _)] => "Yes: ",
            [(Group::Review, _)] => "No, not without a look: ",
            [(Group::Kept, _)] => "No: ",
            _ => "Partly: ",
        });
        for (at, (group, of_group)) in groups.iter().enumerate() {
            line.separator(at, groups.len());
            self.clause(&mut line, *group, of_group);
        }
        line.text(".");

        let mut sentences = vec![line.done()];
        // Only the largest are named; the others are said to exist.
        if self.sheet.more > 0 {
            sentences.push(self.said(&format!(
                "Only the largest {} named here, and {} more match.",
                if rows.len() == 1 {
                    "one is".to_owned()
                } else {
                    format!("{} are", rows.len())
                },
                grouped(self.sheet.more)
            )));
        }
        sentences.extend(self.reasons(&rows));

        sentences
    }

    /// `“a” (4 GB) is safe to clean`, `“b” is kept`, `“c” is left alone by Platter`, with the rule's title
    /// when the clause is about one folder that a rule decided.
    fn clause(&self, line: &mut Line<'_>, group: Group, rows: &[Indexed<'_>]) {
        let one = rows.len() == 1;
        let guarded = rows.iter().all(|(_, row)| row.by == Some(By::Guard));

        line.rows(rows, 0);
        line.text(match (group, one) {
            (Group::Safe, true) => " is safe to clean",
            (Group::Safe, false) => " are safe to clean",
            (Group::Review, true) => " needs your review",
            (Group::Review, false) => " need your review",
            (Group::Kept, true) if guarded => " is left alone by Platter",
            (Group::Kept, false) if guarded => " are left alone by Platter",
            (Group::Kept, true) => " is kept",
            (Group::Kept, false) => " are kept",
        });
        self.titled(line, rows);
    }

    /// ` (Rust build output)` after a clause about one folder that a rule decided.
    fn titled(&self, line: &mut Line<'_>, rows: &[Indexed<'_>]) {
        let [(_, row)] = rows else {
            return;
        };
        if !matches!(row.by, Some(By::Rule | By::User)) {
            return;
        }
        let Some(title) = row.note.as_deref().filter(|title| !title.is_empty()) else {
            return;
        };

        line.text(" (");
        line.quoted(title);
        line.text(")");
    }

    /// What more to say about the folder the question was about: the rule's own words, why the guard leaves
    /// it alone, that its size is unknown.
    fn reasons(&self, rows: &[Indexed<'_>]) -> Vec<Vec<Segment>> {
        let mut sentences = Vec::new();

        if let [(_, row)] = rows {
            // The words of a rule or of the guard, unless a note says just that, or the folder is only
            // unreadable (said below), or it is safe (the rule's reason for it is no warning).
            let said = |why: &str| self.sheet.notes.iter().any(|note| note.trim() == why.trim());
            let skipped = row.verdict == Some(Verdict::Safe) || (row.by == Some(By::Guard) && row.unreadable);

            if let Some(why) = row.why.as_deref().filter(|why| !skipped && !said(why)) {
                sentences.push(self.note(why));
            }
        }

        let unreadable: Vec<Indexed<'_>> = rows.iter().copied().filter(|(_, row)| row.unreadable).collect();
        match (unreadable.len(), rows.len()) {
            (0, _) => {}
            (1, 1) => sentences.push(self.said("Platter could not read it, so its size is unknown.")),
            (count, total) if count == total => {
                sentences.push(self.said("Platter could not read them, so their sizes are unknown."));
            }
            // Some of them: said by name, so it is clear which.
            (count, _) => {
                let mut line = self.line();

                line.text("Platter could not read ");
                for (at, (index, row)) in unreadable.iter().take(NAMED_ROWS).enumerate() {
                    line.separator(at, unreadable.len().min(NAMED_ROWS));
                    line.path(&row.path, Place::Row(*index));
                }
                line.text(if count == 1 {
                    ", so its size is unknown."
                } else {
                    ", so their sizes are unknown."
                });
                sentences.push(line.done());
            }
        }

        sentences
    }

    /// The model's labels: how many and for what, then each with the reason the model gave.
    fn labelled(&self) -> Vec<Vec<Segment>> {
        let mut sentences = Vec::new();

        let verdicts = [(Verdict::Review, "to review"), (Verdict::Keep, "to keep")];
        let totals: Vec<(&FactTotal, &str)> = verdicts
            .iter()
            .filter_map(|(verdict, words)| Some((self.total_of(TotalKind::Verdict(*verdict))?, *words)))
            .collect();
        let count: u64 = totals.iter().filter_map(|(total, _)| total.count).sum();
        // One folder is said by its own sentence.
        if count > 1 {
            let mut line = self.line();

            line.text(&format!(
                "The model labelled {} {}",
                grouped(count),
                if count == 1 { "folder" } else { "folders" }
            ));
            self.in_scope_named(&mut line);
            line.text(": ");
            for (at, (total, words)) in totals.iter().enumerate() {
                line.separator(at, totals.len());
                line.text(&format!("{} {words}", grouped(total.count.unwrap_or(0))));
                if let Some(bytes) = total.bytes {
                    line.text(" (");
                    line.size(bytes);
                    line.text(")");
                }
            }
            line.text(".");
            sentences.push(line.done());
        }

        for (index, row) in self.rows().into_iter().take(NAMED_ROWS) {
            let mut line = self.line();

            line.text("The model marked ");
            line.row(index, row);
            line.text(if row.verdict == Some(Verdict::Keep) {
                " to keep"
            } else {
                " for review"
            });
            if let Some(reason) = row
                .note
                .as_deref()
                .map(reason_of)
                .filter(|reason| !reason.is_empty())
            {
                line.text(": ");
                line.quoted(&reason);
            }
            line.text(".");
            sentences.push(line.done());
        }

        let unnamed = self.sheet.rows.len().saturating_sub(NAMED_ROWS) as u64 + self.sheet.more;
        if unnamed > 0 {
            sentences.push(self.said(&format!(
                "{} more labelled {} not listed.",
                grouped(unnamed),
                if unnamed == 1 { "folder is" } else { "folders are" }
            )));
        }

        sentences
    }

    /// Size, then what the folder is made of, how old it is, what the rules matched in it and its largest
    /// parts.
    fn explain(&self) -> Vec<Vec<Segment>> {
        let mut sentences: Vec<Vec<Segment>> = Vec::new();
        sentences.extend(self.holds());

        let categories: Vec<&FactTotal> = self
            .sheet
            .totals
            .iter()
            .filter(|total| total.kind == TotalKind::Category)
            .collect();
        // One type that is nearly all of it says nothing: the folder is what it is.
        let one_type = categories
            .first()
            .and_then(|total| total.percent)
            .is_some_and(|percent| percent >= 95);
        if !categories.is_empty() && !one_type {
            let mut line = self.line();

            line.text(if categories.len() == 1 {
                "The largest part by type is "
            } else {
                "The largest parts by type are "
            });
            for (at, total) in categories.iter().enumerate() {
                line.separator(at, categories.len());
                line.text(&total.label);
                line.text(" (");
                if let Some(percent) = total.percent {
                    line.text(&format!("{percent}%"));
                }
                if let Some(bytes) = total.bytes {
                    line.text(if total.percent.is_some() { ", " } else { "" });
                    line.size(bytes);
                }
                line.text(")");
            }
            line.text(".");
            sentences.push(line.done());
        }

        if let Some(older) = self.total_of(TotalKind::Older) {
            let mut line = self.line();

            match (older.bytes, older.percent) {
                (Some(0), _) => line.text("None of it is older than a year."),
                (Some(bytes), Some(0)) => {
                    line.text("Less than 1% of it (");
                    line.size(bytes);
                    line.text(") is older than a year.");
                }
                (Some(bytes), Some(percent)) => {
                    line.text(&format!("{percent}% of it ("));
                    line.size(bytes);
                    line.text(") is older than a year.");
                }
                _ => line.text("Some of it is older than a year."),
            }
            sentences.push(line.done());
        }

        let rules: Vec<&FactTotal> = self
            .sheet
            .totals
            .iter()
            .filter(|total| total.kind == TotalKind::Rule)
            .collect();
        if !rules.is_empty() {
            let mut line = self.line();

            line.text("Platter's rules match ");
            for (at, total) in rules.iter().enumerate() {
                line.separator(at, rules.len());
                line.quoted(&total.label);
                if let Some(bytes) = total.bytes {
                    line.text(" (");
                    line.size(bytes);
                    line.text(")");
                }
            }
            line.text(" inside it.");
            sentences.push(line.done());
        }

        let rows = self.rows();
        if !rows.is_empty() {
            let mut line = self.line();

            line.text(if rows.len() == 1 {
                "The biggest part is "
            } else {
                "The biggest parts are "
            });
            line.rows(&rows, 0);
            line.text(".");
            sentences.push(line.done());
        }

        sentences
    }

    /// The answer about one file, for every intent: what it is, then where Platter stands on it.
    ///
    /// `“~/Downloads/old.iso” is a file of 4.2 GB, last changed 1 year ago. It is safe to clean (“Old
    /// downloads”).`
    fn file(&self, file: &FileFact, row: &FactRow) -> Vec<Vec<Segment>> {
        let mut line = self.line();
        line.mark_unreadable = false;

        line.path(&row.path, Place::Row(0));
        line.text(" is a file of ");
        line.size(row.size);
        if let Some(days) = file.age_days {
            line.text(", last changed ");
            line.text(&age_words(days));
        }
        line.text(".");

        let mut sentences = vec![line.done()];
        sentences.extend(self.standing(row));

        sentences
    }

    /// Where Platter stands on a file: the guard first, then a rule, then the model's label, else that no
    /// rule covers it.
    fn standing(&self, row: &FactRow) -> Vec<Vec<Segment>> {
        let titled = |words: &str| {
            let mut line = self.line();

            line.text(words);
            if let Some(title) = row.note.as_deref().filter(|title| !title.is_empty()) {
                line.text(" (");
                line.quoted(title);
                line.text(")");
            }
            line.text(".");

            line.done()
        };

        match (row.by, row.verdict) {
            (Some(By::Guard), _) => {
                let mut sentences = vec![self.said("Platter leaves it alone.")];
                sentences.extend(row.why.as_deref().map(|why| self.note(why)));

                sentences
            }
            (Some(By::Rule | By::User), Some(Verdict::Safe)) => vec![titled("It is safe to clean")],
            (Some(By::Rule | By::User), Some(Verdict::Keep)) => vec![titled("It is kept")],
            (Some(By::Rule | By::User), _) => {
                let mut sentences = vec![titled("It needs your review")];
                sentences.extend(row.why.as_deref().map(|why| self.note(why)));

                sentences
            }
            (Some(By::Model), Some(Verdict::Keep)) => vec![self.said("The local model marked it to keep.")],
            (Some(By::Model), _) => vec![self.said("The local model marked it for review.")],
            (None, _) => vec![self.said("No rule covers it, so deleting it is your call.")],
        }
    }

    /// `Matches for “node_modules”: “a” (1 GB), “b” (2 GB) and 3 more.`
    fn find(&self) -> Vec<Vec<Segment>> {
        let rows = self.rows();
        if rows.is_empty() {
            return Vec::new();
        }

        let mut line = self.line();

        if self.sheet.name.is_empty() {
            line.text("Folders found: ");
        } else {
            line.text("Matches for ");
            line.quoted(&self.sheet.name);
            line.text(": ");
        }
        line.rows(&rows, self.sheet.more);
        line.text(".");

        vec![line.done()]
    }

    /// The biggest folders, the size of the scan and what the suggestions total.
    fn general(&self) -> Vec<Vec<Segment>> {
        let mut sentences: Vec<Vec<Segment>> = Vec::new();

        let rows = self.rows();
        if !rows.is_empty() {
            let mut line = self.line();

            line.text(if rows.len() == 1 {
                "The biggest folder is "
            } else {
                "The biggest folders are "
            });
            line.rows(&rows, 0);
            line.text(".");
            sentences.push(line.done());
        }

        sentences.extend(self.holds());
        for total in &self.sheet.totals {
            sentences.extend(self.total(total));
        }

        sentences
    }
}

/// How long ago a file was last changed: `today`, `1 day ago`, `12 days ago`, `3 months ago`, `2 years ago`.
/// Up to 30 days it counts days, under a year months of 30 days, then years of 365 days.
fn age_words(days: u64) -> String {
    let (count, unit) = match days {
        0 => return "today".to_owned(),
        1..=30 => (days, "day"),
        31..=364 => (days / 30, "month"),
        _ => (days / 365, "year"),
    };

    format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" })
}

/// How a family of folders is called, in the singular and the plural.
fn family(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Any => ("folder", "folders"),
        Kind::Cache => ("cache", "caches"),
        Kind::Build => (
            "build output or dependency folder",
            "build output or dependency folders",
        ),
        Kind::Media => ("media folder", "media folders"),
    }
}

/// The last part of a path as the answer shows it.
fn base_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_owned()
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();

    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The reason the model gave, as it is quoted: on one line, without a closing full stop.
fn reason_of(note: &str) -> String {
    plain(note).trim_end_matches(['.', '!', '?', ' ']).to_owned()
}
