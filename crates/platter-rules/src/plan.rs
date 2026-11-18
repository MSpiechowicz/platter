//! Which rules can apply to a scan, and where. Everything that needs a path is resolved here, once.

use std::path::{Path, PathBuf};

use platter_core::dto::Verdict;
use platter_core::{Environment, NodeId, NodeKind, Os, Tree, UserRule};

use crate::resolve::{locate, same_os_str, strip_prefix_folded};
use crate::rule::{FilesIn, Matcher, Named, Rule, RuleSet};

/// What a rule does at a node it is anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// The node itself is a match.
    Claim,
    /// Files below the node are looked at (a file rule).
    Region,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Anchor {
    pub node: u32,
    pub rule: usize,
    pub role: Role,
}

/// The rules that apply to this tree and environment, ready for the single pass.
pub(crate) struct Plan {
    /// Fixed and home rules found in the tree, sorted by node, then rule.
    anchors: Vec<Anchor>,
    /// One bit per node: does an anchor sit here? Keeps the per-node cost to a single bit test.
    marked: Vec<u64>,
    /// One bit per node: does a `keep` anchor sit strictly below it? Such a folder is never claimed by
    /// another rule, because acting on it would act on the kept folder inside. A `keep` anchor can sit on
    /// any kind of node (a skipped mount, say), so this sees a kept mount too.
    holds_keep: Vec<u64>,
    /// Name rules (rule indexes, in table order).
    pub named: Vec<usize>,
}

impl Plan {
    pub fn new(tree: &Tree, env: &Environment, set: &RuleSet) -> Self {
        let mut anchors = Vec::new();
        let mut named = Vec::new();
        // Windows file names are not case sensitive, so a rule's path finds the folder however it is spelled.
        let fold_case = env.os == Os::Windows;

        for (index, rule) in set.rules().iter().enumerate() {
            if rule.os.is_some_and(|os| os != env.os) {
                continue;
            }

            match &rule.matcher {
                Matcher::Fixed(_)
                | Matcher::Home(_)
                | Matcher::System(_)
                | Matcher::UserDir(_)
                | Matcher::User(_) => {
                    for path in claimed_paths(rule, env, set.user()) {
                        push_anchor(
                            &mut anchors,
                            tree,
                            &path,
                            index,
                            rule.verdict,
                            Role::Claim,
                            fold_case,
                        );
                    }
                }
                Matcher::Files(files) => {
                    if let Some(path) = files_folder(env, files.location) {
                        push_anchor(
                            &mut anchors,
                            tree,
                            &path,
                            index,
                            rule.verdict,
                            Role::Region,
                            fold_case,
                        );
                    }
                }
                Matcher::Named(_) => named.push(index),
            }
        }

        anchors.sort_unstable_by_key(|anchor| (anchor.node, anchor.rule));

        let mut marked = vec![0u64; tree.len().div_ceil(64)];
        for anchor in &anchors {
            marked[anchor.node as usize / 64] |= 1 << (anchor.node % 64);
        }

        let mut holds_keep = vec![0u64; tree.len().div_ceil(64)];
        for anchor in &anchors {
            if set.rules()[anchor.rule].verdict != Verdict::Keep {
                continue;
            }

            let mut above = tree.parent(NodeId(anchor.node));
            while let Some(node) = above {
                holds_keep[node.0 as usize / 64] |= 1 << (node.0 % 64);
                above = tree.parent(node);
            }
        }

        Self {
            anchors,
            marked,
            holds_keep,
            named,
        }
    }

    /// Whether a `keep` anchor sits somewhere below `node`.
    pub fn holds_keep(&self, node: NodeId) -> bool {
        self.holds_keep[node.0 as usize / 64] & (1 << (node.0 % 64)) != 0
    }

    pub fn is_anchor(&self, node: NodeId) -> bool {
        self.marked[node.0 as usize / 64] & (1 << (node.0 % 64)) != 0
    }

    /// The anchors at `node`, ordered by rule.
    pub fn anchors_at(&self, node: NodeId) -> &[Anchor] {
        let start = self.anchors.partition_point(|anchor| anchor.node < node.0);
        let len = self.anchors[start..]
            .iter()
            .take_while(|anchor| anchor.node == node.0)
            .count();

        &self.anchors[start..start + len]
    }
}

fn under_home(env: &Environment, relative: &str) -> Option<PathBuf> {
    env.home.as_ref().map(|home| home.join(relative))
}

/// The folder a file rule looks below, when known.
fn files_folder(env: &Environment, location: FilesIn) -> Option<PathBuf> {
    match location {
        FilesIn::Home(relative) => under_home(env, relative),
        FilesIn::UserDir(dir) => env.user_dir(dir),
    }
}

/// The folders a path rule (fixed, home or personal folder, or the user's own) is about, when known.
fn claimed_paths(rule: &Rule, env: &Environment, user: &[UserRule]) -> Vec<PathBuf> {
    let one = |path: Option<PathBuf>| path.into_iter().collect();

    match &rule.matcher {
        Matcher::Fixed(path) => vec![PathBuf::from(path)],
        Matcher::Home(relative) => one(under_home(env, relative)),
        Matcher::System(relative) => one(env.system_root.as_ref().map(|root| root.join(relative))),
        Matcher::UserDir(dir) => one(env.user_dir(*dir)),
        Matcher::User(verdict) => user
            .iter()
            .filter(|rule| rule.verdict == *verdict)
            .map(|rule| rule.folder.clone())
            .collect(),
        Matcher::Named(_) | Matcher::Files(_) => Vec::new(),
    }
}

/// The `keep` rule whose folder holds the whole scan, when the scan starts inside one. A scan of the
/// kept folder itself is not "inside": the root is then matched like any other node.
pub(crate) fn kept_root(tree: &Tree, env: &Environment, set: &RuleSet) -> Option<usize> {
    let root = tree.root_path();

    set.rules().iter().enumerate().find_map(|(index, rule)| {
        if rule.verdict != Verdict::Keep || rule.os.is_some_and(|os| os != env.os) {
            return None;
        }

        let fold_case = env.os == Os::Windows;

        claimed_paths(rule, env, set.user())
            .iter()
            .any(|kept| {
                if fold_case {
                    !same_os_str(root.as_os_str(), kept.as_os_str())
                        && strip_prefix_folded(root, kept).is_some()
                } else {
                    root != kept && root.starts_with(kept)
                }
            })
            .then_some(index)
    })
}

/// Anchors `rule` at the node at `path`, when the scan contains one.
///
/// Only a directory is a place to suggest or look into. A `keep` claim is anchored at whatever node it
/// finds, though: a kept folder that is a mount the scan did not enter (`one_file_system`), a link or a
/// file in the tree is still kept, and everything around it must see that. Such a node is never entered.
fn push_anchor(
    anchors: &mut Vec<Anchor>,
    tree: &Tree,
    path: &Path,
    rule: usize,
    verdict: Verdict,
    role: Role,
    fold_case: bool,
) {
    let Some(node) = locate(tree, path, fold_case) else {
        return;
    };

    let keeps = verdict == Verdict::Keep && role == Role::Claim;
    if tree.kind(node) == NodeKind::Dir || keeps {
        anchors.push(Anchor {
            node: node.0,
            rule,
            role,
        });
    }
}

/// The name rule of `rule`, which the plan only lists for name rules.
pub(crate) fn named_of(rule: &Rule) -> Option<&Named> {
    match &rule.matcher {
        Matcher::Named(named) => Some(named),
        _ => None,
    }
}
