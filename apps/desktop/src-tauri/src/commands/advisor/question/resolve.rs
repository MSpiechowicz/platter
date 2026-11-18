//! Where the subject of a question is in the finished scan, or whether it is clear at all.
//!
//! The subject is looked up in the tree and nowhere else, in this order:
//!
//! 1. A typed path is that exact folder, or else that exact file.
//! 2. A tool the rules know (`docker`) is the folders its rules matched in this scan; where the rules have
//!    one place that the scan does not hold, the answer says where it is.
//! 3. A personal folder by its role (`Downloads`, `Pobrane`) is the user's real one (XDG user-dirs): a folder
//!    with the same name somewhere else is never "your Downloads".
//! 4. A bare name is `~/<name>`, else the folders with that name. A file counts only by its exact name, and
//!    an exact file beats folders that only start with the name or hold it.
//!
//! A subject is never guessed. When two unrelated tools, or two folders of a similar standing, fit the
//! words, the question is asked back ([`Clarify`]); a later `ask_scan` with the id of a choice resolves the
//! same candidates again and takes that one. A subject that is not in the scan is [`Scope::Missing`], and
//! the answer says only that.

use std::path::{Path, PathBuf};

use platter_ai::WellKnown;
use platter_core::query::{
    FolderMatch, MAX_ENTRY_MATCHES, MAX_FOLDER_MATCHES, entry_at, find_files_named, find_folders,
};
use platter_core::{NodeId, NodeKind, UserDir};

use super::reading::Subject;
use super::{Absent, ByRule, Lens, Scope, Tool, name_words, quoted};
use crate::error::{CommandError, CommandResult};

/// What a `pick` that matches no choice is refused with.
pub(super) const STALE_CHOICE: &str = "that choice is no longer available, so ask again";

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

/// What the user saw of a choice, and what it stood for: a pick is taken only when this reading finds the
/// same under its id (a tool's choice with the same title but other folders is not the same choice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceKey {
    label: String,
    path: Option<String>,
    node: Option<NodeId>,
    target: Scope,
}

impl Choice {
    pub(super) fn key(&self) -> ChoiceKey {
        ChoiceKey {
            label: self.label.clone(),
            path: self.path.clone(),
            node: self.node,
            target: self.target.clone(),
        }
    }
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

/// The folders a bare name fits: of the best tier only.
#[derive(Default)]
struct Named {
    nodes: Vec<NodeId>,
    /// The name is the folder's whole name (or `~/<name>`), not only the start of it or a part of it.
    exact: bool,
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
        WellKnown::Downloads => Some(UserDir::Downloads),
        WellKnown::Public | WellKnown::Templates => None,
    }
}

fn base_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

impl Lens<'_> {
    /// Resolves what the question names. `pick` is the id of a choice of the same question asked back
    /// before; one that fits no choice (or a question that is not asked back now) is refused. `shown` is
    /// what the user was shown for that question, when the caller knows it: the choice this reading finds
    /// under `pick` must be the one shown under it, so a reading that came out otherwise is refused too.
    pub(super) fn resolve(
        &self,
        subject: &Subject,
        pick: Option<u8>,
        shown: Option<&[ChoiceKey]>,
    ) -> CommandResult<Resolved> {
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
                Some(choice)
                    if shown.is_some_and(|shown| shown.get(usize::from(id)) != Some(&choice.key())) =>
                {
                    Err(CommandError::invalid(STALE_CHOICE))
                }
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

        let files = self.files_named(text);
        let by_name = self.folders_named(text);
        if !files.is_empty() {
            return self.of_files(text, &files, &by_name);
        }

        let mut by_name = by_name.nodes;
        if by_name.is_empty() {
            // The words of a text that is no name are looked for as folders only.
            for word in name_words(text) {
                by_name = self.folders_named(&word).nodes;
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

    /// A path the user typed: that folder exactly, or else that file.
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
            None => match self.file_at(text) {
                Some(node) => Outcome::Scope(self.file_scope(node), Vec::new()),
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
            },
        }
    }

    /// The file at exactly this path, when the scan holds one.
    fn file_at(&self, path: &str) -> Option<NodeId> {
        entry_at(self.tree, self.review.home(), path).filter(|&node| self.tree.kind(node) == NodeKind::File)
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
    /// folder that has another name here is said (`There is no “~/Downloads”. Your downloads folder is
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
                "There is no {}. Your {} folder is {}.",
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
    fn folders_named(&self, text: &str) -> Named {
        let exact = format!("~/{text}");
        if let Some(found) = self.look(&exact).filter(|found| found.path == exact) {
            return Named {
                nodes: vec![found.node],
                exact: true,
            };
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
            return Named::default();
        };

        let nodes: Vec<NodeId> = found
            .iter()
            .filter(|found| tier(found) == best)
            .map(|found| found.node)
            .collect();

        Named {
            nodes: self.of_similar_standing(nodes),
            exact: best == 0,
        }
    }

    /// The files with exactly this name (case ignored), for a subject that is a bare name: `~/<name>`, else
    /// the files called that anywhere in the scan. Of several, the largest alone when the others are much
    /// smaller, else every one of similar standing.
    fn files_named(&self, text: &str) -> Vec<NodeId> {
        if let Some(node) = self.file_at(&format!("~/{text}")) {
            return vec![node];
        }

        // Files only, so any number of folders of the same name cannot push a small file out.
        let found =
            find_files_named(self.tree, self.review.home(), text, MAX_ENTRY_MATCHES).unwrap_or_default();
        let nodes: Vec<NodeId> = found.iter().map(|found| found.node).collect();

        self.of_similar_standing(nodes)
    }

    /// Of entries of one name, largest first and with the ones inside another left out: the largest alone
    /// when the others are a quarter of it or less, else every one of a similar size.
    fn of_similar_standing(&self, mut nodes: Vec<NodeId>) -> Vec<NodeId> {
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

    /// A bare name that is the exact name of files. One file, with no folder of that exact name, is the
    /// scope. A file and a folder both called that, or several files of a similar size, are asked back.
    fn of_files(&self, text: &str, files: &[NodeId], folders: &Named) -> Outcome {
        let folders: &[NodeId] = if folders.exact { &folders.nodes } else { &[] };

        match (files, folders) {
            ([node], []) => Outcome::Scope(self.file_scope(*node), Vec::new()),
            _ => Outcome::Ask(self.which_entry(text, folders, files)),
        }
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

    /// The question asked back for files, or for files and folders, that share a name: the folders first,
    /// then the files, at most [`MAX_CHOICES`] and at least one of each kind that there is. A choice says
    /// what it is when the kinds are mixed.
    fn which_entry(&self, text: &str, folders: &[NodeId], files: &[NodeId]) -> Clarify {
        let mixed = !folders.is_empty();
        let folders = &folders[..folders.len().min(MAX_CHOICES - 1)];
        let files = &files[..files.len().min(MAX_CHOICES - folders.len())];

        let choice = |node: NodeId, is_file: bool| {
            let path = self.review.display(node);
            let label = match (mixed, is_file) {
                (false, _) => path.clone(),
                (true, true) => format!("{path} (file)"),
                (true, false) => format!("{path} (folder)"),
            };
            let target = if is_file {
                Scope::File {
                    node,
                    path: path.clone(),
                }
            } else {
                Scope::Folder {
                    node,
                    path: path.clone(),
                }
            };

            Choice {
                label,
                path: Some(path),
                node: Some(node),
                target,
            }
        };
        let choices = folders
            .iter()
            .map(|&node| choice(node, false))
            .chain(files.iter().map(|&node| choice(node, true)))
            .collect();

        let what = if mixed { "file or folder" } else { "file" };

        Clarify {
            prompt: format!(
                "I found more than one {what} called {}. Which one do you mean?",
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
