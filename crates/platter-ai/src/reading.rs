//! What the words of a question already say, without any model.
//!
//! The model is never trusted with what the words decide. [`signals`] reads a question (English or Polish,
//! letters folded to plain ASCII, so `usunąć` and `usunac` are one word) into the parts a program can be
//! sure of: whether it is about a disk at all, a clear intent word (`biggest`, `delete`, `why`, `where`),
//! a family of folders (`cache`, `build`), a media category (`videos`, `zdjęcia`), a path the user typed, a
//! personal folder by its name (`Downloads`, `Pobrane`), a number of rows, and whether the question is Polish.
//! The rest (which folder a bare name means, what the rules call a tool) needs the scan and the rules and is
//! done by the backend.
//!
//! # Precedence
//!
//! 1. The words win over the model: a clear intent word, a kind word, a media word, a typed path, a
//!    personal folder or a tool the rules know, each of them replaces what the model read.
//! 2. The model fills only what the words leave open: the intent when no intent word is there, a folder name
//!    that no rule, no path and no personal folder explains, the kind when no word and no subject say it, the
//!    limit when no number is there.
//! 3. A question with no word about disks, files, folders or cleaning and no path is off topic, whatever the
//!    model says.
//! 4. Intent words are looked at in this order: why, label, where, delete, size. The first one found is the
//!    intent (`why is ~/.cache so big` is a why, not a size).

use crate::question::{Intent, Kind};

/// A category of media a question can be about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Video,
    Image,
    Audio,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Video => "video",
            MediaKind::Image => "image",
            MediaKind::Audio => "audio",
        }
    }
}

/// A personal folder with a well-known role, whatever it is called on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WellKnown {
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Public,
    Templates,
    Videos,
}

impl WellKnown {
    pub const ALL: [WellKnown; 8] = [
        WellKnown::Desktop,
        WellKnown::Documents,
        WellKnown::Downloads,
        WellKnown::Music,
        WellKnown::Pictures,
        WellKnown::Public,
        WellKnown::Templates,
        WellKnown::Videos,
    ];

    /// The folder's usual English name below the home directory.
    pub fn english(self) -> &'static str {
        match self {
            WellKnown::Desktop => "Desktop",
            WellKnown::Documents => "Documents",
            WellKnown::Downloads => "Downloads",
            WellKnown::Music => "Music",
            WellKnown::Pictures => "Pictures",
            WellKnown::Public => "Public",
            WellKnown::Templates => "Templates",
            WellKnown::Videos => "Videos",
        }
    }

    /// The role a folder name has, in English or in Polish, whatever the case and the accents (`pobrane`,
    /// `Downloads`, `Obrazy`).
    pub fn from_name(name: &str) -> Option<Self> {
        match fold(name).as_str() {
            "desktop" | "pulpit" => Some(WellKnown::Desktop),
            "documents" | "dokumenty" => Some(WellKnown::Documents),
            "downloads" | "pobrane" => Some(WellKnown::Downloads),
            "music" | "muzyka" => Some(WellKnown::Music),
            "pictures" | "obrazy" => Some(WellKnown::Pictures),
            "public" | "publiczny" => Some(WellKnown::Public),
            "templates" | "szablony" => Some(WellKnown::Templates),
            "videos" | "wideo" => Some(WellKnown::Videos),
            // macOS names the videos folder Movies. Elsewhere a Movies folder is just a folder.
            "movies" if cfg!(target_os = "macos") => Some(WellKnown::Videos),
            _ => None,
        }
    }
}

/// What the words of a question say by themselves. See the module's documentation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signals {
    /// The question has a word about disks, folders, files, sizes or cleaning, or a path.
    pub about_disk: bool,
    /// A clear intent word.
    pub intent: Option<Intent>,
    /// `cache` or `build`, by a word. Media is [`Signals::media`].
    pub kind: Option<Kind>,
    /// A media category, when the question asks about media and not about a folder with that name.
    pub media: Option<MediaKind>,
    /// Personal folders named by their English or Polish names (never those read as media).
    pub roles: Vec<WellKnown>,
    /// A path the user typed (`~/Projekty`, `/var`, the home as `~`), as typed.
    pub path: Option<String>,
    /// A hidden folder named by itself (`.cache`, `.local`).
    pub hidden: Option<String>,
    /// A number of rows asked for (`top 5`, `3 biggest`), 1 to 10.
    pub limit: Option<u8>,
    /// The question is Polish.
    pub polish: bool,
    /// `everything` / `wszystko` with a word of deleting.
    pub everything: bool,
    /// The folded words of the question.
    pub tokens: Vec<String>,
}

/// `text` in lower case with the Polish letters written without their marks.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'ą' => 'a',
            'ć' => 'c',
            'ę' => 'e',
            'ł' => 'l',
            'ń' => 'n',
            'ó' => 'o',
            'ś' => 's',
            'ź' | 'ż' => 'z',
            other => other,
        })
        .collect()
}

/// The folded words of a question: letters and digits, everything else separates.
pub fn words_of_question(question: &str) -> Vec<String> {
    let mut words: Vec<String> = fold(question)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();

    for word in &mut words {
        if let Some(repaired) = repaired(word) {
            *word = repaired.to_owned();
        }
    }

    words
}

/// Words a typo is read as: one letter wrong, missing, extra or swapped (six letters or more, so a short
/// word is never taken for another).
const REPAIRABLE: &[&str] = &[
    "delete",
    "biggest",
    "largest",
    "docker",
    "storage",
    "directory",
    "dlaczego",
    "usunac",
    "wyczysc",
    "najwieksze",
    "najwiekszy",
    "cleanup",
];

fn repaired(word: &str) -> Option<&'static str> {
    if word.chars().count() < 5 || REPAIRABLE.contains(&word) {
        return None;
    }

    REPAIRABLE
        .iter()
        .find(|known| within_one_edit(word, known))
        .copied()
}

/// Whether `a` and `b` differ by one letter put in, left out, changed or swapped with its neighbour.
fn within_one_edit(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }

    let mut at = 0;
    while at < a.len() && at < b.len() && a[at] == b[at] {
        at += 1;
    }
    if at == a.len() && at == b.len() {
        return true;
    }

    let rest = |x: &[char], y: &[char]| x[1..] == y[1..];
    let (ra, rb) = (&a[at..], &b[at..]);
    match a.len().cmp(&b.len()) {
        std::cmp::Ordering::Equal => {
            rest(ra, rb) || (ra.len() >= 2 && ra[0] == rb[1] && ra[1] == rb[0] && ra[2..] == rb[2..])
        }
        std::cmp::Ordering::Less => ra == &rb[1..],
        std::cmp::Ordering::Greater => &ra[1..] == rb,
    }
}

fn has_stem(tokens: &[String], stems: &[&str]) -> bool {
    tokens
        .iter()
        .any(|token| stems.iter().any(|stem| token.starts_with(stem)))
}

fn has_word(tokens: &[String], words: &[&str]) -> bool {
    tokens.iter().any(|token| words.contains(&token.as_str()))
}

fn has_sequence(tokens: &[String], sequence: &[&str]) -> bool {
    tokens
        .windows(sequence.len())
        .any(|window| window.iter().zip(sequence).all(|(token, word)| token == word))
}

/// Words that make a question one about a disk. A stem matches the words that begin with it.
const DISK_STEMS: &[&str] = &[
    "disk", "drive", "dysk", "space", "storag", "folder", "director", "katalog", "file", "plik", "size",
    "rozmiar", "big", "larg", "najwiek", "wielk", "cach", "docker", "steam", "trash", "kosz", "junk",
    "clean", "delet", "remov", "wipe", "purg", "empt", "usun", "wyczys", "skasow", "skasuj", "label", "scan",
    "miejsc", "zajmuj", "takes", "taking", "usage", "eating", "hog", "overview", "summary", "podsumow",
    "pamiec", "smiec", "node", "build", "etykiet", "oznacz", "gigabyte", "megabyte", "terabyte", "heav",
];
const DISK_WORDS: &[&str] = &["gb", "mb", "tb", "kb"];

const EXPLAIN_STEMS: &[&str] = &["dlaczego", "czemu", "wyjasni"];
const FIND_WORDS: &[&str] = &[
    "where",
    "gdzie",
    "locate",
    "find",
    "znajdz",
    "lokalizacja",
    "lokalizacje",
];
const CLEAN_STEMS: &[&str] = &[
    "delet",
    "remov",
    "clean",
    "wipe",
    "purg",
    "empt",
    "erase",
    "discard",
    "uninstall",
    "usun",
    "wyczys",
    "skasow",
    "skasuj",
    "kasuj",
    "oprozn",
    "safe",
    "bezpiecz",
    "junk",
    "smiec",
    "zwolni",
    "czyst",
    "wyrzu",
    "pozb",
];
const SIZE_STEMS: &[&str] = &[
    "big", "larg", "najwiek", "zajmuj", "miejsc", "heav", "hog", "eating", "consum", "usage", "rozmiar",
    "wielk",
];
const SIZE_WORDS: &[&str] = &[
    "most", "take", "takes", "taking", "use", "uses", "using", "space", "much", "size", "ile", "top",
];

fn intent_of(tokens: &[String]) -> Option<Intent> {
    let explains = has_word(tokens, &["why", "explain"])
        || has_stem(tokens, EXPLAIN_STEMS)
        || has_sequence(tokens, &["what", "s", "in"])
        || has_sequence(tokens, &["what", "is", "in"])
        || has_sequence(tokens, &["whats", "in"])
        || has_sequence(tokens, &["what", "is", "inside"])
        || has_sequence(tokens, &["what", "s", "inside"])
        || has_sequence(tokens, &["co", "jest", "w"])
        || has_sequence(tokens, &["co", "siedzi"])
        || has_sequence(tokens, &["z", "czego"]);
    if explains {
        return Some(Intent::Explain);
    }

    if has_stem(tokens, &["label", "etykiet", "oznacz"]) {
        return Some(Intent::Labelled);
    }
    if has_word(tokens, FIND_WORDS) {
        return Some(Intent::Find);
    }
    // "free" alone is not deleting ("free space"): "free up" and "to free" are.
    let frees = has_sequence(tokens, &["free", "up"])
        || has_sequence(tokens, &["to", "free"])
        || has_sequence(tokens, &["get", "rid"])
        || has_sequence(tokens, &["throw", "away"]);
    if has_stem(tokens, CLEAN_STEMS) || frees {
        return Some(Intent::Cleanable);
    }
    if has_stem(tokens, SIZE_STEMS) || has_word(tokens, SIZE_WORDS) {
        return Some(Intent::Largest);
    }
    if has_stem(tokens, &["overview", "summary", "podsumow", "przeglad"]) {
        return Some(Intent::General);
    }

    None
}

const MEDIA_ONLY: &[(&str, MediaKind)] = &[
    ("movie", MediaKind::Video),
    ("movies", MediaKind::Video),
    ("film", MediaKind::Video),
    ("films", MediaKind::Video),
    ("filmy", MediaKind::Video),
    ("filmow", MediaKind::Video),
    ("filmach", MediaKind::Video),
    ("photo", MediaKind::Image),
    ("photos", MediaKind::Image),
    ("zdjecia", MediaKind::Image),
    ("zdjec", MediaKind::Image),
    ("zdjeciach", MediaKind::Image),
    ("fotki", MediaKind::Image),
    ("fotografie", MediaKind::Image),
    ("song", MediaKind::Audio),
    ("songs", MediaKind::Audio),
    ("piosenki", MediaKind::Audio),
    ("audio", MediaKind::Audio),
    ("mp3", MediaKind::Audio),
    ("clips", MediaKind::Video),
];

/// A folder name that is also a media category (`videos`).
const MEDIA_OR_FOLDER: &[(&str, MediaKind, WellKnown)] = &[
    ("videos", MediaKind::Video, WellKnown::Videos),
    ("wideo", MediaKind::Video, WellKnown::Videos),
    ("pictures", MediaKind::Image, WellKnown::Pictures),
    ("obrazy", MediaKind::Image, WellKnown::Pictures),
    ("music", MediaKind::Audio, WellKnown::Music),
    ("muzyka", MediaKind::Audio, WellKnown::Music),
];

const FOLDER_CUES: &[&str] = &[
    "folder",
    "folders",
    "folderze",
    "directory",
    "dir",
    "katalog",
    "katalogu",
    "foldery",
];

/// The words that name one personal folder and no media category.
fn role_of(token: &str) -> Option<WellKnown> {
    match token {
        "desktop" | "pulpit" | "pulpicie" => Some(WellKnown::Desktop),
        "documents" | "dokumenty" | "dokumentach" => Some(WellKnown::Documents),
        "downloads" | "download" | "pobrane" | "pobranych" => Some(WellKnown::Downloads),
        "templates" | "szablony" => Some(WellKnown::Templates),
        "publiczny" | "publiczne" => Some(WellKnown::Public),
        _ => None,
    }
}

/// A word that is a hidden folder by itself (`.cache`), with the punctuation around it cut off.
fn hidden_token(raw: &str) -> Option<String> {
    let token = trimmed(raw);

    (token.starts_with('.') && token.chars().nth(1).is_some_and(char::is_alphabetic) && !token.contains('/'))
        .then(|| token.to_owned())
}

fn trimmed(raw: &str) -> &str {
    raw.trim_matches(|c: char| {
        matches!(
            c,
            '?' | ',' | ';' | ':' | '!' | '"' | '\'' | '(' | ')' | '“' | '”'
        )
    })
    .trim_end_matches('.')
}

/// A word that is a path (`~/Projekty`, `/var`, `~`).
fn path_token(raw: &str) -> Option<String> {
    let token = trimmed(raw);

    if token == "~" || token.starts_with("~/") {
        return Some(token.to_owned());
    }
    let rooted = token.starts_with('/')
        && token
            .chars()
            .nth(1)
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if rooted || token == "/" {
        return Some(token.to_owned());
    }

    None
}

/// A path in straight double quotes (`"~/My Files/a.txt"`), so that white space can be part of it: the
/// path with the quotes cut off, and the question without the quoted text. A quote opens only at the start
/// of the text or after white space, and closes at the next quote, the same rule the question box follows
/// when it completes a path. A quote in the middle of a word (`5" wide`) opens nothing, and the characters
/// after a closing quote belong to its word. Only the first quoted text that is a path counts, and a quote
/// that is never closed quotes nothing.
fn quoted_path(question: &str) -> Option<(String, String)> {
    let mut from = 0;

    while from < question.len() {
        let rest = &question[from..];
        let word = rest.trim_start();
        let open = from + (rest.len() - word.len());

        if word.is_empty() {
            return None;
        }

        // The word that starts here ends at white space, or at its closing quote when it opens with one.
        let mut end = open;
        if word.starts_with('"') {
            // Without a closing quote there is no quoted text.
            let close = open + 1 + question[open + 1..].find('"')?;
            let text = question[open + 1..close].trim();

            if text == "~" || text.starts_with("~/") || (text.starts_with('/') && text.len() > 1) {
                let rest = format!("{} {}", &question[..open], &question[close + 1..]);

                return Some((text.to_owned(), rest));
            }
            end = close + 1;
        }

        let tail = &question[end..];
        from = end + tail.find(char::is_whitespace).unwrap_or(tail.len());
    }

    None
}

/// A word that reads as the name of a file: a dot inside it, not at its start, with 1 to 5 letters or digits
/// after the last dot to its end (`old.iso`, `notes.txt`), the punctuation around it cut off. A version
/// (`v1.2`) reads like that too, which is no harm: it only makes the question one about the disk, and the
/// model still says when it is not.
fn file_name_token(raw: &str) -> bool {
    let token = trimmed(raw);
    let Some((stem, extension)) = token.rsplit_once('.') else {
        return false;
    };

    !stem.is_empty()
        && !token.contains('/')
        && (1..=5).contains(&extension.chars().count())
        && extension.chars().all(char::is_alphanumeric)
}

fn limit_of(tokens: &[String]) -> Option<u8> {
    let limit_words = [
        "top",
        "biggest",
        "largest",
        "folders",
        "folder",
        "caches",
        "items",
        "najwiekszych",
        "foldery",
        "najwieksze",
        "pierwszych",
    ];

    tokens.iter().enumerate().find_map(|(at, token)| {
        let number: u8 = token.parse().ok().filter(|number| (1..=10).contains(number))?;
        let next = tokens.get(at + 1).map(String::as_str);
        let before = at
            .checked_sub(1)
            .and_then(|before| tokens.get(before))
            .map(String::as_str);

        (next.is_some_and(|word| limit_words.contains(&word)) || before == Some("top")).then_some(number)
    })
}

const POLISH_WORDS: &[&str] = &[
    "czy",
    "co",
    "ile",
    "jak",
    "jakie",
    "jaki",
    "gdzie",
    "dlaczego",
    "czemu",
    "ktore",
    "mozna",
    "moge",
    "mam",
    "moje",
    "moj",
    "mojego",
    "miejsca",
    "dysku",
    "zajmuje",
    "zajmuja",
    "usunac",
    "usun",
    "wyczysc",
    "najwieksze",
    "najwiekszy",
    "foldery",
    "pliki",
    "pokaz",
    "kosz",
    "wszystko",
];

/// Reads what the words of `question` say by themselves.
pub fn signals(question: &str) -> Signals {
    // A quoted path is one path, whatever white space is in it, and is no words of the question.
    let (quoted, unquoted) = match quoted_path(question) {
        Some((path, rest)) => (Some(path), rest),
        None => (None, question.to_owned()),
    };
    let raw_words: Vec<&str> = unquoted.split_whitespace().collect();
    // A typed path is a path, not words: `/var/cache` is no word about caches.
    let spoken: Vec<&str> = raw_words
        .iter()
        .copied()
        .filter(|word| path_token(word).is_none() && hidden_token(word).is_none())
        .collect();
    let tokens = words_of_question(&spoken.join(" "));

    let path = quoted
        .or_else(|| raw_words.iter().find_map(|word| path_token(word)))
        .or_else(|| home_word(&tokens).then(|| "~".to_owned()));
    let hidden = raw_words.iter().find_map(|word| hidden_token(word));
    let intent = intent_of(&tokens);
    let kind = if has_stem(&tokens, &["cach"])
        || has_sequence(&tokens, &["pamiec", "podreczna"])
        || has_sequence(&tokens, &["pamieci", "podrecznej"])
    {
        Some(Kind::Cache)
    } else if has_stem(&tokens, &["build", "dependenc", "kompilac", "artifact"]) {
        Some(Kind::Build)
    } else {
        None
    };

    let mut roles: Vec<WellKnown> = Vec::new();
    for token in &tokens {
        if let Some(role) = role_of(token)
            && !roles.contains(&role)
        {
            roles.push(role);
        }
    }

    let mut media = None;
    if path.is_none() {
        media = MEDIA_ONLY
            .iter()
            .find(|(word, _)| tokens.iter().any(|token| token == word))
            .map(|(_, kind)| *kind);
    }
    for &(word, category, role) in MEDIA_OR_FOLDER {
        let Some(at) = tokens.iter().position(|token| token == word) else {
            continue;
        };
        let after = tokens.get(at + 1).map(String::as_str);
        let before = at
            .checked_sub(1)
            .and_then(|before| tokens.get(before))
            .map(String::as_str);
        let names_a_folder = after.is_some_and(|next| FOLDER_CUES.contains(&next))
            || matches!(before, Some("in" | "w" | "we" | "inside" | "under"))
            || path.is_some()
            || intent != Some(Intent::Largest);

        if names_a_folder {
            if !roles.contains(&role) {
                roles.push(role);
            }
        } else if media.is_none() {
            media = Some(category);
        }
    }

    let about_disk = path.is_some()
        || hidden.is_some()
        || raw_words.iter().any(|word| file_name_token(word))
        || media.is_some()
        || !roles.is_empty()
        || has_stem(&tokens, DISK_STEMS)
        || has_word(&tokens, DISK_WORDS);
    let everything = has_word(&tokens, &["everything", "wszystko"]) && intent == Some(Intent::Cleanable);
    let polish =
        question.chars().any(|c| "ąćęłńóśźżĄĆĘŁŃÓŚŹŻ".contains(c)) || has_word(&tokens, POLISH_WORDS);

    Signals {
        about_disk,
        intent,
        kind,
        media,
        roles,
        path,
        hidden,
        limit: limit_of(&tokens),
        polish,
        everything,
        tokens,
    }
}

/// `my home folder`, `home directory`, `katalog domowy`: the home folder, said in words.
fn home_word(tokens: &[String]) -> bool {
    tokens.iter().enumerate().any(|(at, token)| {
        let next = tokens.get(at + 1).map(String::as_str);
        let before = at
            .checked_sub(1)
            .and_then(|before| tokens.get(before))
            .map(String::as_str);

        match token.as_str() {
            "home" => {
                matches!(next, Some("folder" | "directory" | "dir" | "drive"))
                    || matches!(before, Some("my" | "in" | "moj" | "mojego"))
            }
            "domowy" | "domowym" | "domowego" => matches!(before, Some("katalog" | "katalogu" | "folder")),
            _ => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(question: &str) -> Signals {
        signals(question)
    }

    #[test]
    fn a_quoted_path_keeps_the_punctuation_at_its_end() {
        // Bare, the full stop and the bracket are trimmed off as sentence punctuation.
        assert_eq!(
            read("is ~/Downloads/report. big").path.as_deref(),
            Some("~/Downloads/report")
        );

        // In quotes, they are part of the name.
        assert_eq!(
            read(r#"is "~/Downloads/report." big"#).path.as_deref(),
            Some("~/Downloads/report.")
        );
        assert_eq!(read(r#"is "~/x/foo)" big?"#).path.as_deref(), Some("~/x/foo)"));
    }

    #[test]
    fn a_path_in_straight_quotes_is_one_path_with_its_white_space() {
        let signals = read(r#"how big is "~/My Files/a.txt"?"#);

        assert_eq!(signals.path.as_deref(), Some("~/My Files/a.txt"));
        assert_eq!(signals.tokens, ["how", "big", "is"]);
        assert!(signals.about_disk);

        assert_eq!(
            read(r#"why is "/srv/my data" so big"#).path.as_deref(),
            Some("/srv/my data")
        );
        assert_eq!(read(r#"what is in "~""#).path.as_deref(), Some("~"));
    }

    #[test]
    fn a_quote_in_the_middle_of_a_word_opens_no_quoted_text() {
        let signals = read(r#"it's 5" wide, how big is "~/x/report.""#);

        assert_eq!(signals.path.as_deref(), Some("~/x/report."));
        assert_eq!(signals.tokens, ["it", "s", "5", "wide", "how", "big", "is"]);

        assert_eq!(
            read(r#"is say" big "~/x/foo)""#).path.as_deref(),
            Some("~/x/foo)")
        );
    }

    #[test]
    fn a_quoted_word_ends_at_its_closing_quote() {
        // The quote that closes `"a b"` is not the one that opens the path, which follows its word.
        assert_eq!(
            read(r#"is "a b"x "~/My Files/a.txt" big"#).path.as_deref(),
            Some("~/My Files/a.txt")
        );
        // A quote that follows a closing quote in the same word opens nothing.
        assert_eq!(read(r#"is "a"b"~/x y" big"#).path, None);
    }

    #[test]
    fn quotes_that_hold_no_path_are_no_path() {
        assert_eq!(read(r#"what is "node_modules" here"#).path, None);
        // An unclosed quote is no quoted text: the word is a path as it always was.
        assert_eq!(read(r#"how big is "~/open"#).path.as_deref(), Some("~/open"));
        assert_eq!(
            read(r#"how big is "~/nospace.txt""#).path.as_deref(),
            Some("~/nospace.txt")
        );
    }

    #[test]
    fn the_name_of_a_file_makes_a_question_about_the_disk() {
        let find = read("where is old.iso?");
        assert!(find.about_disk);
        assert_eq!(find.intent, Some(Intent::Find));

        assert!(read("how big is notes.txt").about_disk);
        assert!(read("is archive.tar.gz big").about_disk);
        assert!(
            read("where is v1.2").about_disk,
            "a version reads like a file name"
        );

        // No dot inside the word, a dot at its start (a hidden folder is read on its own), or a long tail.
        assert!(!read("write me a poem").about_disk);
        assert!(!read("what is the weather. tell me").about_disk);
        assert!(!read("see www.example.international").about_disk);
    }

    #[test]
    fn a_sentence_ending_in_e_g_is_on_topic_only_for_the_model_to_turn_down() {
        let signals = read("write me a poem, e.g. about spring.");

        // The words cannot tell, so the question is not decided off topic by them: the model decides.
        assert!(signals.about_disk);
        assert_eq!(signals.intent, None);
        assert_eq!(signals.path, None);
    }

    #[test]
    fn the_name_of_a_file_the_model_returns_as_the_scope_survives_grounding() {
        let asked = |scope: &str| crate::question::ScanQuestion {
            intent: Intent::Find,
            scope: scope.to_owned(),
            kind: Kind::Any,
            limit: 5,
        };

        for question in [
            "where is old.iso?",
            "find the file Old.ISO.",
            "where is \"old.iso\"",
        ] {
            assert_eq!(
                crate::question::ground(question, asked("old.iso")).scope,
                "old.iso",
                "{question}"
            );
        }
        // A name that is only the start of a longer word is no scope.
        assert_eq!(
            crate::question::ground("where is old.iso2", asked("old.iso")).scope,
            "",
        );
    }

    #[test]
    fn the_polish_marks_are_folded_and_typos_of_a_few_words_are_repaired() {
        assert_eq!(fold("Usunąć ŁÓDŹ"), "usunac lodz");
        assert_eq!(words_of_question("czy mogę usunąć?"), ["czy", "moge", "usunac"]);
        assert_eq!(words_of_question("dokcer"), ["docker"]);
        assert_eq!(words_of_question("biggset foldres"), ["biggest", "foldres"]);
        assert_eq!(
            words_of_question("steak"),
            ["steak"],
            "a short word is never repaired"
        );
    }

    #[test]
    fn a_clear_intent_word_decides_the_intent_in_a_fixed_order() {
        let intent = |question: &str| read(question).intent;

        assert_eq!(intent("why is ~/.cache so big?"), Some(Intent::Explain));
        assert_eq!(intent("what's in ~/.cache?"), Some(Intent::Explain));
        assert_eq!(intent("dlaczego to jest takie duże"), Some(Intent::Explain));
        assert_eq!(intent("what did the model label?"), Some(Intent::Labelled));
        assert_eq!(intent("where is docker?"), Some(Intent::Find));
        assert_eq!(intent("gdzie jest docker"), Some(Intent::Find));
        assert_eq!(
            intent("can i safely delete the docker images"),
            Some(Intent::Cleanable)
        );
        assert_eq!(intent("co mogę usunąć?"), Some(Intent::Cleanable));
        assert_eq!(
            intent("how much space would I free by emptying it"),
            Some(Intent::Cleanable)
        );
        assert_eq!(
            intent("which folder does take the biggest amount of data?"),
            Some(Intent::Largest)
        );
        assert_eq!(intent("co zajmuje najwięcej miejsca?"), Some(Intent::Largest));
        assert_eq!(intent("what's eating my disk"), Some(Intent::Largest));
        assert_eq!(intent("give me an overview of the disk"), Some(Intent::General));
        assert_eq!(intent("tell me something about this disk"), None);
        assert_eq!(
            intent("how much free space is left"),
            Some(Intent::Largest),
            "free space is not deleting"
        );
    }

    #[test]
    fn only_a_question_with_a_word_about_disks_or_a_path_is_about_the_disk() {
        for question in [
            "write me a poem",
            "what is the capital of France?",
            "can you tell me a joke",
            "hello",
        ] {
            assert!(!read(question).about_disk, "{question}");
        }
        for question in [
            "what's eating my disk",
            "ile miejsca zajmują filmy?",
            "is ~/Projekty ok?",
            "czy mogę usunąć obrazy dockera?",
            "wyczyść kosz",
            "how big is it, in GB",
            "can I delete my Downloads?",
        ] {
            assert!(read(question).about_disk, "{question}");
        }
    }

    #[test]
    fn a_typed_path_a_home_in_words_and_the_personal_folders_are_found() {
        assert_eq!(read("why is ~/.cache so big?").path.as_deref(), Some("~/.cache"));
        assert_eq!(read("biggest folders in /var").path.as_deref(), Some("/var"));
        assert_eq!(read("can I delete ~/Pobrane?").path.as_deref(), Some("~/Pobrane"));
        assert_eq!(read("what is in ~?").path.as_deref(), Some("~"));
        assert_eq!(read("why is my home folder so big").path.as_deref(), Some("~"));
        assert_eq!(read("either/or").path, None);
        assert_eq!(
            read("dlaczego .cache jest taki duży?").hidden.as_deref(),
            Some(".cache")
        );
        assert_eq!(
            read("dlaczego .cache jest taki duży?").kind,
            None,
            ".cache is a name, not a word about caches"
        );
        assert_eq!(read("it ends here...").hidden, None);
        assert_eq!(
            read("biggest in /var/cache").kind,
            None,
            "a path is no word about caches"
        );
        assert_eq!(read("what is big and/or old").path, None);

        assert_eq!(read("can I delete my Downloads?").roles, [WellKnown::Downloads]);
        assert_eq!(read("czy mogę usunąć Pobrane").roles, [WellKnown::Downloads]);
        assert_eq!(
            read("what is in my Documents folder?").roles,
            [WellKnown::Documents]
        );
        assert_eq!(read("is my Videos folder big").roles, [WellKnown::Videos]);
    }

    #[test]
    fn media_words_are_media_unless_the_question_names_a_folder() {
        let media = |question: &str| read(question).media;

        assert_eq!(media("how much do my videos take?"), Some(MediaKind::Video));
        assert_eq!(media("ile miejsca zajmują filmy?"), Some(MediaKind::Video));
        assert_eq!(media("how large are my photos?"), Some(MediaKind::Image));
        assert_eq!(media("how much does my music take up?"), Some(MediaKind::Audio));
        assert_eq!(media("ile zajmuje muzyka"), Some(MediaKind::Audio));
        // A folder with the name, not the category.
        assert_eq!(media("how much does my Videos folder take?"), None);
        assert_eq!(media("can I delete my videos?"), None);
        assert_eq!(media("biggest things in ~/Videos"), None);
        assert_eq!(media("what is in the Pictures?"), None);
        assert_eq!(read("can I delete my videos?").roles, [WellKnown::Videos]);
    }

    #[test]
    fn kind_limit_polish_and_everything_come_from_words() {
        assert_eq!(read("what are my biggest caches?").kind, Some(Kind::Cache));
        assert_eq!(read("czy mogę usunąć pamięć podręczną").kind, Some(Kind::Cache));
        assert_eq!(read("what are the biggest build folders").kind, Some(Kind::Build));
        assert_eq!(read("biggest folders").kind, None);

        assert_eq!(read("show me the 3 biggest folders").limit, Some(3));
        assert_eq!(read("top 7 caches").limit, Some(7));
        assert_eq!(read("list my 7 largest folders").limit, Some(7));
        assert_eq!(read("node 18 build folders").limit, None);
        assert_eq!(read("biggest folders").limit, None);

        assert!(read("co zajmuje najwięcej miejsca?").polish);
        assert!(read("jakie foldery są największe").polish);
        assert!(!read("what takes the most space?").polish);

        assert!(read("clean everything").everything);
        assert!(read("usuń wszystko").everything);
        assert!(!read("show everything big").everything);
    }
}
