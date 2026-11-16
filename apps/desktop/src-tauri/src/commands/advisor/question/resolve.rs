//! Where the subject of a question is in the finished scan, or whether it is clear at all.
//!
//! The subject is looked up in the tree and nowhere else, in this order:
//!
//! 1. A typed path is that exact folder.
//! 2. A tool the rules know (`docker`) is the folders its rules matched in this scan; where the rules have
//!    one place that the scan does not hold, the answer says where it is.
//! 3. A personal folder by its role (`Downloads`, `Pobrane`) is the user's real one (XDG user-dirs): a folder
//!    with the same name somewhere else is never "your Downloads".
//! 4. A bare name is `~/<name>`, else the folders with that name.
//!
//! A subject is never guessed. When two unrelated tools, or two folders of a similar standing, fit the
//! words, the question is asked back ([`Clarify`]); a later `ask_scan` with the id of a choice resolves the
//! same candidates again and takes that one. A subject that is not in the scan is [`Scope::Missing`], and
//! the answer says only that.

use std::path::{Path, PathBuf};

use platter_ai::WellKnown;
use platter_core::query::{FolderMatch, MAX_FOLDER_MATCHES, find_folders};
use platter_core::{NodeId, UserDir};

use super::reading::Subject;
use super::{Absent, ByRule, Lens, Scope, Tool, name_words, quoted};
use crate::error::{CommandError, CommandResult};

/// What a `pick` that matches no choice is refused with.
pub(super) const STALE_CHOICE: &str = "that choice is no longer available; ask again";

/// Most choices of one question asked back.
const MAX_CHOICES: usize = 4;

/// A folder counts as a peer of the largest one of the same name when it is at least a quarter of it.
const PEER_SHARE: u64 = 4;

/// One way to read an unclear subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Choice {
    pub label: String,
    pub path: Option<String>,
    pub node: Option<NodeId>,
    pub target: Scope,
}

/// A question asked back: what the words could mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Clarify {
    pub prompt: String,
    pub choices: Vec<Choice>,
}

enum Outcome {
    Scope(Scope, Vec<String>),
    Ask(Clarify),
}

/// Where a question's subject is.
pub(super) struct Resolved {
    pub scope: Scope,
    pub notes: Vec<String>,
    pub clarify: Option<Clarify>,
}

impl Resolved {
    pub fn whole() -> Self {
        Self {
            scope: Scope::Whole,
            notes: Vec::new(),
            clarify: None,
        }
    }
}

/// The personal folder's role as the machine's own setting knows it.
fn user_dir_of(role: WellKnown) -> Option<UserDir> {
    match role {
        WellKnown::Desktop => Some(UserDir::Desktop),
        WellKnown::Documents => Some(UserDir::Documents),
        WellKnown::Music => Some(UserDir::Music),
        WellKnown::Pictures => Some(UserDir::Pictures),
        WellKnown::Videos => Some(UserDir::Videos),
        WellKnown::Downloads | WellKnown::Public | WellKnown::Templates => None,
    }
}

fn base_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

impl Lens<'_> {
    /// Resolves what the question names. `pick` is the id of a choice of the same question asked back
    /// before; one that fits no choice (or a question that is not asked back now) is refused.
    pub(super) fn resolve(&self, subject: &Subject, pick: Option<u8>) -> CommandResult<Resolved> {
        let outcome = match subject {
            Subject::None => Outcome::Scope(Scope::Whole, Vec::new()),
            Subject::Path(text) => self.resolve_path(text),
            Subject::Role { role, typed } => self.resolve_role(*role, Some(typed)),
            Subject::Rules { said, ids } => self.resolve_tools(said, ids),
            Subject::Name(text) => self.resolve_text(text),
        };

        match (outcome, pick) {
            (Outcome::Scope(scope, notes), None) => Ok(Resolved {
                scope,
                notes,
                clarify: None,
            }),
            (Outcome::Ask(clarify), None) => Ok(Resolved {
                scope: Scope::Whole,
                notes: Vec::new(),
                clarify: Some(clarify),
            }),
            (Outcome::Ask(clarify), Some(id)) => match clarify.choices.get(usize::from(id)) {
                Some(choice) => Ok(Resolved {
                    scope: choice.target.clone(),
                    notes: Vec::new(),
                    clarify: None,
                }),
                None => Err(CommandError::invalid(STALE_CHOICE)),
            },
            (Outcome::Scope(..), Some(_)) => Err(CommandError::invalid(STALE_CHOICE)),
        }
    }

    /// The scope a text names (a path or a name), as one folder. For a text that asks back, the first
    /// choice.
    #[cfg(test)]
    pub(super) fn resolve_scope(&self, text: &str) -> Scope {
        match self.resolve_text(text) {
            Outcome::Scope(scope, _) => scope,
            Outcome::Ask(clarify) => clarify.choices[0].target.clone(),
        }
    }

    fn look(&self, query: &str) -> Option<FolderMatch> {
        find_folders(self.tree, self.review.home(), query, 1)
            .ok()
            .and_then(|found| found.into_iter().next())
    }

    /// A path or a name the model read, as the tree knows it.
    fn resolve_text(&self, text: &str) -> Outcome {
        let text = text.trim();
        if text.is_empty() {
            return Outcome::Scope(Scope::Whole, Vec::new());
        }

        if text == "~" || text.starts_with("~/") || text.starts_with('/') {
            return self.resolve_path(text);
        }
        if let Some(role) = WellKnown::from_name(text) {
            return self.resolve_role(role, Some(text));
        }

        // A name with a slash in it is a path of folders, never what a rule is called.
        if !text.contains('/') {
            match self.by_rule(text) {
                Some(ByRule::Found {
                    title,
                    nodes,
                    notes,
                    tools,
                    ..
                }) => return self.of_tools(text, title, nodes, tools, notes),
                Some(ByRule::Absent(absent)) => {
                    return Outcome::Scope(
                        Scope::Missing {
                            text: text.to_owned(),
                            absent: Some(absent),
                        },
                        Vec::new(),
                    );
                }
                None => {}
            }
        }

        let mut by_name = self.folders_named(text);
        if by_name.is_empty() {
            for word in name_words(text) {
                by_name = self.folders_named(&word);
                if !by_name.is_empty() {
                    break;
                }
            }
        }

        match by_name.as_slice() {
            [] => Outcome::Scope(
                Scope::Missing {
                    text: text.to_owned(),
                    absent: None,
                },
                Vec::new(),
            ),
            [node] => Outcome::Scope(self.folder(*node), Vec::new()),
            nodes => Outcome::Ask(self.which_folder(text, nodes)),
        }
    }

    /// A path the user typed: that folder exactly.
    fn resolve_path(&self, text: &str) -> Outcome {
        // With a slash in it the query is only looked up as a path, so what comes back is exactly that
        // folder; the bare `~` is also found by name, so its path is compared.
        let hit = self
            .look(text)
            .filter(|found| text.contains('/') || found.path == "~");
        let role = self.role_of_path(text);

        match hit {
            Some(found) => {
                let mut notes = Vec::new();
                if let Some((role, real)) = role
                    && real != found.path
                {
                    notes.push(format!("Your {} folder is {}.", role_word(role), quoted(&real)));
                }

                Outcome::Scope(self.folder(found.node), notes)
            }
            None => Outcome::Scope(
                Scope::Missing {
                    text: text.to_owned(),
                    absent: role.map(|(role, real)| Absent {
                        named: text.to_owned(),
                        hint: format!("your {} folder is {}", role_word(role), quoted(&real)),
                    }),
                },
                Vec::new(),
            ),
        }
    }

    /// For `~/Documents`: the role of that name and where the user's own folder of it is.
    fn role_of_path(&self, text: &str) -> Option<(WellKnown, String)> {
        let name = text.strip_prefix("~/")?.trim_end_matches('/');
        if name.contains('/') {
            return None;
        }
        let role = WellKnown::from_name(name)?;
        let real = self.query_of(&self.role_path(role)?);

        Some((role, real))
    }

    /// Where the user's folder of this role is: the machine's own setting, else its usual name in the home.
    fn role_path(&self, role: WellKnown) -> Option<PathBuf> {
        if let Some(dir) = user_dir_of(role) {
            return self.review.env().user_dir(dir);
        }

        self.dirs
            .iter()
            .find(|(kind, _)| *kind == role)
            .map(|(_, path)| path.clone())
            .or_else(|| {
                self.review
                    .env()
                    .home
                    .as_ref()
                    .map(|home| home.join(role.english()))
            })
    }

    /// A path as the tree displays it: the home written as `~`.
    fn query_of(&self, path: &Path) -> String {
        match self.review.home() {
            Some(home) => match path.strip_prefix(home) {
                Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
                Ok(rest) => format!("~/{}", rest.to_string_lossy()),
                Err(_) => path.to_string_lossy().into_owned(),
            },
            None => path.to_string_lossy().into_owned(),
        }
    }

    /// The user's folder of a role. `typed` is the name the user used, when known: an English name for a
    /// folder that has another name here is said (`There is no “~/Downloads”; your downloads folder is
    /// “~/Pobrane”.`).
    fn resolve_role(&self, role: WellKnown, typed: Option<&str>) -> Outcome {
        let english = role.english();
        let missing = |hint: Option<String>| {
            Outcome::Scope(
                Scope::Missing {
                    text: english.to_owned(),
                    absent: hint.map(|hint| Absent {
                        named: english.to_owned(),
                        hint,
                    }),
                },
                Vec::new(),
            )
        };
        let Some(path) = self.role_path(role) else {
            return missing(None);
        };

        let query = self.query_of(&path);
        let Some(found) = self.look(&query).filter(|found| found.path == query) else {
            return missing(Some(format!(
                "your {} folder is {}",
                role_word(role),
                quoted(&query)
            )));
        };

        let mut notes = Vec::new();
        let said_in_english = typed.is_none_or(|typed| typed.eq_ignore_ascii_case(english));
        if said_in_english && !base_name(&found.path).eq_ignore_ascii_case(english) {
            notes.push(format!(
                "There is no {}; your {} folder is {}.",
                quoted(&format!("~/{english}")),
                role_word(role),
                quoted(&found.path)
            ));
        }

        Outcome::Scope(self.folder(found.node), notes)
    }

    /// A tool of the rules the question named, by the ids of its rules.
    fn resolve_tools(&self, said: &str, ids: &[&'static str]) -> Outcome {
        let named: Vec<&platter_rules::Rule> = self
            .review
            .rules()
            .rules()
            .iter()
            .filter(|rule| ids.contains(&rule.id))
            .collect();

        match self.by_named_rules(&named, said) {
            Some(ByRule::Found {
                title,
                nodes,
                notes,
                tools,
                ..
            }) => self.of_tools(said, title, nodes, tools, notes),
            Some(ByRule::Absent(absent)) => Outcome::Scope(
                Scope::Missing {
                    text: said.to_owned(),
                    absent: Some(absent),
                },
                Vec::new(),
            ),
            None => Outcome::Scope(
                Scope::Missing {
                    text: said.to_owned(),
                    absent: None,
                },
                Vec::new(),
            ),
        }
    }

    /// The folders of the tools that matched: one tool is the scope (with `notes` about its places outside
    /// the scan), two or more are asked back.
    fn of_tools(
        &self,
        said: &str,
        title: String,
        nodes: Vec<NodeId>,
        tools: Vec<Tool>,
        notes: Vec<String>,
    ) -> Outcome {
        if tools.len() < 2 {
            return Outcome::Scope(self.scope_of(title, nodes), notes);
        }

        let choices = tools
            .into_iter()
            .take(MAX_CHOICES)
            .map(|tool| {
                let target = self.scope_of(tool.title.clone(), tool.nodes);
                let (path, node) = match &target {
                    Scope::Folder { node, path } => (Some(path.clone()), Some(*node)),
                    _ => (None, None),
                };

                Choice {
                    label: tool.title,
                    path,
                    node,
                    target,
                }
            })
            .collect();

        Outcome::Ask(Clarify {
            prompt: format!(
                "{} could mean more than one thing. Which one do you mean?",
                quoted(said)
            ),
            choices,
        })
    }

    /// The folders with a name, for a subject that is a bare name: `~/<name>`, else the folders called that
    /// (or, failing that, starting with it or holding it), largest first. Folders inside another of them are
    /// not separate; of the rest, the largest alone when the others are much smaller (a quarter or less of
    /// it), else every one of similar standing.
    fn folders_named(&self, text: &str) -> Vec<NodeId> {
        let exact = format!("~/{text}");
        if let Some(found) = self.look(&exact).filter(|found| found.path == exact) {
            return vec![found.node];
        }

        let found = find_folders(self.tree, self.review.home(), text, MAX_FOLDER_MATCHES).unwrap_or_default();
        let lowered = text.to_lowercase();
        let tier = |found: &FolderMatch| {
            let name = base_name(&found.path).to_lowercase();

            if name == lowered {
                0
            } else if name.starts_with(&lowered) {
                1
            } else {
                2
            }
        };
        let Some(best) = found.iter().map(tier).min() else {
            return Vec::new();
        };

        let mut nodes: Vec<NodeId> = found
            .iter()
            .filter(|found| tier(found) == best)
            .map(|found| found.node)
            .collect();
        self.by_size(&mut nodes);
        let outer: Vec<NodeId> = nodes
            .iter()
            .copied()
            .filter(|&node| {
                !nodes
                    .iter()
                    .any(|&other| other != node && self.within(node, other))
            })
            .collect();

        let Some(&largest) = outer.first() else {
            return Vec::new();
        };
        let peers: Vec<NodeId> = outer
            .iter()
            .copied()
            .filter(|&node| self.size(node).saturating_mul(PEER_SHARE) >= self.size(largest))
            .collect();

        if peers.len() < 2 { vec![largest] } else { peers }
    }

    /// The question asked back for folders that share a name.
    fn which_folder(&self, text: &str, nodes: &[NodeId]) -> Clarify {
        let choices = nodes
            .iter()
            .take(MAX_CHOICES)
            .map(|&node| {
                let path = self.review.display(node);

                Choice {
                    label: path.clone(),
                    path: Some(path.clone()),
                    node: Some(node),
                    target: Scope::Folder { node, path },
                }
            })
            .collect();

        Clarify {
            prompt: format!(
                "I found more than one folder called {}. Which one do you mean?",
                quoted(text)
            ),
            choices,
        }
    }
}

/// `downloads`, `documents`: the role as it is said in a sentence.
fn role_word(role: WellKnown) -> String {
    role.english().to_lowercase()
}
