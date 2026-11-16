//! The folders the local model may be asked about: big, readable, and none that a rule already speaks for.

use std::cmp::Reverse;

use platter_core::{Environment, NodeId, NodeKind, Tree};
use platter_rules::{Explainer, RuleSet, scan_is_kept};

use crate::protect::Protected;

/// How many folders to pick and how big they must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateLimits {
    /// The smallest allocated size of a candidate. Smaller folders are not even entered.
    pub min_bytes: u64,
    /// The most candidates; the largest are kept.
    pub max: usize,
}

impl CandidateLimits {
    /// The 50 largest folders of at least 256 MiB.
    pub const DEFAULT: Self = Self {
        min_bytes: 256 << 20,
        max: 50,
    };
}

/// The folders of a scan the local model may label, largest first, at most `limits.max`.
///
/// The search goes top down and enters only folders of at least `limits.min_bytes`. A folder is a candidate
/// when it is a readable folder (with nothing unreadable inside, since such a folder could not be cleaned
/// anyway), is not the scan root, is not covered by a rule match (it is one, or lies inside one), holds no
/// match, and no protected place is it, holds it or lies around it. A candidate is not entered, so
/// candidates never contain one another. A folder that holds a match or a protected place is entered, not
/// chosen, so what lies beside the match can be. Matched folders are never entered.
///
/// Without a known home directory (nothing can be protected) and for a scan that starts inside a kept
/// folder there are no candidates. Protected places are compared by path only here; the plan that cleans a
/// label compares identities too.
pub fn label_candidates(
    tree: &Tree,
    env: &Environment,
    rules: &RuleSet,
    limits: CandidateLimits,
) -> Vec<NodeId> {
    if scan_is_kept(tree, env, rules) {
        return Vec::new();
    }
    let Some(protected) = Protected::for_environment(env) else {
        return Vec::new();
    };

    let explainer = Explainer::new(tree, env, rules);
    let mut candidates: Vec<NodeId> = Vec::new();
    let mut pending = vec![tree.root()];

    while let Some(node) = pending.pop() {
        if tree.kind(node) != NodeKind::Dir
            || tree.allocated(node) < limits.min_bytes
            || tree.is_unreadable(node)
        {
            continue;
        }

        // A match is never entered, and nothing inside one is looked at.
        if explainer.covering(node).is_some() {
            continue;
        }

        let chosen = node != tree.root()
            && !explainer.holds(node)
            && !tree.has_unreadable(node)
            && protected.refuses(&tree.path_bytes(node)).is_none();

        if chosen {
            candidates.push(node);
        } else {
            pending.extend(tree.children(node));
        }
    }

    candidates.sort_unstable_by_key(|&node| (Reverse(tree.allocated(node)), node));
    candidates.truncate(limits.max);
    candidates
}
