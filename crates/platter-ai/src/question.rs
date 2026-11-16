//! A free question about a scan, once the model has read it into a query.
//!
//! The model sees only the question and answers with an [`Intent`], a scope text, a [`Kind`] and a
//! limit. Nothing here is trusted: [`ground`] keeps a scope only when the question really contains it,
//! so a scope the model made up never reaches the folder lookup, and the lookup itself only looks inside
//! the finished scan.

use crate::facts::is_invisible;

/// Longest question, in characters, after trimming.
pub const MAX_QUESTION_CHARS: usize = 500;
/// Longest scope text kept, in characters (the grammar allows no more).
pub const MAX_SCOPE_CHARS: usize = 120;
/// Most rows a question may ask for.
pub const MAX_LIMIT: u8 = 10;

/// What the question asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    /// The biggest folders.
    Largest,
    /// What can be cleaned.
    Cleanable,
    /// What the model labelled.
    Labelled,
    /// Why a folder is big.
    Explain,
    /// Where a folder with a name is.
    Find,
    /// About the disk, but unclear: an overview.
    General,
    /// Not about the scan.
    Offtopic,
}

impl Intent {
    pub const ALL: [Intent; 7] = [
        Intent::Largest,
        Intent::Cleanable,
        Intent::Labelled,
        Intent::Explain,
        Intent::Find,
        Intent::General,
        Intent::Offtopic,
    ];

    /// The word the grammar writes.
    pub fn as_str(self) -> &'static str {
        match self {
            Intent::Largest => "largest",
            Intent::Cleanable => "cleanable",
            Intent::Labelled => "labelled",
            Intent::Explain => "explain",
            Intent::Find => "find",
            Intent::General => "general",
            Intent::Offtopic => "offtopic",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|intent| intent.as_str() == word)
    }
}

/// Which family of folders the question is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Any,
    Cache,
    Build,
    Media,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Any, Kind::Cache, Kind::Build, Kind::Media];

    /// The word the grammar writes.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Any => "any",
            Kind::Cache => "cache",
            Kind::Build => "build",
            Kind::Media => "media",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == word)
    }
}

/// The question as a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanQuestion {
    pub intent: Intent,
    /// A folder (`~/Projekty`, `/var`, `Downloads`) or, for `find`, the name looked for. Empty: the
    /// whole scan. At most 120 plain characters.
    pub scope: String,
    pub kind: Kind,
    /// 1 to 10.
    pub limit: u8,
}

impl ScanQuestion {
    /// The question the backend falls back to when the model's answer cannot be read: an overview.
    pub fn general() -> Self {
        Self {
            intent: Intent::General,
            scope: String::new(),
            kind: Kind::Any,
            limit: 5,
        }
    }
}

/// Why a question cannot be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuestionError {
    #[error("The question is empty.")]
    Empty,
    #[error("The question is longer than {MAX_QUESTION_CHARS} characters.")]
    TooLong,
}

/// The question as it goes to the model: trimmed, 1 to 500 characters, with every control and invisible
/// character turned into a space and runs of white space made one.
pub fn clean_question(raw: &str) -> Result<String, QuestionError> {
    let trimmed = raw.trim();
    if trimmed.chars().count() > MAX_QUESTION_CHARS {
        return Err(QuestionError::TooLong);
    }

    let spaced: String = trimmed
        .chars()
        .map(|c| if is_invisible(c) { ' ' } else { c })
        .collect();
    let cleaned = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return Err(QuestionError::Empty);
    }

    Ok(cleaned)
}

/// Cleans a scope the model wrote: no invisible characters, trimmed, at most 120 characters.
pub(crate) fn clean_scope(scope: &str) -> String {
    let visible: String = scope.chars().filter(|c| !is_invisible(*c)).collect();
    let cut: String = visible.trim().chars().take(MAX_SCOPE_CHARS).collect();

    cut.trim_end().to_owned()
}

/// Words that name no folder, so a scope that is only one of them is no scope.
const STOP_WORDS: [&str; 52] = [
    "a", "all", "an", "and", "any", "are", "as", "at", "big", "can", "do", "does", "file", "files", "folder",
    "folders", "for", "from", "how", "i", "if", "in", "is", "it", "me", "most", "much", "my", "of", "on",
    "or", "show", "space", "take", "takes", "that", "the", "them", "there", "these", "this", "those", "to",
    "up", "us", "what", "where", "which", "why", "with", "you", "your",
];

/// Keeps what the model read only as far as the question backs it.
///
/// - The scope stays only when the question contains it as whole words (ignoring case, a leading `~/` and a
///   trailing `/`): at least two characters, and not a stop word such as `is` or `the`. A longer text
///   containing it ("Downloads" in "my downloads folder") counts, a piece of a word ("an" in "clean") does
///   not. `~` stays when the question says `home` or `~`, `/` when it contains a `/`. Otherwise the scope is
///   empty: the whole scan.
/// - The kind only means something for `largest`, `cleanable` and `find` (a find of a kind is a largest);
///   any other intent gets `any`.
pub fn ground(question: &str, asked: ScanQuestion) -> ScanQuestion {
    let kind = match asked.intent {
        Intent::Largest | Intent::Cleanable | Intent::Find => asked.kind,
        _ => Kind::Any,
    };
    let scope = if scope_is_backed(question, &asked.scope) {
        asked.scope
    } else {
        String::new()
    };

    ScanQuestion { scope, kind, ..asked }
}

fn scope_is_backed(question: &str, scope: &str) -> bool {
    let scope = scope.trim();
    if scope.is_empty() {
        return false;
    }

    let question = question.to_lowercase();
    if scope == "/" {
        return question.contains('/');
    }

    let bare = scope.strip_prefix("~/").unwrap_or(scope).trim_end_matches('/');
    if bare.is_empty() || bare == "~" {
        return question.contains("home") || question.contains('~');
    }

    let bare = bare.to_lowercase();
    if bare.chars().count() < 2 || STOP_WORDS.contains(&bare.as_str()) {
        return false;
    }

    contains_whole(&question, &bare)
}

/// Whether `needle` is in `text` and neither begins nor ends in the middle of a word.
fn contains_whole(text: &str, needle: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';

    text.match_indices(needle).any(|(start, found)| {
        let before = text[..start].chars().next_back();
        let after = text[start + found.len()..].chars().next();

        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(intent: Intent, scope: &str, kind: Kind) -> ScanQuestion {
        ScanQuestion {
            intent,
            scope: scope.to_owned(),
            kind,
            limit: 5,
        }
    }

    #[test]
    fn a_question_is_trimmed_and_cleaned() {
        assert_eq!(
            clean_question("  big   folders\n in ~/Projekty \t").unwrap(),
            "big folders in ~/Projekty"
        );
        assert_eq!(clean_question("a\u{202E}b\u{0}c").unwrap(), "a b c");
    }

    #[test]
    fn a_blank_or_long_question_is_refused() {
        assert_eq!(clean_question(""), Err(QuestionError::Empty));
        assert_eq!(clean_question(" \n\t "), Err(QuestionError::Empty));
        assert_eq!(clean_question("\u{200B}\u{202E}"), Err(QuestionError::Empty));
        assert_eq!(clean_question(&"x".repeat(501)), Err(QuestionError::TooLong));
        assert_eq!(
            clean_question(&format!("  {}  ", "x".repeat(500))).unwrap().len(),
            500
        );
        assert_eq!(clean_question(&"ž".repeat(500)).unwrap().chars().count(), 500);
    }

    #[test]
    fn a_scope_the_question_contains_stays() {
        let q = "What is biggest in ~/Projekty/app?";
        assert_eq!(
            ground(q, asked(Intent::Largest, "~/Projekty/app", Kind::Any)).scope,
            "~/Projekty/app"
        );
        assert_eq!(
            ground(q, asked(Intent::Largest, "Projekty/app/", Kind::Any)).scope,
            "Projekty/app/"
        );
        assert_eq!(
            ground(
                "why is DOWNLOADS big",
                asked(Intent::Explain, "downloads", Kind::Any)
            )
            .scope,
            "downloads"
        );
        assert_eq!(
            ground(
                "where is node_modules?",
                asked(Intent::Find, "node_modules", Kind::Any)
            )
            .scope,
            "node_modules"
        );
    }

    #[test]
    fn a_scope_the_model_made_up_is_dropped() {
        let q = "what takes the most space?";
        assert_eq!(
            ground(q, asked(Intent::Largest, "~/Documents", Kind::Any)).scope,
            ""
        );
        assert_eq!(
            ground(q, asked(Intent::Largest, "/etc/shadow", Kind::Any)).scope,
            ""
        );
        assert_eq!(ground(q, asked(Intent::Largest, "", Kind::Any)).scope, "");
        assert_eq!(ground(q, asked(Intent::Largest, "   ", Kind::Any)).scope, "");
    }

    #[test]
    fn a_scope_must_be_in_the_question_as_whole_words() {
        // A piece of a word is no scope: "an" is in "clean", "a" in "what", "is" in "this".
        for scope in ["an", "a", "is", "p", "dow", "Download"] {
            assert_eq!(
                ground(
                    "what can I clean in this Downloads folder?",
                    asked(Intent::Cleanable, scope, Kind::Any)
                )
                .scope,
                "",
                "{scope}"
            );
        }
        // A stop word is no scope even where it stands as a word.
        for scope in ["the", "is", "my", "files"] {
            assert_eq!(
                ground(
                    "what is the biggest of my files?",
                    asked(Intent::Largest, scope, Kind::Any)
                )
                .scope,
                "",
                "{scope}"
            );
        }

        // Whole words and phrases stay, with the punctuation around them.
        let q = "why is Program Files so big? and what about node_modules, .cache or ~/Projekty/app/";
        for scope in ["Program Files", "node_modules", ".cache", "~/Projekty/app", "big"] {
            let kept = ground(q, asked(Intent::Explain, scope, Kind::Any)).scope;

            assert_eq!(kept, if scope == "big" { "" } else { scope }, "{scope}");
        }
        assert_eq!(
            ground("what is in go?", asked(Intent::Explain, "go", Kind::Any)).scope,
            "go",
            "two characters are enough"
        );
        assert_eq!(
            ground(
                "node_modules2 is big",
                asked(Intent::Find, "node_modules", Kind::Any)
            )
            .scope,
            "",
            "a word that goes on is another word"
        );
    }

    #[test]
    fn the_home_and_the_root_need_their_word() {
        assert_eq!(
            ground(
                "biggest things in my home",
                asked(Intent::Largest, "~", Kind::Any)
            )
            .scope,
            "~"
        );
        assert_eq!(
            ground("biggest things in ~", asked(Intent::Largest, "~/", Kind::Any)).scope,
            "~/"
        );
        assert_eq!(
            ground("biggest things", asked(Intent::Largest, "~", Kind::Any)).scope,
            ""
        );
        assert_eq!(
            ground("delete everything in /", asked(Intent::Cleanable, "/", Kind::Any)).scope,
            "/"
        );
        assert_eq!(
            ground("delete everything", asked(Intent::Cleanable, "/", Kind::Any)).scope,
            ""
        );
    }

    #[test]
    fn the_kind_counts_only_for_largest_and_cleanable() {
        for intent in [
            Intent::Labelled,
            Intent::Explain,
            Intent::General,
            Intent::Offtopic,
        ] {
            assert_eq!(
                ground("q", asked(intent, "", Kind::Cache)).kind,
                Kind::Any,
                "{intent:?}"
            );
        }
        assert_eq!(
            ground("q", asked(Intent::Largest, "", Kind::Cache)).kind,
            Kind::Cache
        );
        assert_eq!(
            ground("q", asked(Intent::Cleanable, "", Kind::Media)).kind,
            Kind::Media
        );
    }

    #[test]
    fn the_words_round_trip() {
        for intent in Intent::ALL {
            assert_eq!(Intent::from_word(intent.as_str()), Some(intent));
        }
        for kind in Kind::ALL {
            assert_eq!(Kind::from_word(kind.as_str()), Some(kind));
        }
        assert_eq!(Intent::from_word("Largest"), None);
        assert_eq!(Kind::from_word("safe"), None);
    }
}
