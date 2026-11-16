//! A cleanup plan: what would be cleaned, how, and what was refused.

use platter_core::dto::Verdict;
use platter_core::{EntryProbe, Environment, Tree};
use platter_rules::{RuleSet, matches, scan_is_kept};

use crate::action::{CleanupAction, action_for};
use crate::guard::{Guard, Refusal, VerifiedTarget};
use crate::path::{components, display, is_within, owned};
use crate::protect::Protected;

/// The verified targets of one selected rule and how they are cleaned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanGroup {
    pub(crate) rule_id: &'static str,
    pub(crate) verdict: Verdict,
    pub(crate) action: CleanupAction,
    pub(crate) targets: Vec<VerifiedTarget>,
    /// Sum of the targets' scanned sizes.
    pub(crate) allocated: u64,
}

/// Read-only from outside: only planning decides what a group does.
impl PlanGroup {
    pub fn rule_id(&self) -> &'static str {
        self.rule_id
    }
    pub fn verdict(&self) -> Verdict {
        self.verdict
    }
    pub fn action(&self) -> CleanupAction {
        self.action
    }
    pub fn targets(&self) -> &[VerifiedTarget] {
        &self.targets
    }
    /// Sum of the targets' scanned sizes.
    pub fn allocated(&self) -> u64 {
        self.allocated
    }
}

/// A matched node that is not in the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub rule_id: &'static str,
    /// The path for people to read.
    pub path: String,
    pub reason: Refusal,
}

/// The result of planning. Making it changes nothing on disk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CleanupPlan {
    pub(crate) groups: Vec<PlanGroup>,
    pub refused: Vec<Refused>,
    /// Selected ids that are not rules, or rules that matched nothing in this scan.
    pub not_found: Vec<String>,
    /// Sum of all groups' scanned sizes. What cleaning frees can differ.
    pub allocated: u64,
    /// The folder the scan started from, and how many components it has: below it nothing acted on may
    /// cross a mount.
    pub(crate) scan_root: std::path::PathBuf,
    pub(crate) root_depth: usize,
}

impl CleanupPlan {
    /// The groups of verified targets, one per selected rule that has any.
    pub fn groups(&self) -> &[PlanGroup] {
        &self.groups
    }

    /// The folder the scan started from.
    pub fn scan_root(&self) -> &std::path::Path {
        &self.scan_root
    }

    pub fn targets(&self) -> impl Iterator<Item = &VerifiedTarget> {
        self.groups.iter().flat_map(|group| group.targets.iter())
    }
}

/// Plans the cleanup of the rules in `selected` over a finished scan. The rules run again, so every
/// match is planned (a suggestion lists only the largest few), and each match goes through the guard,
/// which looks at the disk through `probe`. Nothing is changed.
pub fn plan_cleanup(
    tree: &Tree,
    env: &Environment,
    rules: &RuleSet,
    selected: &[&str],
    probe: &impl EntryProbe,
) -> CleanupPlan {
    let mut plan = CleanupPlan {
        scan_root: tree.root_path().to_path_buf(),
        root_depth: components(tree.root_path().as_os_str().as_encoded_bytes())
            .map_or(0, |parts| parts.len()),
        ..CleanupPlan::default()
    };
    let found = matches(tree, env, rules);

    for id in selected {
        if !found.iter().any(|rule| rule.id == *id) {
            plan.not_found.push((*id).to_owned());
        }
    }

    let chosen: Vec<_> = found.iter().filter(|rule| selected.contains(&rule.id)).collect();
    let protected = Protected::for_environment(env).map(|mut protected| {
        protected.learn_identities(probe);
        protected
    });
    let root = owned(tree.root_path().as_os_str().as_encoded_bytes());

    let (Some(protected), Some(root)) = (protected, root.clone()) else {
        let reason = if root.is_none() {
            Refusal::BadPath
        } else {
            Refusal::HomeUnknown
        };
        for rule in chosen {
            for &node in &rule.nodes {
                plan.refused.push(Refused {
                    rule_id: rule.id,
                    path: display(&tree.path_bytes(node)),
                    reason: reason.clone(),
                });
            }
        }
        return plan;
    };

    let guard = Guard {
        tree,
        root,
        protected: &protected,
        probe,
    };

    for rule in chosen {
        let Some(action) = action_for(rule.id) else {
            let reason = if rule.verdict == Verdict::Keep {
                Refusal::Kept
            } else {
                Refusal::NotCleaned
            };
            for &node in &rule.nodes {
                plan.refused.push(Refused {
                    rule_id: rule.id,
                    path: display(&tree.path_bytes(node)),
                    reason: reason.clone(),
                });
            }
            continue;
        };

        // The files next to a folder that made a name rule match it, to be checked again.
        let named = rules
            .rules()
            .iter()
            .find(|known| known.id == rule.id)
            .and_then(|known| match &known.matcher {
                platter_rules::Matcher::Named(named) => Some(*named),
                _ => None,
            });
        let single: Vec<&str> = named.and_then(|named| named.sibling).into_iter().collect();
        let mut markers: Vec<&[&str]> = Vec::new();
        if !single.is_empty() {
            markers.push(&single);
        }
        if let Some(named) = named
            && !named.sibling_any.is_empty()
        {
            markers.push(named.sibling_any);
        }

        let mut group = PlanGroup {
            rule_id: rule.id,
            verdict: rule.verdict,
            action,
            targets: Vec::new(),
            allocated: 0,
        };
        for &node in &rule.nodes {
            match guard.verify(
                node,
                rule.verdict,
                matches!(action, CleanupAction::RunTool(_)),
                &markers,
            ) {
                Ok(target) => {
                    group.allocated = group.allocated.saturating_add(target.allocated());
                    group.targets.push(target);
                }
                Err(reason) => plan.refused.push(Refused {
                    rule_id: rule.id,
                    path: display(&tree.path_bytes(node)),
                    reason,
                }),
            }
        }

        if !group.targets.is_empty() {
            plan.groups.push(group);
        }
    }

    drop_nested(&mut plan);
    plan.allocated = plan
        .groups
        .iter()
        .fold(0u64, |sum, group| sum.saturating_add(group.allocated));
    plan
}

/// The rule id of folders the user marked by hand in the Explorer.
pub const MARKED: &str = "marked";

/// Plans the cleanup of folders and files the user marked by hand. No rule vouches for them, so each is
/// `review` and goes to the trash, never deleted directly. Each goes through the same guard as a rule's
/// match (protected places, links, mounts, freshness), and is refused when a `keep` rule protects it, a
/// folder inside it, or a folder around it. A marked entry inside another marked one is left to the
/// outer one. Nothing is changed.
pub fn plan_marked(
    tree: &Tree,
    env: &Environment,
    rules: &RuleSet,
    marked: &[platter_core::NodeId],
    probe: &impl EntryProbe,
) -> CleanupPlan {
    let mut plan = CleanupPlan {
        scan_root: tree.root_path().to_path_buf(),
        root_depth: components(tree.root_path().as_os_str().as_encoded_bytes())
            .map_or(0, |parts| parts.len()),
        ..CleanupPlan::default()
    };
    let refuse = |plan: &mut CleanupPlan, node: platter_core::NodeId, reason: Refusal| {
        plan.refused.push(Refused {
            rule_id: MARKED,
            path: display(&tree.path_bytes(node)),
            reason,
        });
    };

    let mut nodes: Vec<platter_core::NodeId> = marked
        .iter()
        .copied()
        .filter(|&node| tree.contains(node))
        .collect();
    nodes.sort_unstable();
    nodes.dedup();

    let protected = Protected::for_environment(env).map(|mut protected| {
        protected.learn_identities(probe);
        protected
    });
    let root = owned(tree.root_path().as_os_str().as_encoded_bytes());
    let (Some(protected), Some(root)) = (protected, root.clone()) else {
        let reason = if root.is_none() {
            Refusal::BadPath
        } else {
            Refusal::HomeUnknown
        };
        for node in nodes {
            refuse(&mut plan, node, reason.clone());
        }
        return plan;
    };

    if scan_is_kept(tree, env, rules) {
        for node in nodes {
            refuse(&mut plan, node, Refusal::Kept);
        }
        return plan;
    }

    // Everything a keep rule protects, as chains of ancestors, to compare with each marked entry.
    let kept: Vec<Vec<platter_core::NodeId>> = matches(tree, env, rules)
        .into_iter()
        .filter(|rule| rule.verdict == Verdict::Keep)
        .flat_map(|rule| rule.nodes)
        .map(|node| tree.ancestry(node))
        .collect();

    let guard = Guard {
        tree,
        root,
        protected: &protected,
        probe,
    };
    let mut group = PlanGroup {
        rule_id: MARKED,
        verdict: Verdict::Review,
        action: CleanupAction::Trash,
        targets: Vec::new(),
        allocated: 0,
    };

    for node in nodes {
        let chain = tree.ancestry(node);
        let touches_kept = kept.iter().any(|kept_chain| {
            chain.contains(kept_chain.last().unwrap_or(&node)) || kept_chain.contains(&node)
        });
        if touches_kept {
            refuse(&mut plan, node, Refusal::Kept);
            continue;
        }

        match guard.verify(node, Verdict::Review, false, &[]) {
            Ok(target) => {
                group.allocated = group.allocated.saturating_add(target.allocated());
                group.targets.push(target);
            }
            Err(reason) => refuse(&mut plan, node, reason),
        }
    }

    if !group.targets.is_empty() {
        plan.groups.push(group);
    }
    drop_nested(&mut plan);
    plan.allocated = plan
        .groups
        .iter()
        .fold(0u64, |sum, group| sum.saturating_add(group.allocated));
    plan
}

/// The rules never match inside a match, so this finds nothing; it is checked anyway, because acting on
/// a folder and on something inside it would act twice. The inner one is refused.
fn drop_nested(plan: &mut CleanupPlan) {
    let mut all: Vec<(Vec<Vec<u8>>, usize, usize)> = Vec::new();
    for (g, group) in plan.groups.iter().enumerate() {
        for (t, target) in group.targets.iter().enumerate() {
            if let Some(parts) = components(target.path()) {
                all.push((parts.into_iter().map(<[u8]>::to_vec).collect(), g, t));
            }
        }
    }
    all.sort();

    let mut nested: Vec<(usize, usize)> = Vec::new();
    let mut outer: Option<&Vec<Vec<u8>>> = None;
    for (parts, g, t) in &all {
        match outer {
            Some(o) if is_within(parts, o) => nested.push((*g, *t)),
            _ => outer = Some(parts),
        }
    }

    nested.sort_unstable_by(|a, b| b.cmp(a));
    for (g, t) in nested {
        let group = &mut plan.groups[g];
        let target = group.targets.remove(t);
        group.allocated = group.allocated.saturating_sub(target.allocated());
        plan.refused.push(Refused {
            rule_id: group.rule_id,
            path: target.display_path(),
            reason: Refusal::Nested,
        });
    }
    plan.groups.retain(|group| !group.targets.is_empty());
}
