//! Which rules can apply to a scan, and where. Everything that needs a path is resolved here, once.

use std::path::{Path, PathBuf};

use platter_core::dto::Verdict;
use platter_core::{Environment, NodeId, NodeKind, Tree};

use crate::resolve::locate;
use crate::rule::{Matcher, Named, Rule};

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
    /// Name rules (rule indexes, in table order).
    pub named: Vec<usize>,
}

impl Plan {
    pub fn new(tree: &Tree, env: &Environment, rules: &[Rule]) -> Self {
        let mut anchors = Vec::new();
        let mut named = Vec::new();

        for (index, rule) in rules.iter().enumerate() {
            if rule.os.is_some_and(|os| os != env.os) {
                continue;
            }

            match &rule.matcher {
                Matcher::Fixed(_) | Matcher::Home(_) | Matcher::UserDir(_) => {
                    if let Some(path) = claimed_path(rule, env) {
                        push_anchor(&mut anchors, tree, &path, index, Role::Claim);
                    }
                }
                Matcher::Files(files) => {
                    if let Some(path) = under_home(env, files.home_relative) {
                        push_anchor(&mut anchors, tree, &path, index, Role::Region);
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

        Self {
            anchors,
            marked,
            named,
        }
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

/// The folder a path rule (fixed, home or personal folder) is about, when it is known.
pub(crate) fn claimed_path(rule: &Rule, env: &Environment) -> Option<PathBuf> {
    match &rule.matcher {
        Matcher::Fixed(path) => Some(PathBuf::from(path)),
        Matcher::Home(relative) => under_home(env, relative),
        Matcher::UserDir(dir) => env.user_dir(*dir),
        Matcher::Named(_) | Matcher::Files(_) => None,
    }
}

/// The `keep` rule whose folder holds the whole scan, when the scan starts inside one. A scan of the
/// kept folder itself is not "inside": the root is then matched like any other node.
pub(crate) fn kept_root(tree: &Tree, env: &Environment, rules: &[Rule]) -> Option<usize> {
    let root = tree.root_path();

    rules.iter().enumerate().find_map(|(index, rule)| {
        if rule.verdict != Verdict::Keep || rule.os.is_some_and(|os| os != env.os) {
            return None;
        }

        let kept = claimed_path(rule, env)?;
        (root != kept && root.starts_with(&kept)).then_some(index)
    })
}

/// Anchors `rule` at the directory at `path`, when the scan contains one.
fn push_anchor(anchors: &mut Vec<Anchor>, tree: &Tree, path: &Path, rule: usize, role: Role) {
    let Some(node) = locate(tree, path) else {
        return;
    };

    // A file, a link or a skipped mount is never suggested, and a mount has nothing to look into.
    if tree.kind(node) == NodeKind::Dir {
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
