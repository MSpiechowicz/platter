import type { CleanupPlanDto, PlanGroupDto } from '../../api/types';
import type { Action, CleanupResult, CleanupStore } from '../../state/cleanup.svelte';
import { fmt } from '../../state/fmt.svelte';
import { platform } from '../../state/platform.svelte';
import { itemCount } from './advisor';
import { shownPath, visibleText } from './display';

/*
 * The words of the cleanup controls. Every string that came from the backend (paths, reasons, messages, tool
 * names) goes through `visibleText` or `shownPath` here, and is only ever rendered as plain text.
 */

export const OUTDATED_TEXT = 'The disk changed. Rescan to clean again.';

/** The tooltip of a figure greyed out because it describes the scan from before a cleanup. */
export const STALE_TEXT = 'From the scan before the cleanup — rescan to update';

/** Details listed under the status line are cut after this many; the rest is counted. */
const MAX_LINES = 5;

export interface FooterStatus {
  /** The sentence that says where things stand. */
  text: string;
  /** More plain lines: refusals, problems, undo failures. */
  details: string[];
  actions: Action[];
  /** Review items are included, or something went wrong. */
  attention: boolean;
}

/** The selection as the footer describes it while nothing runs. */
export function selectionText(store: CleanupStore): string {
  const selected = store.selectedItems.length;

  if (store.cleanable.length === 0) {
    return 'Nothing to clean here.';
  }

  if (selected === 0) {
    return 'Nothing selected.';
  }

  const review = store.reviewSelected.length;

  if (review > 0) {
    return `Includes ${review} review ${review === 1 ? 'item' : 'items'} (${fmt.size(store.reviewBytes)}). You will confirm them first.`;
  }

  return store.choice === 'safe'
    ? 'All safe items, nothing else.'
    : `${selected} safe ${selected === 1 ? 'item' : 'items'} selected.`;
}

/** What cleaning one group does, in plain words: the review dialog says it for every group. */
export function actionText(group: PlanGroupDto): string {
  switch (group.action) {
    case 'delete':
      return 'Deleted for good.';
    case 'emptyFolder':
      return 'Contents deleted for good. The folder stays.';
    case 'emptyTrash':
      return `The ${platform.trash} is emptied for good.`;
    case 'trash':
      return `Moved to ${platform.theTrash}. This can be undone.`;
    case 'runTool': {
      const command = group.command === undefined ? 'a system tool' : visibleText(group.command);

      return group.needsElevation
        ? `Runs ${command}, which asks for ${platform.isWindows ? 'administrator rights' : 'the administrator password'}.`
        : `Runs ${command}.`;
    }
  }
}

const more = (count: number, what: string): string => `… and ${count} more ${what}`;

/**
 * The refusals and the rules that matched nothing: why part of the selection is not in the plan. `refused: false`
 * leaves the refusals out, for the footer: a toast already said how many and why, and paths do not fit there.
 */
export function planNotes(
  plan: CleanupPlanDto | undefined,
  titleOf: (ruleId: string) => string,
  { refused = true }: { refused?: boolean } = {},
): string[] {
  if (!plan) {
    return [];
  }

  if (!refused) {
    return plan.notFound.length > 0 ? [foundNothing(plan, titleOf)] : [];
  }

  const lines = plan.refused
    .slice(0, MAX_LINES)
    .map((item) => `Not cleaned: ${shownPath(item.path)} (${visibleText(item.reason)})`);
  const left = Math.max(0, plan.refused.length - MAX_LINES) + plan.refusedMore;

  if (left > 0) {
    lines.push(more(left, 'not cleaned'));
  }

  if (plan.notFound.length > 0) {
    lines.push(foundNothing(plan, titleOf));
  }

  return lines;
}

function foundNothing(plan: CleanupPlanDto, titleOf: (ruleId: string) => string): string {
  return `Found nothing for: ${plan.notFound.map((id) => visibleText(titleOf(id))).join(', ')}`;
}

/** `freedShown`: the caller writes what was freed as a figure of its own, so the sentence leaves it out. */
function resultText(result: CleanupResult, freedShown = false): string {
  if (result.kind === 'failed') {
    return `Cleaning failed: ${visibleText(result.message ?? 'unknown error')}.`;
  }

  const parts: string[] = [];
  const onlyTrashed = result.trashed > 0 && result.trashed === result.cleaned;

  if (result.cancelled) {
    parts.push('Stopped before everything was cleaned.');
  }

  if (result.freed !== undefined && result.freed > 0) {
    if (!freedShown) {
      parts.push(`Freed ${fmt.size(result.freed)}.`);
    }
  } else if (result.cleaned === 0) {
    parts.push('Nothing was cleaned.');
  } else if (!onlyTrashed && result.freed === 0) {
    parts.push('Cleaned. The free space has not changed yet.');
  } else if (!onlyTrashed) {
    parts.push(`Cleaned ${itemCount(result.cleaned)}.`);
  }

  if (result.trashed > 0) {
    parts.push(`Moved to ${platform.theTrash}: ${itemCount(result.trashed)}.`);

    if (result.undoable) {
      parts.push('This can be undone.');
    }
  }

  if (result.withProblems > 0) {
    parts.push(
      result.withProblems === 1 ? '1 item had a problem.' : `${result.withProblems} items had problems.`,
    );
  }

  if (result.toolsNotRun.length > 0) {
    parts.push(`Not run: ${result.toolsNotRun.map(visibleText).join(', ')}.`);
  }

  if (result.undo && !result.undo.error) {
    parts.push(`Put back ${itemCount(result.undo.restored)}.`);
  }

  return parts.join(' ');
}

function resultDetails(result: CleanupResult): string[] {
  const lines = result.problems
    .slice(0, MAX_LINES)
    .map((problem) => `${shownPath(problem.path)}: ${visibleText(problem.detail)}`);

  if (result.problems.length > MAX_LINES) {
    lines.push(more(result.problems.length - MAX_LINES, 'with problems'));
  }

  const undo = result.undo;

  if (undo?.error) {
    lines.push(`Could not undo: ${visibleText(undo.error)}`);
  }

  for (const failure of undo?.failures.slice(0, MAX_LINES) ?? []) {
    lines.push(`Not put back: ${shownPath(failure.path)} (${visibleText(failure.error)})`);
  }

  if ((undo?.failures.length ?? 0) > MAX_LINES) {
    lines.push(more((undo?.failures.length ?? 0) - MAX_LINES, 'not put back'));
  }

  return lines;
}

export interface FooterOptions {
  /** The view shows what was freed as a big figure: the status does not say it again. */
  freedShown?: boolean;
}

/** What the footer says now, from the flow's state. */
export function footerStatus(store: CleanupStore, options: FooterOptions = {}): FooterStatus {
  const phase = store.phase;
  const notes = planNotes(store.plan, (id) => store.suggestions.find((item) => item.id === id)?.title ?? id, {
    refused: false,
  });
  const actions = store.actions;

  switch (phase.kind) {
    case 'planning':
      return { text: 'Checking what can be cleaned…', details: [], actions, attention: false };

    case 'review':
      return { text: 'Waiting for your answer.', details: notes, actions, attention: true };

    case 'running': {
      const { group, done, total } = phase.progress;
      const shown = Math.min(done, total);
      const text = phase.cancelling
        ? 'Stopping after the item in progress…'
        : group === undefined
          ? phase.cleanupId === undefined
            ? 'Confirm or cancel in the dialog.'
            : 'Starting…'
          : `Cleaning ${visibleText(group)} · ${shown} of ${total}`;

      return { text, details: notes, actions, attention: false };
    }

    case 'done': {
      const text = [resultText(phase.result, options.freedShown), store.outdated ? OUTDATED_TEXT : '']
        .filter(Boolean)
        .join(' ');
      const failed = phase.result.kind === 'failed' || phase.result.withProblems > 0;

      return {
        text,
        details: [...notes, ...resultDetails(phase.result)],
        actions,
        attention: failed,
      };
    }

    case 'idle': {
      if (store.notice !== undefined) {
        return {
          text: [visibleText(store.notice), store.outdated ? OUTDATED_TEXT : ''].filter(Boolean).join(' '),
          details: notes,
          actions,
          attention: false,
        };
      }

      if (store.outdated) {
        return { text: OUTDATED_TEXT, details: notes, actions, attention: false };
      }

      return {
        text: selectionText(store),
        details: [],
        actions,
        attention: store.reviewSelected.length > 0,
      };
    }
  }
}
