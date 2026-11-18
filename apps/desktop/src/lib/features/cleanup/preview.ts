import type {
  CleanupActionKind,
  CleanupOptions,
  CleanupPlanDto,
  PlanGroupDto,
  SuggestionDto,
} from '../../api/types';

/*
 * What the "What happens when you clean" card says about a preview plan: how the selection splits by what happens
 * to it, and one row per plan group. Plain values in and out, no stores.
 */

/** What becomes of the bytes: they go to the trash, they are gone for good, or a tool of their own decides. */
export type Outcome = 'trash' | 'deleted' | 'tool';

/** The order the bar, the legend and the table list them in. */
export const OUTCOMES: readonly Outcome[] = ['trash', 'deleted', 'tool'];

export const OUTCOME_NAMES: Record<Outcome, string> = {
  trash: 'Moved to the trash',
  deleted: 'Deleted for good',
  tool: 'Cleaned by its own tool',
};

/** The one thing to know about each outcome: whether it can be undone. */
export const OUTCOME_NOTES: Record<Outcome, string> = {
  trash: 'Undo from History for 30 days',
  deleted: 'Cannot be undone',
  tool: 'The tool decides what it removes',
};

export function outcomeOf(action: CleanupActionKind): Outcome {
  switch (action) {
    case 'trash':
      return 'trash';
    case 'runTool':
      return 'tool';
    case 'delete':
    case 'emptyFolder':
    case 'emptyTrash':
      return 'deleted';
  }
}

/** What Platter does to the group, in the words of the table. A tool group shows its command instead (see `command`). */
const DOES: Record<Exclude<CleanupActionKind, 'runTool'>, string> = {
  trash: 'Moved to the trash',
  delete: 'Deleted',
  emptyTrash: 'Emptied for good',
  emptyFolder: 'Emptied',
};

export interface OutcomeTotal {
  outcome: Outcome;
  bytes: number;
  items: number;
}

export interface PreviewRow {
  ruleId: string;
  outcome: Outcome;
  title: string;
  /** The suggestion's display path; undefined for a group no suggestion stands behind. */
  path: string | undefined;
  /** What Platter does, in words; for a tool the command is shown instead. */
  does: string;
  /** The command a tool runs; only for a tool group that names one. */
  command: string | undefined;
  needsElevation: boolean;
  /** The suggestion's reason, which says how the item comes back; undefined when no suggestion matches. */
  reason: string | undefined;
  bytes: number;
}

export interface Preview {
  /** All three outcomes, in `OUTCOMES` order, some with no bytes. */
  totals: OutcomeTotal[];
  /** Grouped by outcome in `OUTCOMES` order, the largest first inside each. */
  rows: PreviewRow[];
  items: number;
  bytes: number;
  /** Matches the plan leaves alone, the ones it lists and the ones it only counts. */
  refused: number;
}

function rowOf(group: PlanGroupDto, suggestions: readonly SuggestionDto[]): PreviewRow {
  const suggestion = suggestions.find((item) => item.id === group.ruleId);

  return {
    ruleId: group.ruleId,
    outcome: outcomeOf(group.action),
    title: group.title,
    path: suggestion?.path,
    does: group.action === 'runTool' ? 'Runs its own tool' : DOES[group.action],
    command: group.action === 'runTool' ? group.command : undefined,
    needsElevation: group.needsElevation,
    reason: suggestion?.reason,
    bytes: group.allocated,
  };
}

export function previewOf(plan: CleanupPlanDto, suggestions: readonly SuggestionDto[]): Preview {
  const rows = plan.groups
    .map((group) => ({ row: rowOf(group, suggestions), items: group.items }))
    .sort(
      (a, b) =>
        OUTCOMES.indexOf(a.row.outcome) - OUTCOMES.indexOf(b.row.outcome) || b.row.bytes - a.row.bytes,
    );

  const totals = OUTCOMES.map((outcome) => {
    const own = rows.filter((entry) => entry.row.outcome === outcome);

    return {
      outcome,
      bytes: own.reduce((sum, entry) => sum + entry.row.bytes, 0),
      items: own.reduce((sum, entry) => sum + entry.items, 0),
    };
  });

  return {
    totals,
    rows: rows.map((entry) => entry.row),
    items: totals.reduce((sum, total) => sum + total.items, 0),
    bytes: totals.reduce((sum, total) => sum + total.bytes, 0),
    refused: plan.refused.length + plan.refusedMore,
  };
}

/** What a preview plan is asked for: the selected ids and the options of the Cleanup settings, and one key for both. */
export interface PreviewRequest {
  ids: string[];
  options: CleanupOptions;
  /** Equal for equal requests: a plan made for one key describes every request with the same key. */
  key: string;
}

/** The request for the selected items. `options` is read for the two settings that go into a plan, and no more. */
export function requestOf(items: readonly { id: string }[], options: CleanupOptions): PreviewRequest {
  const ids = items.map((item) => item.id);
  const picked = { keepPackageVersions: options.keepPackageVersions, askForAdmin: options.askForAdmin };

  return { ids, options: picked, key: JSON.stringify([ids, picked.keepPackageVersions, picked.askForAdmin]) };
}

/** The first preview, made while the page was still waiting: the card starts from it and does not ask again. */
export interface FirstPreview {
  key: string;
  /** The plan, or why there is none. */
  plan?: CleanupPlanDto;
  failure?: string;
}
