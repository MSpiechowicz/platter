//! Explains where a folder stands among the rule matches: what covers it, what is matched inside it
//! and what is matched beside it. Built once from one pass of the rules, then answers from memory.

use std::collections::HashMap;

use platter_core::dto::{SuggestionSource, Verdict};
use platter_core::units::Units;
use platter_core::{Environment, NodeId, Tree};

use crate::evaluate::{run, source_of};
use crate::found::Found;
use crate::reason::{Facts, render};
use crate::rule::{Rule, RuleSet};

/// The rule that covers a folder: it matched the folder itself or a folder above it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Covering {
    pub id: &'static str,
    pub title: &'static str,
    pub verdict: Verdict,
    /// Whose rule it is: the built-in table's or the user's own (`user-keep`, `user-safe`).
    pub source: SuggestionSource,
    /// The rule's reason, rendered for the matches it really made in this scan.
    pub reason: String,
    /// The matched node: the folder asked about, or the ancestor that carries the match.
    pub at: NodeId,
}

/// One rule that matched something in the scan.
struct Entry {
    id: &'static str,
    title: &'static str,
    verdict: Verdict,
    source: SuggestionSource,
    reason: String,
}

/// Where the rule matches of one scan are, as three questions about a node.
///
/// Built from the same pass as [`evaluate`](crate::evaluate) and [`matches`](crate::matches), so it
/// agrees with them: a kept folder is a match, nothing is matched inside a match, and a scan that starts
/// inside a kept folder has no matches at all.
pub struct Explainer<'a> {
    tree: &'a Tree,
    entries: Vec<Entry>,
    /// Matched node -> entry.
    at: HashMap<u32, usize>,
    /// Folder -> bytes matched strictly below it, per entry.
    below: HashMap<u32, Vec<(usize, u64)>>,
    /// Folder -> the matched nodes that are its direct children, with their entry.
    children: HashMap<u32, Vec<(u32, usize)>>,
}

impl<'a> Explainer<'a> {
    /// Runs the rules over `tree` once. The sizes in the reasons are written in binary units.
    pub fn new(tree: &'a Tree, env: &Environment, rules: &RuleSet) -> Self {
        Self::with_units(tree, env, rules, Units::Binary)
    }

    /// [`Explainer::new`], with the sizes in the reasons written in `units`.
    pub fn with_units(tree: &'a Tree, env: &Environment, rules: &RuleSet, units: Units) -> Self {
        match run(tree, env, rules) {
            Some((verdicts, found)) => Self::from_found(tree, rules.rules(), &verdicts, &found, units),
            None => Self::from_found(tree, &[], &[], &[], units),
        }
    }

    /// From the result of a pass: `verdicts` and `found` hold one item per rule.
    pub(crate) fn from_found(
        tree: &'a Tree,
        rules: &[Rule],
        verdicts: &[Verdict],
        found: &[Found],
        units: Units,
    ) -> Self {
        let mut explainer = Self {
            tree,
            entries: Vec::new(),
            at: HashMap::new(),
            below: HashMap::new(),
            children: HashMap::new(),
        };

        for ((rule, verdict), found) in rules.iter().zip(verdicts).zip(found) {
            if found.is_empty() {
                continue;
            }

            let facts = Facts {
                count: found.all.len() as u64,
                days: rule.matcher.age_days(),
                min_bytes: rule.matcher.min_bytes(),
                units,
            };
            explainer.entries.push(Entry {
                id: rule.id,
                title: rule.title,
                verdict: *verdict,
                source: source_of(rule),
                reason: render(rule.reason, &facts),
            });

            let entry = explainer.entries.len() - 1;
            for &node in &found.all {
                explainer.record(node, entry);
            }
        }

        explainer
    }

    fn record(&mut self, node: NodeId, entry: usize) {
        self.at.insert(node.0, entry);

        let allocated = self.tree.allocated(node);
        let mut parent = self.tree.parent(node);
        if let Some(parent) = parent {
            self.children.entry(parent.0).or_default().push((node.0, entry));
        }

        while let Some(folder) = parent {
            let sums = self.below.entry(folder.0).or_default();
            match sums.iter_mut().find(|(known, _)| *known == entry) {
                Some((_, bytes)) => *bytes = bytes.saturating_add(allocated),
                None => sums.push((entry, allocated)),
            }
            parent = self.tree.parent(folder);
        }
    }

    /// The rule match at `node` or at the nearest folder above it, if any.
    pub fn covering(&self, node: NodeId) -> Option<Covering> {
        if !self.tree.contains(node) {
            return None;
        }

        let mut current = Some(node);
        while let Some(at) = current {
            if let Some(&entry) = self.at.get(&at.0) {
                let entry = &self.entries[entry];
                return Some(Covering {
                    id: entry.id,
                    title: entry.title,
                    verdict: entry.verdict,
                    source: entry.source,
                    reason: entry.reason.clone(),
                    at,
                });
            }
            current = self.tree.parent(at);
        }

        None
    }

    /// The rules that matched below `node`, each with the bytes it matched there, largest first (ties by
    /// rule id). Empty when nothing below `node` is matched. The match at `node` itself is not counted.
    pub fn inside(&self, node: NodeId) -> Vec<(&'static str, u64)> {
        let mut found: Vec<(&'static str, u64)> = self
            .below
            .get(&node.0)
            .into_iter()
            .flatten()
            .map(|&(entry, bytes)| (self.entries[entry].id, bytes))
            .collect();
        found.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

        found
    }

    /// The rules that matched on the siblings of `node` (not on `node` itself), by rule id.
    pub fn beside(&self, node: NodeId) -> Vec<&'static str> {
        let Some(parent) = self.tree.parent(node) else {
            return Vec::new();
        };

        let mut ids: Vec<&'static str> = self
            .children
            .get(&parent.0)
            .into_iter()
            .flatten()
            .filter(|&&(sibling, _)| sibling != node.0)
            .map(|&(_, entry)| self.entries[entry].id)
            .collect();
        ids.sort_unstable();
        ids.dedup();

        ids
    }

    /// Whether some match lies strictly below `node`.
    pub fn holds(&self, node: NodeId) -> bool {
        self.below.contains_key(&node.0)
    }

    /// Whether `node` is covered by a match or holds one: a folder the rules already have an opinion on.
    pub fn touches(&self, node: NodeId) -> bool {
        self.holds(node) || self.covering(node).is_some()
    }
}
