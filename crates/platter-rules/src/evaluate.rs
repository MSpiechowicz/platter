//! The single pass: walks the tree top-down once and hands every rule the nodes it matched.

use std::collections::HashSet;
use std::path::Path;

use platter_core::dto::{SuggestionDto, SuggestionSource, SuggestionsDto, Verdict, VerdictTotals};
use platter_core::{Environment, ModelLabel, ModelVerdict, NodeId, NodeKind, Os, Tree};

use crate::display::{join, shorten_home};
use crate::explain::Explainer;
use crate::found::Found;
use crate::plan::{Plan, Role, kept_root, named_of};
use crate::reason::{Facts, render};
use crate::resolve::{has_sibling_file, same_os_str};
use crate::rule::{Files, Matcher, Named, Rule, RuleSet};

/// The result of [`evaluate`]: what could be cleaned, labelled by verdict.
pub type Suggestions = SuggestionsDto;

/// Labels what in `tree` could be cleaned. Pure: reads the tree and the environment, changes nothing.
///
/// One top-down pass. A node that matches a rule is recorded and not entered, so the outermost match
/// wins, nothing inside it is matched again and no byte is counted twice. Unreadable folders, skipped
/// mounts and empty nodes never match (except a `keep` rule's own folder, which is kept whatever node
/// it is: a skipped mount, say, is a match with nothing to show), and a folder with an unreadable part inside, whose size and age
/// are not really known, is never `safe`. When two rules match the same folder, a `keep` rule wins, then the
/// earlier rule. A `keep` folder is never entered, and a scan that starts inside one gets no suggestions.
///
/// Without a known home directory nothing can be kept, so name rules (`target`, `node_modules`, ...) are
/// at most `review`.
///
/// Suggestions come safe first, then review, then keep, largest first within each verdict.
pub fn evaluate(tree: &Tree, env: &Environment, rules: &RuleSet) -> Suggestions {
    evaluate_with(tree, env, rules, &[])
}

/// The id of the suggestion that lists the local model's labels.
pub const ADVISOR_ID: &str = "advisor";

const ADVISOR_TITLE: &str = "Labelled by the local model";

const ADVISOR_REASON: &str = "The local model thinks these folders hold data that can be made again or is only \
     temporary. It can be wrong, so look at each folder before cleaning it.";

/// [`evaluate`], plus one more suggestion for what the local model labelled: id `advisor`, source `model`,
/// verdict `review`. Only `review` labels are listed (`keep` labels never are), and a label whose folder is
/// covered by a rule match, holds one, or lies inside a kept folder is left out, so the model never
/// speaks where a rule already has. A label inside another listed label is left to the outer one, and the
/// scan root, unreadable and empty folders are not listed. A scan that starts inside a kept folder gets no
/// suggestions at all. With no labels this is exactly [`evaluate`].
pub fn evaluate_with(tree: &Tree, env: &Environment, rules: &RuleSet, labels: &[ModelLabel]) -> Suggestions {
    let Some((verdicts, found)) = run(tree, env, rules) else {
        return Suggestions {
            suggestions: Vec::new(),
            totals: VerdictTotals::default(),
        };
    };

    let advisor = advisor_found(tree, rules.rules(), &verdicts, &found, labels);
    assemble(tree, env, rules.rules(), &verdicts, found, advisor)
}

/// What the model's `review` labels cover, after the folders the rules have an opinion on are left out.
fn advisor_found(
    tree: &Tree,
    rules: &[Rule],
    verdicts: &[Verdict],
    found: &[Found],
    labels: &[ModelLabel],
) -> Found {
    let mut advisor = Found::default();
    if labels.is_empty() {
        return advisor;
    }

    let explainer = Explainer::from_found(tree, rules, verdicts, found);
    let mut nodes: Vec<NodeId> = labels
        .iter()
        .filter(|label| label.verdict == ModelVerdict::Review)
        .map(|label| label.node)
        .filter(|&node| listable(tree, &explainer, node))
        .collect();
    nodes.sort_unstable();
    nodes.dedup();

    let listed: HashSet<NodeId> = nodes.iter().copied().collect();
    for node in nodes {
        let ancestry = tree.ancestry(node);
        let inside_another = ancestry[..ancestry.len() - 1]
            .iter()
            .any(|above| listed.contains(above));

        if !inside_another {
            advisor.add(tree, node, None);
        }
    }

    advisor
}

/// Whether a label on `node` may be listed: a readable, non-empty folder below the scan root that no rule
/// covers or holds something of.
fn listable(tree: &Tree, explainer: &Explainer, node: NodeId) -> bool {
    tree.contains(node)
        && tree.kind(node) == NodeKind::Dir
        && tree.parent(node).is_some()
        && tree.allocated(node) > 0
        && !tree.is_unreadable(node)
        && !explainer.touches(node)
}

/// Every node one rule matched, with the verdict the rule has on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleMatches {
    /// The rule's id (see [`Rule::id`]).
    pub id: &'static str,
    pub verdict: Verdict,
    /// All matched nodes, in the order the pass met them (a suggestion lists only the largest few).
    pub nodes: Vec<NodeId>,
}

/// The same pass as [`evaluate`], answered as every matched node per rule instead of display text. For
/// code that has to act on all matches, not show them. Rules that matched nothing are left out.
pub fn matches(tree: &Tree, env: &Environment, rules: &RuleSet) -> Vec<RuleMatches> {
    let Some((verdicts, found)) = run(tree, env, rules) else {
        return Vec::new();
    };

    rules
        .rules()
        .iter()
        .zip(verdicts)
        .zip(found)
        .filter(|(_, found)| !found.is_empty())
        .map(|((rule, verdict), found)| RuleMatches {
            id: rule.id,
            verdict,
            nodes: found.all,
        })
        .collect()
}

/// Whether the scan starts inside a folder a `keep` rule protects: then nothing in it may be cleaned.
pub fn scan_is_kept(tree: &Tree, env: &Environment, rules: &RuleSet) -> bool {
    kept_root(tree, env, rules).is_some()
}

/// The single pass. `None` when the scan starts inside a kept folder, so nothing may be suggested.
pub(crate) fn run(tree: &Tree, env: &Environment, rules: &RuleSet) -> Option<(Vec<Verdict>, Vec<Found>)> {
    if kept_root(tree, env, rules).is_some() {
        return None;
    }

    let plan = Plan::new(tree, env, rules);
    let verdicts: Vec<Verdict> = rules
        .rules()
        .iter()
        .map(|rule| effective_verdict(rule, env))
        .collect();
    let mut walker = Walker {
        tree,
        env,
        rules: rules.rules(),
        verdicts: &verdicts,
        plan,
        found: rules.rules().iter().map(|_| Found::default()).collect(),
        sets: vec![Vec::new()],
    };

    walker.walk();
    let found = walker.found;
    Some((verdicts, found))
}

/// A rule's verdict on this machine: a name rule is not trusted with `safe` when no home directory is
/// known, because then no personal folder can be kept out of its reach.
fn effective_verdict(rule: &Rule, env: &Environment) -> Verdict {
    match (rule.verdict, &rule.matcher) {
        (Verdict::Safe, Matcher::Named(_)) if env.home.is_none() => Verdict::Review,
        (verdict, _) => verdict,
    }
}

/// A run of sibling nodes still to visit, with the file rules that apply to them.
struct Frame {
    next: u32,
    end: u32,
    set: usize,
}

struct Walker<'a> {
    tree: &'a Tree,
    env: &'a Environment,
    rules: &'a [Rule],
    /// The verdict of each rule on this machine (see `effective_verdict`).
    verdicts: &'a [Verdict],
    plan: Plan,
    found: Vec<Found>,
    /// Interned sets of active file rules (rule indexes, ascending). Set 0 is empty.
    sets: Vec<Vec<usize>>,
}

impl Walker<'_> {
    fn walk(&mut self) {
        // Depth-first over runs of siblings: the stack holds one frame per level, not one per node.
        let mut stack = vec![Frame {
            next: 0,
            end: 1,
            set: 0,
        }];

        while let Some(frame) = stack.last_mut() {
            if frame.next == frame.end {
                stack.pop();
                continue;
            }

            let node = NodeId(frame.next);
            let set = frame.set;
            frame.next += 1;

            let Some(inner) = self.visit(node, set) else {
                continue;
            };

            let mut children = self.tree.children(node);
            let count = u32::try_from(children.len()).unwrap_or(0);
            if let Some(first) = children.next() {
                stack.push(Frame {
                    next: first.0,
                    end: first.0 + count,
                    set: inner,
                });
            }
        }
    }

    /// Handles one node. Returns the file-rule set for its children, or `None` when the node was
    /// matched (or has nothing to enter).
    fn visit(&mut self, node: NodeId, set: usize) -> Option<usize> {
        match self.tree.kind(node) {
            NodeKind::Dir => {
                if let Some((rule, name)) = self.matching_rule(node) {
                    // A kept folder is never entered, whatever is in it, and it is kept whatever its size: a
                    // folder of empty files or of hard links charged elsewhere has no bytes of its own (nor
                    // has any folder where the filesystem gives a directory no blocks). Only a folder with
                    // bytes is shown as a suggestion.
                    if self.rules[rule].verdict == Verdict::Keep {
                        if self.tree.allocated(node) > 0 {
                            self.found[rule].add(self.tree, node, name);
                        } else {
                            self.found[rule].add_unlisted(node);
                        }
                        return None;
                    }

                    // A folder with a kept folder inside is entered, not claimed: acting on it would act on
                    // what is kept.
                    if !self.plan.holds_keep(node) && self.is_suggestible(node, self.verdicts[rule]) {
                        self.found[rule].add(self.tree, node, name);
                        return None;
                    }
                }

                Some(self.with_regions(node, set))
            }
            // Not entered, but a kept one (a skipped mount, a link, a file) is still kept: a match with
            // nothing to show, which every question of "is this kept?" sees.
            _ if self.plan.is_anchor(node) => {
                self.note_kept(node);
                None
            }
            NodeKind::File => {
                self.match_file(node, set);
                None
            }
            _ => None,
        }
    }

    /// Records the `keep` rule anchored at a node that is not a directory.
    fn note_kept(&mut self, node: NodeId) {
        let rule = self
            .plan
            .anchors_at(node)
            .iter()
            .find(|anchor| anchor.role == Role::Claim && self.rules[anchor.rule].verdict == Verdict::Keep)
            .map(|anchor| anchor.rule);

        if let Some(rule) = rule {
            self.found[rule].add_unlisted(node);
        }
    }

    /// The first rule, in table order, that matches this directory, with the index of the matched name.
    fn matching_rule(&self, node: NodeId) -> Option<(usize, Option<usize>)> {
        let mut best: Option<(usize, Option<usize>)> = None;

        if self.plan.is_anchor(node) {
            let claims = || {
                self.plan
                    .anchors_at(node)
                    .iter()
                    .filter(|anchor| anchor.role == Role::Claim)
            };
            let keep = claims().find(|anchor| self.rules[anchor.rule].verdict == Verdict::Keep);
            if let Some(anchor) = keep {
                return Some((anchor.rule, None));
            }

            best = claims().next().map(|anchor| (anchor.rule, None));
        }

        // The scan root has no parent in the tree, so a name rule cannot check its surroundings.
        if self.tree.parent(node).is_none() {
            return best;
        }

        for &index in &self.plan.named {
            if best.is_some_and(|(rule, _)| rule < index) {
                break;
            }

            if let Some(named) = named_of(&self.rules[index])
                && let Some(name) = self.named_match(named, node)
            {
                best = Some((index, Some(name)));
                break;
            }
        }

        best
    }

    /// Index of the name in `rule.names` that `node` carries, if the rule's conditions hold.
    fn named_match(&self, rule: &Named, node: NodeId) -> Option<usize> {
        let name = self.tree.name(node);
        let index = rule
            .names
            .iter()
            .position(|candidate| candidate.as_bytes() == name)?;

        if let Some(age) = rule.age
            && !age.holds(self.tree.mtime(node), self.env.now)
        {
            return None;
        }
        if let Some(marker) = rule.sibling
            && !has_sibling_file(self.tree, node, marker)
        {
            return None;
        }
        if !rule.sibling_any.is_empty()
            && !rule
                .sibling_any
                .iter()
                .any(|marker| has_sibling_file(self.tree, node, marker))
        {
            return None;
        }
        if !rule.not_inside.is_empty() || !rule.not_below.is_empty() || !rule.not_below_drive.is_empty() {
            // Only for a folder that passed every cheaper test, so few paths are ever built.
            let path = self.tree.path_of(node);
            let parent = path.parent().unwrap_or(&path);

            if parent.components().any(|component| {
                rule.not_inside
                    .iter()
                    .any(|name| component.as_os_str().as_encoded_bytes() == name.as_bytes())
            }) {
                return None;
            }
            if rule.not_below.iter().any(|below| path.starts_with(below)) {
                return None;
            }
            if !rule.not_below_drive.is_empty()
                && self.env.os == Os::Windows
                && below_windows_system(&path, self.env, rule.not_below_drive)
            {
                return None;
            }
        }

        Some(index)
    }

    /// Unreadable folders have an unknown size and empty ones nothing to offer: neither is suggested. Nor
    /// is a folder with an unreadable folder inside labelled `safe`: its size and age only cover what was
    /// read. Such a folder can still be a `review` item, which the user confirms one by one.
    fn is_suggestible(&self, node: NodeId, verdict: Verdict) -> bool {
        if self.tree.allocated(node) == 0 || self.tree.is_unreadable(node) {
            return false;
        }

        verdict != Verdict::Safe || !self.tree.has_unreadable(node)
    }

    /// The set of file rules active below `node`: the inherited ones plus those anchored here.
    fn with_regions(&mut self, node: NodeId, inherited: usize) -> usize {
        if !self.plan.is_anchor(node) {
            return inherited;
        }

        let mut rules = self.sets[inherited].clone();
        rules.extend(
            self.plan
                .anchors_at(node)
                .iter()
                .filter(|anchor| anchor.role == Role::Region)
                .map(|anchor| anchor.rule),
        );
        rules.sort_unstable();
        rules.dedup();

        if rules == self.sets[inherited] {
            return inherited;
        }

        self.sets.push(rules);
        self.sets.len() - 1
    }

    /// Records the file under the first active file rule it satisfies.
    fn match_file(&mut self, node: NodeId, set: usize) {
        for position in 0..self.sets[set].len() {
            let rule = self.sets[set][position];

            if let Matcher::Files(files) = &self.rules[rule].matcher
                && self.file_matches(files, node)
            {
                self.found[rule].add(self.tree, node, None);
                return;
            }
        }
    }

    fn file_matches(&self, files: &Files, node: NodeId) -> bool {
        let allocated = self.tree.allocated(node);
        if allocated == 0 || allocated < files.min_bytes {
            return false;
        }

        if let Some(category) = files.category
            && self.tree.category(node) != category
        {
            return false;
        }

        files
            .age
            .is_none_or(|age| age.holds(self.tree.mtime(node), self.env.now))
    }
}

/// Turns the matches into suggestions: rendered text, display paths, ordering and totals. `advisor` is
/// what the model's labels cover; it becomes one more review suggestion after all the rules.
fn assemble(
    tree: &Tree,
    env: &Environment,
    rules: &[Rule],
    verdicts: &[Verdict],
    found: Vec<Found>,
    advisor: Found,
) -> Suggestions {
    let mut totals = VerdictTotals::default();
    let mut ranked: Vec<(usize, SuggestionDto)> = Vec::new();

    for (index, (rule, found)) in rules.iter().zip(&found).enumerate() {
        if !found.is_listed() {
            continue;
        }

        let facts = Facts {
            count: found.items,
            days: rule.matcher.age_days(),
            min_bytes: rule.matcher.min_bytes(),
        };
        let verdict = verdicts[index];
        let total = match verdict {
            Verdict::Safe => &mut totals.safe,
            Verdict::Review => &mut totals.review,
            Verdict::Keep => &mut totals.keep,
        };
        *total = total.saturating_add(found.allocated);

        ranked.push((
            index,
            SuggestionDto {
                id: rule.id.to_owned(),
                title: rule.title.to_owned(),
                source: source_of(rule),
                path: display_path(tree, env, rule, found),
                verdict,
                reason: render(rule.reason, &facts),
                allocated: found.allocated,
                items: found.items,
                shared_files: found.shared_files,
                nodes: found.largest_nodes(),
            },
        ));
    }

    if !advisor.is_empty() {
        totals.review = totals.review.saturating_add(advisor.allocated);
        ranked.push((rules.len(), advisor_suggestion(tree, env, &advisor)));
    }

    ranked.sort_by_key(|(index, dto)| (dto.verdict, std::cmp::Reverse(dto.allocated), *index));

    Suggestions {
        suggestions: ranked.into_iter().map(|(_, dto)| dto).collect(),
        totals,
    }
}

/// Who decided a rule: the user for the rules made of their own, otherwise the built-in table.
pub(crate) fn source_of(rule: &Rule) -> SuggestionSource {
    match rule.matcher {
        Matcher::User(_) => SuggestionSource::User,
        _ => SuggestionSource::Rule,
    }
}

fn advisor_suggestion(tree: &Tree, env: &Environment, advisor: &Found) -> SuggestionDto {
    let path = match advisor.single() {
        Some(node) => shorten_home(&tree.path_of(node), env.home.as_deref()),
        None => glob_below(tree, env, advisor, "**"),
    };

    SuggestionDto {
        id: ADVISOR_ID.to_owned(),
        title: ADVISOR_TITLE.to_owned(),
        source: SuggestionSource::Model,
        path,
        verdict: Verdict::Review,
        reason: ADVISOR_REASON.to_owned(),
        allocated: advisor.allocated,
        items: advisor.items,
        shared_files: advisor.shared_files,
        nodes: advisor.largest_nodes(),
    }
}

/// Whether `path` is below `%SystemRoot%` or below one of `folders` at the root of the system drive,
/// compared the way Windows compares names (see `same_windows_name`).
fn below_windows_system(path: &Path, env: &Environment, folders: &[&str]) -> bool {
    let Some(system_root) = env.system_root.as_deref() else {
        return false;
    };

    let below = |base: &Path| {
        let mut path = path.components();
        base.components().all(|part| {
            path.next()
                .is_some_and(|other| same_os_str(other.as_os_str(), part.as_os_str()))
        })
    };
    let drive = system_root.ancestors().last().unwrap_or(system_root);

    below(system_root) || folders.iter().any(|folder| below(&drive.join(folder)))
}

/// The path or pattern shown for a rule, written with the OS's own separator.
fn display_path(tree: &Tree, env: &Environment, rule: &Rule, found: &Found) -> String {
    let shown = shown_path(tree, env, rule, found);

    if env.os == Os::Windows {
        shown.replace('/', "\\")
    } else {
        shown
    }
}

/// The path or pattern shown for a rule: its own path for fixed and home rules, the real path for a
/// single match, otherwise a glob below the deepest folder that holds all matches. Home rules say `~/`.
fn shown_path(tree: &Tree, env: &Environment, rule: &Rule, found: &Found) -> String {
    let home = env.home.as_deref();

    match &rule.matcher {
        Matcher::Fixed(path) => (*path).to_owned(),
        Matcher::Home("") => "~".to_owned(),
        Matcher::Home(relative) => format!("~/{relative}"),
        Matcher::System(relative) => format!("%SystemRoot%/{relative}"),
        Matcher::UserDir(dir) => env.user_dir(*dir).map_or_else(
            || format!("~/{}", dir.default_name()),
            |path| shorten_home(&path, home),
        ),
        Matcher::Named(named) => {
            if let Some(node) = found.single() {
                return shorten_home(&tree.path_of(node), home);
            }

            let pattern = format!("**/{}", name_pattern(named, &found.names));
            glob_below(tree, env, found, &pattern)
        }
        Matcher::Files(_) => {
            if let Some(node) = found.single() {
                return shorten_home(&tree.path_of(node), home);
            }

            glob_below(tree, env, found, "**")
        }
        // Exact folders, so a pattern would claim more than the user chose.
        Matcher::User(_) => match found.single() {
            Some(node) => shorten_home(&tree.path_of(node), home),
            None => format!("{} folders you chose", found.items),
        },
    }
}

fn glob_below(tree: &Tree, env: &Environment, found: &Found, pattern: &str) -> String {
    let Some(ancestor) = found.common_ancestor() else {
        return pattern.to_owned();
    };

    let base = shorten_home(&tree.path_of(ancestor), env.home.as_deref());
    join(&base, pattern)
}

/// The names that really matched: `target`, or `{__pycache__,.mypy_cache}` for several.
fn name_pattern(named: &Named, matched: &[usize]) -> String {
    let mut indexes = matched.to_vec();
    indexes.sort_unstable();

    let names: Vec<&str> = indexes.iter().map(|&index| named.names[index]).collect();
    match names.as_slice() {
        [only] => (*only).to_owned(),
        several => format!("{{{}}}", several.join(",")),
    }
}
