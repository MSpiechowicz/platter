//! A cleanup plan: what would be cleaned, how, and what was refused.

use platter_core::dto::Verdict;
use platter_core::{EntryProbe, Environment, ModelLabel, ModelVerdict, NodeId, Os, ScannedRoot, Tree};
use platter_rules::{Named, RuleSet, matches, scan_is_kept, workspace_root};

use crate::action::{CleanupAction, CleanupOptions, action_for};
use crate::guard::{Guard, MarkerGroup, Refusal, VerifiedTarget};
use crate::kept::KeptFolders;
use crate::path::{PathStyle, display};

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

/// Why a tool that needs administrator rights is left out when the user turned that off.
pub const NEEDS_ADMIN_OFF: &str = "Needs administrator rights, which are turned off in Settings.";

/// A matched node that is left out of the plan because of an option, not because the guard refused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    pub rule_id: &'static str,
    /// The path for people to read.
    pub path: String,
    pub reason: &'static str,
}

/// The result of planning. Making it changes nothing on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupPlan {
    pub(crate) groups: Vec<PlanGroup>,
    pub refused: Vec<Refused>,
    /// Matches left out because they need administrator rights and the options turned that off. Nothing
    /// was checked for them.
    pub left_out: Vec<LeftOut>,
    /// Selected ids that are not rules, or rules that matched nothing in this scan.
    pub not_found: Vec<String>,
    /// Sum of all groups' scanned sizes. What cleaning frees can differ.
    pub allocated: u64,
    /// The folder the scan started from, and how many components it has: below it nothing acted on may
    /// cross a mount.
    pub(crate) scan_root: std::path::PathBuf,
    pub(crate) root_depth: usize,
    /// The scan root as the scan opened it. Acting below the root needs it to be the same folder still.
    pub(crate) root_identity: Option<ScannedRoot>,
    /// The OS the plan was made for: it decides the syntax of the paths in it, and what carrying it out
    /// may ask of the disk (nothing is deleted for good on Windows).
    pub(crate) os: Os,
    /// What the user chose when the plan was made.
    pub(crate) options: CleanupOptions,
}

impl Default for CleanupPlan {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            refused: Vec::new(),
            left_out: Vec::new(),
            not_found: Vec::new(),
            allocated: 0,
            scan_root: std::path::PathBuf::new(),
            root_depth: 0,
            root_identity: None,
            os: Os::Other,
            options: CleanupOptions::default(),
        }
    }
}

impl CleanupPlan {
    /// What the user chose when the plan was made. The tools in the groups were picked with these.
    pub fn options(&self) -> &CleanupOptions {
        &self.options
    }

    /// The groups of verified targets, one per selected rule that has any.
    pub fn groups(&self) -> &[PlanGroup] {
        &self.groups
    }

    /// The OS the plan was made for.
    pub fn os(&self) -> Os {
        self.os
    }

    /// The folder the scan started from.
    pub fn scan_root(&self) -> &std::path::Path {
        &self.scan_root
    }

    pub fn targets(&self) -> impl Iterator<Item = &VerifiedTarget> {
        self.groups.iter().flat_map(|group| group.targets.iter())
    }

    /// Adds the groups, refusals and unknown ids of `other`, a plan of the same scan, to this plan. A
    /// target that now lies inside (or is the same as) another target of the merged plan is refused as
    /// nested, so acting on the merged plan never acts twice on one place; the totals are worked out
    /// again. The scan root and the options stay this plan's own, unless this plan has no scan root yet.
    pub fn absorb(&mut self, other: CleanupPlan) {
        if self.scan_root.as_os_str().is_empty() {
            self.scan_root = other.scan_root;
            self.root_depth = other.root_depth;
            self.root_identity = other.root_identity;
            self.os = other.os;
            self.options = other.options;
        }

        self.groups.extend(other.groups);
        self.refused.extend(other.refused);
        self.left_out.extend(other.left_out);
        for id in other.not_found {
            if !self.not_found.contains(&id) {
                self.not_found.push(id);
            }
        }

        self.finish();
    }

    /// Refuses nested targets and adds the groups up.
    fn finish(&mut self) {
        drop_nested(self);
        self.allocated = self
            .groups
            .iter()
            .fold(0u64, |sum, group| sum.saturating_add(group.allocated));
    }
}

/// Plans the cleanup of the rules in `selected` over a finished scan. The rules run again, so every
/// match is planned (a suggestion lists only the largest few), and each match goes through the guard,
/// which looks at the disk through `probe`. Nothing is changed.
///
/// The `options` are fixed into the plan: they choose the arguments of the owning tools, and with
/// `ask_for_admin` off a rule whose tool needs administrator rights is listed in `left_out` instead of
/// being planned.
///
/// The kept folders are learned for this plan alone: see [`KeptFolders::learn`]. A caller that makes more
/// than one plan for one request learns them once and uses [`plan_cleanup_with`].
pub fn plan_cleanup(
    tree: &Tree,
    env: &Environment,
    rules: &RuleSet,
    selected: &[&str],
    probe: &impl EntryProbe,
    options: &CleanupOptions,
) -> CleanupPlan {
    let kept = KeptFolders::learn_for(PathStyle::for_os(env.os), rules, probe);

    plan_cleanup_with(tree, env, rules, selected, probe, &kept, options)
}

/// [`plan_cleanup`] with the kept folders already learned (see [`KeptFolders::learn`]).
pub fn plan_cleanup_with(
    tree: &Tree,
    env: &Environment,
    rules: &RuleSet,
    selected: &[&str],
    probe: &impl EntryProbe,
    kept: &KeptFolders,
    options: &CleanupOptions,
) -> CleanupPlan {
    let style = PathStyle::for_os(env.os);
    let mut plan = CleanupPlan {
        scan_root: tree.root_path().to_path_buf(),
        root_depth: style
            .components(tree.root_path().as_os_str().as_encoded_bytes())
            .map_or(0, |parts| parts.len()),
        root_identity: Some(tree.root_identity()),
        os: env.os,
        options: *options,
        ..CleanupPlan::default()
    };
    let found = matches(tree, env, rules);

    for id in selected {
        if !found.iter().any(|rule| rule.id == *id) {
            plan.not_found.push((*id).to_owned());
        }
    }

    let chosen: Vec<_> = found.iter().filter(|rule| selected.contains(&rule.id)).collect();
    let protected = kept.protected_places(env, probe);
    let root = style.owned(tree.root_path().as_os_str().as_encoded_bytes());

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

    // Where a keep link leads is left out when that would hold every plan (`/`, the home folder or
    // above); otherwise a scan that starts in it is a scan of a kept folder.
    let kept = kept.anchored(env.home.as_deref());
    // Folders learned in another syntax than this machine's say nothing about it: nothing is planned.
    let reason = if kept.unresolved() || kept.style() != style {
        Some(Refusal::KeptUnresolved)
    } else if kept.holds_scan(tree.root_path()) {
        Some(Refusal::Kept)
    } else {
        None
    };
    if let Some(reason) = reason {
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
    }

    let guard = Guard {
        style,
        tree,
        root,
        protected: &protected,
        kept: &kept,
        probe,
    };

    for rule in chosen {
        let Some(action) = action_for(rule.id, options, env.os) else {
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

        // A tool that needs administrator rights is left out when the user turned that off. This is
        // decided here, at plan time, and not again when the plan runs.
        if let CleanupAction::RunTool(tool) = action
            && tool.needs_elevation()
            && !options.ask_for_admin
        {
            for &node in &rule.nodes {
                plan.left_out.push(LeftOut {
                    rule_id: rule.id,
                    path: display(&tree.path_bytes(node)),
                    reason: NEEDS_ADMIN_OFF,
                });
            }
            continue;
        }

        // The files that made a name rule match a folder, to be checked again for each one.
        let named = rules
            .rules()
            .iter()
            .find(|known| known.id == rule.id)
            .and_then(|known| match &known.matcher {
                platter_rules::Matcher::Named(named) => Some(*named),
                _ => None,
            });

        let mut group = PlanGroup {
            rule_id: rule.id,
            verdict: rule.verdict,
            action,
            targets: Vec::new(),
            allocated: 0,
        };
        for &node in &rule.nodes {
            let markers = named.map_or_else(Vec::new, |named| markers_of(tree, style, node, &named));

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

    plan.finish();
    plan
}

/// The files that made the name rule `named` match `node`, as complete paths: the one next to it, the
/// lockfile next to it (or the pnpm workspace files of the folder above it that stand in for one). They
/// are the exact files the rule saw, so the guard and the executor look at the same ones again.
fn markers_of(tree: &Tree, style: PathStyle, node: NodeId, named: &Named) -> Vec<MarkerGroup> {
    let path = tree.path_bytes(node);
    let parent = style.parent(&path);
    let beside = |name: &str| style.child(parent, name.as_bytes());
    let mut groups = Vec::new();

    if let Some(sibling) = named.sibling {
        groups.push(MarkerGroup::file(beside(sibling)));
    }

    if !named.sibling_any.is_empty() {
        let mut alternatives: Vec<Vec<Vec<u8>>> =
            named.sibling_any.iter().map(|name| vec![beside(name)]).collect();

        if let Some(folder) = workspace_root(tree, node, named.workspace_root) {
            let folder = tree.path_bytes(folder);
            alternatives.push(
                named
                    .workspace_root
                    .iter()
                    .map(|name| style.child(&folder, name.as_bytes()))
                    .collect(),
            );
        }

        groups.push(MarkerGroup::one_of(alternatives));
    }

    groups
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
    options: &CleanupOptions,
) -> CleanupPlan {
    let kept = KeptFolders::learn_for(PathStyle::for_os(env.os), rules, probe);

    CleanupPlan {
        options: *options,
        ..plan_by_hand(tree, env, rules, marked, probe, &kept, MARKED)
    }
}

/// The rule id of what the local model labelled `review`.
pub const ADVISOR: &str = "advisor";

/// Plans the cleanup of the folders the local model labelled `review` (`keep` labels are ignored). Like
/// [`plan_marked`]: no rule vouches for them, so each is `review` and goes to the trash, never deleted
/// directly; each goes through the same guard (protected places, links, mounts, freshness) and is refused
/// when a `keep` rule protects it, a folder inside it, or a folder around it. The group is
/// [`ADVISOR`]. Nothing is changed.
///
/// The kept folders are learned for this plan alone; a caller that also makes [`plan_cleanup`] for one
/// request learns them once and uses [`plan_labelled_with`].
pub fn plan_labelled(
    tree: &Tree,
    env: &Environment,
    rules: &RuleSet,
    labels: &[ModelLabel],
    probe: &impl EntryProbe,
    options: &CleanupOptions,
) -> CleanupPlan {
    let kept = KeptFolders::learn_for(PathStyle::for_os(env.os), rules, probe);

    plan_labelled_with(tree, env, rules, labels, probe, &kept, options)
}

/// [`plan_labelled`] with the kept folders already learned (see [`KeptFolders::learn`]).
pub fn plan_labelled_with(
    tree: &Tree,
    env: &Environment,
    rules: &RuleSet,
    labels: &[ModelLabel],
    probe: &impl EntryProbe,
    kept: &KeptFolders,
    options: &CleanupOptions,
) -> CleanupPlan {
    let nodes: Vec<NodeId> = labels
        .iter()
        .filter(|label| label.verdict == ModelVerdict::Review)
        .map(|label| label.node)
        .collect();

    CleanupPlan {
        options: *options,
        ..plan_by_hand(tree, env, rules, &nodes, probe, kept, ADVISOR)
    }
}

/// The plan for folders chosen by hand or by the model, as the one group `group`. These only go to the
/// trash and never run a tool, so the options are kept in the plan and change nothing else.
fn plan_by_hand(
    tree: &Tree,
    env: &Environment,
    rules: &RuleSet,
    chosen: &[NodeId],
    probe: &impl EntryProbe,
    kept_folders: &KeptFolders,
    group: &'static str,
) -> CleanupPlan {
    let style = PathStyle::for_os(env.os);
    let mut plan = CleanupPlan {
        scan_root: tree.root_path().to_path_buf(),
        root_depth: style
            .components(tree.root_path().as_os_str().as_encoded_bytes())
            .map_or(0, |parts| parts.len()),
        root_identity: Some(tree.root_identity()),
        os: env.os,
        ..CleanupPlan::default()
    };
    let refuse = |plan: &mut CleanupPlan, node: NodeId, reason: Refusal| {
        plan.refused.push(Refused {
            rule_id: group,
            path: display(&tree.path_bytes(node)),
            reason,
        });
    };

    let mut nodes: Vec<NodeId> = chosen
        .iter()
        .copied()
        .filter(|&node| tree.contains(node))
        .collect();
    nodes.sort_unstable();
    nodes.dedup();

    let protected = kept_folders.protected_places(env, probe);
    let root = style.owned(tree.root_path().as_os_str().as_encoded_bytes());
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

    // Where a keep link leads is left out when that would hold every plan (`/`, the home folder or
    // above); otherwise a scan that starts in it is a scan of a kept folder.
    let kept_folders = kept_folders.anchored(env.home.as_deref());
    let reason = if kept_folders.unresolved() || kept_folders.style() != style {
        Some(Refusal::KeptUnresolved)
    } else if scan_is_kept(tree, env, rules) || kept_folders.holds_scan(tree.root_path()) {
        Some(Refusal::Kept)
    } else {
        None
    };
    if let Some(reason) = reason {
        for node in nodes {
            refuse(&mut plan, node, reason.clone());
        }
        return plan;
    }

    // Everything a keep rule protects, as chains of ancestors, to compare with each marked entry.
    let kept: Vec<Vec<NodeId>> = matches(tree, env, rules)
        .into_iter()
        .filter(|rule| rule.verdict == Verdict::Keep)
        .flat_map(|rule| rule.nodes)
        .map(|node| tree.ancestry(node))
        .collect();

    let guard = Guard {
        style,
        tree,
        root,
        protected: &protected,
        kept: &kept_folders,
        probe,
    };
    let mut group = PlanGroup {
        rule_id: group,
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
    plan.finish();
    plan
}

/// The rules never match inside a match, so this finds nothing; it is checked anyway, because acting on
/// a folder and on something inside it would act twice. The inner one is refused.
fn drop_nested(plan: &mut CleanupPlan) {
    let style = PathStyle::for_os(plan.os);
    // Sorted as compared (Windows folds case), so that a folder comes right before what is inside it.
    let mut all: Vec<(Vec<Vec<u8>>, usize, usize)> = Vec::new();
    for (g, group) in plan.groups.iter().enumerate() {
        for (t, target) in group.targets.iter().enumerate() {
            if let Some(parts) = style.owned(target.path()) {
                all.push((style.folded(&parts), g, t));
            }
        }
    }
    all.sort();

    let mut nested: Vec<(usize, usize)> = Vec::new();
    let mut outer: Option<&Vec<Vec<u8>>> = None;
    for (parts, g, t) in &all {
        match outer {
            Some(o) if style.is_within(parts, o) => nested.push((*g, *t)),
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
