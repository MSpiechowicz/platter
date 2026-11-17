import { ApiRequestError, type Api } from './client';
import {
  ADVISOR_ID,
  advisorNodes,
  RULE_IDS,
  suggestionsOf,
  type MockLabel,
  type UserRuleInput,
} from './mock-suggestions';
import { childrenOf, type MockNode, type MockTree } from './mock-tree';
import type {
  CleanupActionKind,
  CleanupEvent,
  CleanupId,
  CleanupOptions,
  CleanupPlanDto,
  OutcomeDto,
  PlanGroupDto,
  PlanId,
  RefusedDto,
  ScanId,
  SuggestionDto,
  UndoDto,
} from './types';

/*
 * The mock's stand-in for the cleanup commands of `src-tauri/src/commands/cleanup.rs`. It never changes
 * anything, it only tells the story of a cleanup over the mock's own suggestions.
 *
 * Still different from the backend:
 * - The mock knows only the up to 20 largest matches of a rule (what `getSuggestions` lists as `nodes`), so a
 *   group has at most 20 items; the backend plans every match. The exception is the `advisor` group, which holds
 *   every review label the mock advisor made (at most 50) that no rule touches.
 * - `advisor` is a cleanable id only while the advisor is switched on and the scan has review labels that the
 *   `advisor` suggestion lists (`invalid` otherwise: off, no review label, or every label left out because a rule
 *   covers or holds it). As in the backend it is checked after an unknown id and an outdated scan. Its group is
 *   `trash`, verdict `review`, planned after the rules' groups. There is no guard to refuse a label, so nothing is
 *   `refused`. A cleanup outdates only the scan it ran on, and drops that scan's labels (the backend outdates every
 *   finished scan and drops all their labels).
 * - The user's rules (`mock-user-rules.ts`) are in the plans as in the backend: `user-safe` is a cleanable id (group
 *   verdict `safe`, action `trash`, never a direct delete), `user-keep` is never planned (its folders come back as
 *   `refused`, like every kept item), and while the rules file is broken every plan is `invalid` before anything
 *   else is looked at. A change of the rules drops the plans that were waiting (`dropPlans`).
 * - It has no disk to guard, so nothing is refused for being protected or changed since the plan. (A marked plan
 *   refuses the scan root, an item a `keep` suggestion covers, holds or lies in, links, mounts, unreadable
 *   folders and an item inside another marked one, as the backend does; the mock only knows the up to 20
 *   largest matches of a `keep` rule.)
 * - Every target succeeds: there are no `skipped` or `failed` outcomes, no problems, and no tool that is missing.
 * - `freed` is the sum of what was deleted, emptied or cleaned by a tool (not what was trashed, which
 *   frees nothing yet), as the mock cannot measure a disk.
 * - Confirmation: the backend asks the user in a native dialog before anything that cannot be undone and rejects
 *   with `cancelled` when declined. A browser has no such dialog, so `startCleanup` simply proceeds. `undoCleanup`
 *   asks the `confirmUndo` hook (default: yes) and rejects with `cancelled` when it says no.
 */

/** The limits of the backend. */
const PLAN_LIFETIME_MS = 10 * 60 * 1000;
const MAX_PLANS = 4;
const MAX_DONE = 4;
const MAX_RULES = 64;
const MAX_MARKED = 1000;
const MAX_REFUSED = 200;
const SAMPLE_PATHS = 20;

const SCAN_ROOT_REASON = 'it is the scanned folder itself';
const KEPT_REASON = 'kept on purpose';
const NOT_CLEANED_REASON = 'Platter does not clean this';
const NESTED_REASON = 'inside another item of the plan';
const SYMLINK_REASON = 'it is a link';
const MOUNT_REASON = 'it is, or lies below, another mounted filesystem';
const UNREADABLE_REASON = 'part of it could not be read';
const EXCLUDED_REASON = 'it holds a folder you left out of scans';

/** `Tree::holds_excluded`: a folder the user left out lies somewhere inside `node`. */
function holdsExcluded(tree: MockTree, node: MockNode): boolean {
  const pending = [...childrenOf(tree, node)];

  for (let next = pending.pop(); next; next = pending.pop()) {
    if (next.kind === 'mount' && next.skipped === 'excluded') {
      return true;
    }

    pending.push(...childrenOf(tree, next));
  }

  return false;
}

/** The rule id and title of what the user marked by hand (`MARKED` in `crates/platter-cleanup/src/plan.rs`). */
const MARKED_RULE = 'marked';
const MARKED_TITLE = 'Marked in the Explorer';

interface Tool {
  program: string;
  args: string[];
  needsElevation: boolean;
}

type Action = { kind: Exclude<CleanupActionKind, 'runTool'> } | { kind: 'runTool'; tool: Tool };

const DELETE: Action = { kind: 'delete' };
const EMPTY_FOLDER: Action = { kind: 'emptyFolder' };
const TRASH: Action = { kind: 'trash' };

const tool = (program: string, args: string[], needsElevation: boolean): Action => ({
  kind: 'runTool',
  tool: { program, args, needsElevation },
});

/** `NEEDS_ADMIN_OFF` of `crates/platter-cleanup`: why an item needing the password is left out. */
const NEEDS_ADMIN_OFF = 'Needs administrator rights, which are turned off in Settings.';

/** The defaults of `CleanupOptions` in `crates/platter-cleanup`. */
const DEFAULT_OPTIONS: CleanupOptions = { keepPackageVersions: 2, askForAdmin: true };

/** The options as `plan_cleanup` checks them: a missing value takes the default, a wrong one is `invalid`. */
function checkOptions(options: Partial<CleanupOptions> | undefined): CleanupOptions {
  const unknown = Object.keys(options ?? {}).find(
    (key) => key !== 'keepPackageVersions' && key !== 'askForAdmin',
  );

  if (unknown !== undefined) {
    return fail('invalid', `${unknown} is not a cleanup option`);
  }

  const keep = options?.keepPackageVersions ?? DEFAULT_OPTIONS.keepPackageVersions;
  const admin = options?.askForAdmin ?? DEFAULT_OPTIONS.askForAdmin;

  if (keep !== 1 && keep !== 2 && keep !== 3) {
    return fail('invalid', 'keepPackageVersions must be 1, 2 or 3');
  }

  if (typeof admin !== 'boolean') {
    return fail('invalid', 'askForAdmin must be true or false');
  }

  return { keepPackageVersions: keep, askForAdmin: admin };
}

/** The pacman cache keeps as many versions as the options say (`action_for` with `CleanupOptions`). */
function withOptions(id: string, action: Action | undefined, options: CleanupOptions): Action | undefined {
  if (id === 'pacman-cache' && action?.kind === 'runTool') {
    return tool('paccache', [`-rk${options.keepPackageVersions}`], true);
  }

  return action;
}

/** `action_for` of `crates/platter-cleanup/src/action.rs`: the rules not listed are not cleaned. */
const ACTIONS: Record<string, Action> = {
  'rust-target': DELETE,
  'rust-target-recent': DELETE,
  'node-modules': DELETE,
  'node-modules-recent': DELETE,
  'python-cache': DELETE,

  'app-cache': EMPTY_FOLDER,
  'npm-cache': EMPTY_FOLDER,
  'bun-cache': EMPTY_FOLDER,
  'pnpm-store': EMPTY_FOLDER,
  'yarn-cache': EMPTY_FOLDER,
  'cargo-registry': EMPTY_FOLDER,
  'gradle-cache': EMPTY_FOLDER,
  'maven-repository': EMPTY_FOLDER,
  'nuget-packages': EMPTY_FOLDER,
  'go-modules': EMPTY_FOLDER,
  'steam-shader-cache': EMPTY_FOLDER,

  trash: { kind: 'emptyTrash' },

  [ADVISOR_ID]: TRASH,
  'user-safe': TRASH,

  'old-downloads': TRASH,
  'disk-images': TRASH,
  'docker-desktop-vm': TRASH,

  coredumps: tool('find', ['/var/lib/systemd/coredump', '-xdev', '-mindepth', '1', '-delete'], true),
  // The count follows the plan's options (`withOptions`).
  'pacman-cache': tool('paccache', ['-rk2'], true),
  'apt-cache': tool('apt-get', ['clean'], true),
  'dnf-cache': tool('dnf', ['clean', 'packages'], true),
  journal: tool('journalctl', ['--vacuum-time=2weeks'], true),
  'docker-data': tool('docker', ['--context', 'default', 'system', 'prune', '--force'], false),
};

/** The groups are planned in the table's order, the model's labels after them. */
const RULE_ORDER = new Map([...RULE_IDS, ADVISOR_ID].map((id, index) => [id, index]));

const fail = (code: 'notFound' | 'invalid' | 'cancelled', message: string): never => {
  throw new ApiRequestError(code, message);
};

const commandOf = (action: Action): string | undefined =>
  action.kind === 'runTool' ? [action.tool.program, ...action.tool.args].join(' ') : undefined;

interface Target {
  path: string;
  allocated: number;
}

interface PlannedGroup {
  ruleId: string;
  action: Action;
  targets: Target[];
}

interface StoredPlan {
  scan: ScanId;
  groups: PlannedGroup[];
  made: number;
}

type Step =
  | { kind: 'start'; group: PlannedGroup }
  | { kind: 'target'; group: PlannedGroup; target: Target }
  | { kind: 'end'; group: PlannedGroup };

interface Running {
  id: CleanupId;
  cancelled: boolean;
}

export interface MockCleanupHost {
  /** The tree of a scan; rejects with `notFound` for an unknown scan. */
  treeOf(scan: ScanId): MockTree;
  pathOf(tree: MockTree, node: MockNode): string;
  /** The mock's scan duration: 0 plays a cleanup out at once, anything else spreads it over that long. */
  durationMs: number;
  /** The labels the mock advisor made in a scan, none while the advisor is switched off (`advisor` is planned from them). */
  labelsOf(scan: ScanId): readonly MockLabel[];
  /** Whether the advisor is switched on; planning `advisor` is `invalid` while it is not. */
  advisorEnabled(): boolean;
  /** The user's rules now: none, and a `problem`, while their file is broken (every plan is then `invalid`). */
  userRules(): { rules: readonly UserRuleInput[]; problem?: string };
  /** Scans a cleanup ran on, shared with the advisor, which refuses to label them. */
  outdated: ReadonlySet<ScanId>;
  /** A cleanup ran on the scan: it is outdated from now on, and its labels are gone. */
  outdate(scan: ScanId): void;
  /** Test hook: the answer of the Undo confirmation dialog (default: confirmed). */
  confirmUndo?: () => boolean;
}

export type MockCleanupApi = Pick<
  Api,
  'planCleanup' | 'planMarkedCleanup' | 'startCleanup' | 'cancelCleanup' | 'undoCleanup'
>;

export type MockCleanup = MockCleanupApi & {
  /** Forgets every plan that was waiting to be started: the user's rules changed, so none is still what they plan. */
  dropPlans(): void;
};

/** The nodes the `keep` suggestions list (up to 20 per rule, the most the mock knows of what a keep rule matches). */
const keptNodes = (suggestions: SuggestionDto[]): Set<number> =>
  new Set(
    suggestions
      .filter((suggestion) => suggestion.verdict === 'keep')
      .flatMap((suggestion) => suggestion.nodes),
  );

/** The node and the folders above it, nearest first. */
function ancestry(tree: MockTree, node: MockNode): MockNode[] {
  const chain: MockNode[] = [node];

  for (let at = node; at.id !== 0;) {
    at = tree.nodes[at.parent] as MockNode;
    chain.push(at);
  }

  return chain;
}

/** A keep rule protects the node, something inside it, or something around it. */
function touchesKept(tree: MockTree, node: MockNode, kept: ReadonlySet<number>): boolean {
  if (ancestry(tree, node).some((above) => kept.has(above.id))) {
    return true;
  }

  return [...kept].some((id) => {
    const inside = tree.nodes[id];

    return inside !== undefined && ancestry(tree, inside).some((above) => above.id === node.id);
  });
}

/** The node lies inside one of `others`. */
const withinAny = (tree: MockTree, node: MockNode, others: ReadonlySet<number>): boolean =>
  ancestry(tree, node)
    .slice(1)
    .some((above) => others.has(above.id));

/** As in the backend, every plan with something to do is confirmed (the mock has no dialog to show). */
const needsConfirmation = (groups: PlannedGroup[]): boolean => groups.length > 0;

export function createMockCleanup(host: MockCleanupHost): MockCleanup {
  const plans = new Map<PlanId, StoredPlan>();
  /** Scans a cleanup has run on: they no longer describe the disk. */
  const { outdated } = host;
  const confirmUndo = host.confirmUndo ?? (() => true);
  /** Finished runs: the paths each moved to the trash, until it is undone. */
  const done = new Map<CleanupId, string[]>();
  let running: Running | null = null;
  let lastPlan: PlanId = 0;
  let lastCleanup: CleanupId = 0;

  const store = (scan: ScanId, groups: PlannedGroup[]): PlanId => {
    const id = ++lastPlan;
    const now = Date.now();

    for (const [other, stored] of plans) {
      if (now - stored.made >= PLAN_LIFETIME_MS) {
        plans.delete(other);
      }
    }

    // Map order is insertion order: the oldest goes first.
    while (plans.size >= MAX_PLANS) {
      plans.delete(plans.keys().next().value as PlanId);
    }

    plans.set(id, { scan, groups, made: now });

    return id;
  };

  /** A broken rules file refuses every plan: nothing then says which folders the user keeps. */
  const checkRulesUsable = (): readonly UserRuleInput[] => {
    const { rules, problem } = host.userRules();

    if (problem !== undefined) {
      fail(
        'invalid',
        `Your rules file cannot be used, so Platter makes no cleanup plan: ${problem} Fix the file, or start over in the Advisor view.`,
      );
    }

    return rules;
  };

  /** The backend's checks for planning `advisor`: the advisor is on, and some review label can be cleaned. */
  const checkAdvisorPlannable = (scan: ScanId, tree: MockTree, user: readonly UserRuleInput[]): void => {
    if (!host.advisorEnabled()) {
      fail('invalid', 'the local model is switched off in the settings');
    }

    const labels = host.labelsOf(scan);

    if (!labels.some((label) => label.verdict === 'review')) {
      fail('invalid', 'the local model has not labelled any folder to clean in this scan');
    }

    if (advisorNodes(tree, labels, user).length === 0) {
      fail('invalid', 'no labelled folder can be cleaned');
    }
  };

  /** What the plan will do, as the wire description (without an id) and as groups to play out. */
  const describe = (
    scan: ScanId,
    ids: string[],
    tree: MockTree,
    user: readonly UserRuleInput[],
    options: CleanupOptions,
  ): { plan: Omit<CleanupPlanDto, 'planId'>; planned: PlannedGroup[] } => {
    const chosen = new Set(ids);
    const leftOut: RefusedDto[] = [];
    const labels = host.labelsOf(scan);
    const { suggestions } = suggestionsOf(tree, labels, user);
    const byRule = new Map(suggestions.map((suggestion) => [suggestion.id, suggestion]));
    const groups: PlanGroupDto[] = [];
    const planned: PlannedGroup[] = [];
    const refused: RefusedDto[] = [];

    const ordered = [...chosen]
      .filter((id) => byRule.has(id))
      .sort((a, b) => (RULE_ORDER.get(a) ?? 0) - (RULE_ORDER.get(b) ?? 0));

    for (const id of ordered) {
      const suggestion = byRule.get(id) as SuggestionDto;
      const action = withOptions(id, ACTIONS[id], options);

      // The labels are all known, so the group takes every one; a rule's matches are the 20 the suggestion lists.
      const nodes =
        id === ADVISOR_ID
          ? advisorNodes(tree, labels, user)
          : suggestion.nodes.map((node) => tree.nodes[node]).filter((node) => node !== undefined);
      const targets: Target[] = [];

      for (const node of nodes) {
        const path = host.pathOf(tree, node);

        if (node.id === 0) {
          refused.push({ ruleId: id, path, reason: SCAN_ROOT_REASON });
        } else if (suggestion.verdict === 'keep') {
          refused.push({ ruleId: id, path, reason: KEPT_REASON });
        } else if (!action) {
          refused.push({ ruleId: id, path, reason: NOT_CLEANED_REASON });
        } else if (node.kind === 'dir' && holdsExcluded(tree, node)) {
          refused.push({ ruleId: id, path, reason: EXCLUDED_REASON });
        } else {
          targets.push({ path, allocated: node.allocated });
        }
      }

      if (!action || targets.length === 0) {
        continue;
      }

      // With administrator rights off, a tool that needs the password is left out at plan time. The backend lists
      // every match of the rule here, before its guard runs. The mock lists the ones its own checks let through.
      if (action.kind === 'runTool' && action.tool.needsElevation && !options.askForAdmin) {
        leftOut.push(
          ...targets.map((target) => ({ ruleId: id, path: target.path, reason: NEEDS_ADMIN_OFF })),
        );
        continue;
      }

      const command = commandOf(action);

      planned.push({ ruleId: id, action, targets });
      groups.push({
        ruleId: id,
        title: suggestion.title,
        verdict: suggestion.verdict,
        action: action.kind,
        items: targets.length,
        allocated: targets.reduce((sum, target) => sum + target.allocated, 0),
        ...(command === undefined ? {} : { command }),
        needsElevation: action.kind === 'runTool' && action.tool.needsElevation,
        samplePaths: targets.slice(0, SAMPLE_PATHS).map((target) => target.path),
      });
    }

    // What was left out comes first, so the cap never hides it.
    const notDone = [...leftOut, ...refused];

    return {
      plan: {
        scanId: scan,
        options,
        groups,
        refused: notDone.slice(0, 200),
        refusedMore: Math.max(0, notDone.length - 200),
        notFound: [...chosen].filter((id) => !byRule.has(id)),
        allocated: groups.reduce((sum, group) => sum + group.allocated, 0),
        needsConfirmation: needsConfirmation(planned),
      },
      planned,
    };
  };

  const stepsOf = (groups: PlannedGroup[]): Step[] =>
    groups.flatMap((group): Step[] => [
      { kind: 'start', group },
      ...group.targets.map((target): Step => ({ kind: 'target', group, target })),
      { kind: 'end', group },
    ]);

  const outcomeOf = (action: Action): OutcomeDto => {
    switch (action.kind) {
      case 'trash':
        return { kind: 'trashed', problems: 0 };
      case 'runTool':
        return { kind: 'byTool', detail: commandOf(action), problems: 0 };
      default:
        return { kind: 'removed', problems: 0 };
    }
  };

  /** Plays the plan out as the backend's executor does: tools are not stopped halfway, other groups stop between targets. */
  const play = (
    cleanup: CleanupId,
    scan: ScanId,
    groups: PlannedGroup[],
    state: Running,
    onEvent: (event: CleanupEvent) => void,
  ): void => {
    const queue = stepsOf(groups);
    const trashed: string[] = [];
    const total = queue.filter((step) => step.kind === 'target').length;
    const delay = host.durationMs > 0 && total > 0 ? host.durationMs / total : 0;
    let cleaned = 0;
    let freed = 0;

    const finish = (cancelled: boolean): void => {
      // Whatever happened, the scan no longer describes the disk.
      host.outdate(scan);
      running = null;
      done.set(cleanup, trashed);

      while (done.size > MAX_DONE) {
        done.delete(done.keys().next().value as CleanupId);
      }

      onEvent({
        type: 'finished',
        cleanupId: cleanup,
        outdatedScan: scan,
        cleaned,
        withProblems: 0,
        freed,
        cancelled,
        toolsNotRun: [],
        canUndo: trashed.length > 0,
      });
    };

    const pump = (): void => {
      for (;;) {
        const step = queue.shift();

        if (!step) {
          finish(false);
          return;
        }

        const isTool = step.group.action.kind === 'runTool';

        if (state.cancelled && ((step.kind === 'start' && isTool) || (step.kind === 'target' && !isTool))) {
          if (step.kind === 'target') {
            onEvent({ type: 'groupFinished', ruleId: step.group.ruleId });
          }

          finish(true);
          return;
        }

        if (step.kind === 'start') {
          onEvent({ type: 'groupStarted', ruleId: step.group.ruleId, targets: step.group.targets.length });
        } else if (step.kind === 'end') {
          onEvent({ type: 'groupFinished', ruleId: step.group.ruleId });
        } else {
          const outcome = outcomeOf(step.group.action);

          cleaned++;

          // Moving to the trash frees nothing until the trash is emptied, as on a real disk.
          if (outcome.kind === 'trashed') {
            trashed.push(step.target.path);
          } else {
            freed += step.target.allocated;
          }

          onEvent({
            type: 'targetDone',
            ruleId: step.group.ruleId,
            path: step.target.path,
            allocated: step.target.allocated,
            outcome,
          });

          if (delay > 0) {
            setTimeout(pump, delay);
            return;
          }
        }
      }
    };

    // Start on the next tick, so the caller has the id before the first event arrives.
    setTimeout(pump, 0);
  };

  /** Why the backend would refuse a marked node, or `undefined` when it would plan it (the mock has no disk to look at). */
  const refusalOf = (tree: MockTree, node: MockNode, kept: ReadonlySet<number>): string | undefined => {
    if (node.id === 0) {
      return SCAN_ROOT_REASON;
    }

    if (touchesKept(tree, node, kept)) {
      return KEPT_REASON;
    }

    if (node.kind === 'symlink') {
      return SYMLINK_REASON;
    }

    if (node.kind === 'mount') {
      return MOUNT_REASON;
    }

    if (node.unreadable) {
      return UNREADABLE_REASON;
    }

    return node.kind === 'dir' && holdsExcluded(tree, node) ? EXCLUDED_REASON : undefined;
  };

  /** What `plan_marked` does with marked node ids, as a description and as the group to play out. */
  const describeMarked = (
    scan: ScanId,
    ids: number[],
    tree: MockTree,
    user: readonly UserRuleInput[],
    options: CleanupOptions,
  ): { plan: Omit<CleanupPlanDto, 'planId'>; planned: PlannedGroup[] } => {
    const kept = keptNodes(suggestionsOf(tree, [], user).suggestions);
    // Outer items first, so an item inside one that is planned is found out.
    const nodes = [...new Set(ids)]
      .map((id) => tree.nodes[id] as MockNode)
      .sort((a, b) => ancestry(tree, a).length - ancestry(tree, b).length || a.id - b.id);
    const refused: RefusedDto[] = [];
    const targets: Target[] = [];
    const planned = new Set<number>();

    for (const node of nodes) {
      const path = host.pathOf(tree, node);
      const reason = refusalOf(tree, node, kept);

      if (reason !== undefined) {
        refused.push({ ruleId: MARKED_RULE, path, reason });
      } else if (withinAny(tree, node, planned)) {
        refused.push({ ruleId: MARKED_RULE, path, reason: NESTED_REASON });
      } else {
        planned.add(node.id);
        targets.push({ path, allocated: node.allocated });
      }
    }

    const groups: PlanGroupDto[] = [];
    const stored: PlannedGroup[] = [];

    if (targets.length > 0) {
      stored.push({ ruleId: MARKED_RULE, action: TRASH, targets });
      groups.push({
        ruleId: MARKED_RULE,
        title: MARKED_TITLE,
        verdict: 'review',
        action: 'trash',
        items: targets.length,
        allocated: targets.reduce((sum, target) => sum + target.allocated, 0),
        needsElevation: false,
        samplePaths: targets.slice(0, SAMPLE_PATHS).map((target) => target.path),
      });
    }

    return {
      plan: {
        scanId: scan,
        options,
        groups,
        refused: refused.slice(0, MAX_REFUSED),
        refusedMore: Math.max(0, refused.length - MAX_REFUSED),
        notFound: [],
        allocated: groups.reduce((sum, group) => sum + group.allocated, 0),
        needsConfirmation: needsConfirmation(stored),
      },
      planned: stored,
    };
  };

  return {
    dropPlans() {
      plans.clear();
    },

    async planMarkedCleanup(scan, nodes, options) {
      const checked = checkOptions(options);
      const user = checkRulesUsable();

      if (nodes.length === 0 || nodes.length > MAX_MARKED) {
        return fail('invalid', `mark between 1 and ${MAX_MARKED} items`);
      }

      if (outdated.has(scan)) {
        return fail('invalid', 'the disk changed since this scan, so rescan first');
      }

      const tree = host.treeOf(scan);
      const unknown = nodes.find((node) => !Number.isInteger(node) || tree.nodes[node] === undefined);

      if (unknown !== undefined) {
        return fail('notFound', `node ${unknown} does not exist`);
      }

      const { plan, planned } = describeMarked(scan, nodes, tree, user, checked);

      return { planId: store(scan, planned), ...plan };
    },

    async planCleanup(scan, suggestionIds, options) {
      const checked = checkOptions(options);
      const user = checkRulesUsable();

      if (suggestionIds.length === 0 || suggestionIds.length > MAX_RULES) {
        return fail('invalid', `choose between 1 and ${MAX_RULES} suggestions`);
      }

      const unknown = suggestionIds.find((id) => !RULE_ORDER.has(id));

      if (unknown !== undefined) {
        return fail('invalid', `${unknown} is not a rule`);
      }

      if (outdated.has(scan)) {
        return fail('invalid', 'the disk changed since this scan, so rescan first');
      }

      const tree = host.treeOf(scan);

      if (suggestionIds.includes(ADVISOR_ID)) {
        checkAdvisorPlannable(scan, tree, user);
      }

      const { plan, planned } = describe(scan, suggestionIds, tree, user, checked);

      return { planId: store(scan, planned), ...plan };
    },

    async startCleanup(plan, onEvent) {
      if (running) {
        return fail('invalid', 'a cleanup is already running');
      }

      const stored = plans.get(plan);

      if (!stored) {
        return fail('notFound', `plan ${plan} does not exist or was used`);
      }

      // A plan is used once, whatever comes of it.
      plans.delete(plan);

      if (Date.now() - stored.made >= PLAN_LIFETIME_MS) {
        return fail('invalid', 'the plan has expired, so make a new one');
      }

      if (outdated.has(stored.scan)) {
        return fail('invalid', 'the disk changed since this plan was made, so rescan first');
      }

      if (stored.groups.length === 0) {
        return fail('invalid', 'the plan has nothing to clean');
      }

      // A dropped scan is `invalid` here, as in the backend, not `notFound`.
      try {
        host.treeOf(stored.scan);
      } catch {
        return fail('invalid', 'the disk changed since this plan was made, so rescan first');
      }

      const cleanup = ++lastCleanup;
      const state: Running = { id: cleanup, cancelled: false };

      running = state;
      play(cleanup, stored.scan, stored.groups, state, onEvent);

      return cleanup;
    },

    async cancelCleanup(cleanup) {
      if (running?.id === cleanup) {
        running.cancelled = true;
      }
    },

    async undoCleanup(cleanup): Promise<UndoDto[]> {
      const paths = done.get(cleanup);

      if (!paths) {
        return fail('notFound', `cleanup ${cleanup} cannot be undone`);
      }

      // The backend's native dialog: declined, nothing is put back and Undo stays possible.
      if (!confirmUndo()) {
        return fail('cancelled', 'The undo was not confirmed.');
      }

      done.delete(cleanup);

      return paths.map((path) => ({ path, restored: true }));
    },
  };
}
