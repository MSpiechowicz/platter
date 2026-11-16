//! The reading of a question: what the words decide, and what the model only fills in.
//!
//! [`read`] puts the words of the question (see `platter_ai::reading` for the precedence) before the model's
//! reading. It needs the rules, to know a tool by its name (`docker`, `steam`, `trash`, `node_modules`),
//! but not the scan: which folder a name is, and whether it is ambiguous, is the lens's work.

use platter_ai::{Intent, Kind, MediaKind, ScanQuestion, WellKnown, ground, signals, words_of_question};
use platter_core::Os;
use platter_rules::{Rule, RuleSet};

use super::{GENERIC_WORDS, STOP_WORDS, rules_for_words, title_words, words_of};

/// What a question names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// Nothing: the whole scan.
    None,
    /// A path the user typed (`~/Projekty`, `/var`, `~`).
    Path(String),
    /// A personal folder by its role, in English or Polish (`Downloads`, `Pobrane`), and the name used.
    Role { role: WellKnown, typed: String },
    /// A tool the rules know (`docker`, `steam`, `node_modules`): the rules it names and the words that
    /// named them.
    Rules { said: String, ids: Vec<&'static str> },
    /// Words the model read as a folder name, that no rule, path or personal folder explains.
    Name(String),
}

impl Subject {
    /// The subject written as a short symbol (`rule:docker`, `role:downloads`, `path:~/Projekty`,
    /// `name:backup`, `none`): what the evaluation set expects.
    #[cfg(test)]
    pub fn symbol(&self) -> String {
        match self {
            Subject::None => "none".to_owned(),
            Subject::Path(path) => format!("path:{path}"),
            Subject::Role { role, .. } => format!("role:{}", role.english().to_lowercase()),
            Subject::Rules { ids, .. } => {
                let family = ids.first().map_or("", |id| id.split('-').next().unwrap_or(id));

                format!("rule:{family}")
            }
            Subject::Name(name) => format!("name:{name}"),
        }
    }
}

/// What the question comes to, once the words and the model are put together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub query: ScanQuestion,
    pub subject: Subject,
    /// A media category, for the `media` kind.
    pub media: Option<MediaKind>,
    pub polish: bool,
    pub everything: bool,
    /// The words alone decided the intent and the subject, so the model was not needed.
    pub complete: bool,
}

/// The tools of the rules by the words people use for them (and Polish ones): the rule's id, and the words
/// that name it without a doubt. A word every rule has (`cache`) or one that names many things (`image`)
/// is not here.
const TOOLS: &[(&str, &[&str])] = &[
    ("apt-cache", &["apt"]),
    ("dnf-cache", &["dnf"]),
    ("rust-target", &["rust"]),
    ("rust-target-recent", &["rust"]),
    ("node-modules", &["node", "nodejs"]),
    ("node-modules-recent", &["node", "nodejs"]),
    ("python-cache", &["python", "pycache", "pip"]),
    ("npm-cache", &["npm"]),
    ("bun-cache", &["bun"]),
    ("pnpm-store", &["pnpm"]),
    ("yarn-cache", &["yarn"]),
    ("cargo-registry", &["cargo"]),
    ("gradle-cache", &["gradle"]),
    ("nuget-packages", &["nuget"]),
    ("go-modules", &["golang"]),
    ("trash", &["trash", "kosz", "recycle", "wastebasket"]),
    ("coredumps", &["coredump", "coredumps"]),
    ("steam-shader-cache", &["steam", "shader"]),
    ("steam-library", &["steam", "game"]),
    ("pacman-cache", &["pacman"]),
    ("journal", &["journal", "journald"]),
    ("maven-repository", &["maven"]),
    ("docker-data", &["docker", "container", "kontener"]),
    ("docker-desktop-vm", &["docker", "container", "kontener"]),
    ("btrfs-snapshots", &["btrfs", "snapper", "snapshot"]),
];

/// Whether a word of the question is `tool`: the same word, or (for a word of five letters or more) with a
/// short ending (`dockera`, `dockerze`, `containers`).
fn is_word(token: &str, tool: &str) -> bool {
    token == tool || (tool.len() >= 5 && token.starts_with(tool) && token.len() <= tool.len() + 3)
}

/// The rules the tools of the question name: the tools it says by name, then every rule that has them
/// among its own words (`steam library` is the Steam library and not the shader cache), with the words that
/// said it.
pub(super) fn tool_rules<'r>(
    rules: &'r RuleSet,
    os: Os,
    question: &str,
    tokens: &[String],
) -> Option<(String, Vec<&'r Rule>)> {
    let known = |id: &str| {
        rules
            .rules()
            .iter()
            .any(|rule| rule.id == id && rule.os.is_none_or(|only| only == os))
    };

    let mut strong: Vec<String> = Vec::new();
    let mut said: Vec<String> = Vec::new();
    for token in tokens {
        let stem = words_of(token).pop().unwrap_or_else(|| token.clone());

        for (id, tools) in TOOLS {
            let Some(tool) = tools.iter().find(|tool| is_word(&stem, tool)) else {
                continue;
            };
            if known(id) {
                // The tool as the rule's own words have it (`kosz` is `trash`) and as it was said.
                for word in [*tool, tools[0]] {
                    if !strong.contains(&word.to_owned()) {
                        strong.push(word.to_owned());
                    }
                }
                if !said.contains(token) {
                    said.push(token.clone());
                }
            }
        }
    }
    if strong.is_empty() {
        return None;
    }

    // The other words of the question that are words of those rules' titles narrow the choice.
    let skipped: Vec<String> = STOP_WORDS
        .iter()
        .chain(&GENERIC_WORDS)
        .flat_map(|word| words_of(word))
        .collect();
    let mut words = strong;
    for token in tokens {
        for stem in words_of(token) {
            let is_role = WellKnown::from_name(token).is_some();
            let is_title_word = rules
                .rules()
                .iter()
                .any(|rule| title_words(rule).any(|word| word == stem));

            if !is_role
                && is_title_word
                && stem.chars().count() >= 2
                && !skipped.contains(&stem)
                && !words.contains(&stem)
            {
                words.push(stem);
            }
        }
    }

    let named = rules_for_words(rules, os, &words);
    if named.is_empty() {
        return None;
    }

    // The words as they were written (`node_modules`, `dockera`), not as they were split.
    let written: Vec<&str> = question
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.'))
        .filter(|word| words_of_question(word).iter().any(|token| said.contains(token)))
        .collect();
    let said = if written.is_empty() {
        said.join(" ")
    } else {
        written.join(" ")
    };

    Some((said, named))
}

/// What the question names and means, from its words and the model's reading of it (`None`: no model, or
/// one that could not be read).
///
/// The words win: a clear intent word, a kind word, a media word, a path, a personal folder or a tool of the
/// rules replace what the model read. The model fills the intent when no word says it, a folder name that
/// nothing else explains, a number of rows, and the `media` kind when the words say nothing of the kind.
/// Without any word of disks or cleaning (and no path) the question is off topic.
pub fn read(question: &str, model: Option<ScanQuestion>, rules: &RuleSet, os: Os) -> Reading {
    let signals = signals(question);
    let model = model.map(|asked| ground(question, asked));
    let tools = tool_rules(rules, os, question, &signals.tokens);

    let on_topic = signals.about_disk || tools.is_some();
    let model_offtopic = model
        .as_ref()
        .is_some_and(|asked| asked.intent == Intent::Offtopic);
    let polish = signals.polish;

    if !on_topic || (model_offtopic && signals.intent.is_none()) {
        return Reading {
            query: ScanQuestion {
                intent: Intent::Offtopic,
                scope: String::new(),
                kind: Kind::Any,
                limit: 5,
            },
            subject: Subject::None,
            media: None,
            polish,
            everything: false,
            complete: true,
        };
    }

    let typed_role = |role: WellKnown| {
        signals
            .tokens
            .iter()
            .find(|token| WellKnown::from_name(token) == Some(role))
            .cloned()
            .unwrap_or_else(|| role.english().to_owned())
    };
    let subject = match (&signals.path, &tools, signals.roles.first()) {
        (Some(path), _, _) => Subject::Path(path.clone()),
        (None, Some((said, named)), _) => Subject::Rules {
            said: said.clone(),
            ids: named.iter().map(|rule| rule.id).collect(),
        },
        (None, None, Some(role)) => Subject::Role {
            role: *role,
            typed: typed_role(*role),
        },
        (None, None, None) if signals.hidden.is_some() => {
            Subject::Name(signals.hidden.clone().unwrap_or_default())
        }
        (None, None, None) => match model
            .as_ref()
            .map(|asked| asked.scope.as_str())
            .filter(|scope| !scope.is_empty())
        {
            Some(scope) if scope == "~" || scope.starts_with("~/") || scope.starts_with('/') => {
                Subject::Path(scope.to_owned())
            }
            Some(scope) => match WellKnown::from_name(scope.trim_end_matches('/')) {
                Some(role) => Subject::Role {
                    role,
                    typed: scope.to_owned(),
                },
                None => Subject::Name(scope.to_owned()),
            },
            None => Subject::None,
        },
    };

    let model_intent = model
        .as_ref()
        .map(|asked| asked.intent)
        .filter(|intent| *intent != Intent::Offtopic);
    let mut intent = signals.intent.or(model_intent).unwrap_or(Intent::General);
    // A kind or a media category is a question about a family of folders, never a search for a name.
    let media = signals.media.filter(|_| subject == Subject::None);
    let named_tool = matches!(subject, Subject::Rules { .. });
    let kind = if media.is_some() {
        Kind::Media
    } else if named_tool {
        Kind::Any
    } else if let Some(kind) = signals.kind {
        kind
    } else if model.as_ref().is_some_and(|asked| asked.kind == Kind::Media)
        && subject == Subject::None
        && intent == Intent::Largest
    {
        Kind::Media
    } else {
        Kind::Any
    };
    if intent == Intent::Find && subject == Subject::None && kind != Kind::Any {
        intent = Intent::Largest;
    }

    let limit = signals
        .limit
        .or(model.as_ref().map(|asked| asked.limit))
        .unwrap_or(5);
    let scope = match &subject {
        Subject::None => String::new(),
        Subject::Path(path) => path.clone(),
        Subject::Role { role, .. } => role.english().to_owned(),
        Subject::Rules { said, .. } => said.clone(),
        Subject::Name(name) => name.clone(),
    };
    let complete = signals.intent.is_some() && !matches!(subject, Subject::None | Subject::Name(_));

    Reading {
        query: ScanQuestion {
            intent,
            scope,
            kind,
            limit,
        },
        subject,
        media,
        polish,
        everything: signals.everything,
        complete,
    }
}
