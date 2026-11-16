import type { SuggestionDto, VolumeDto } from '../../api/types';
import type { CleanupPhase, GroupState } from '../../state/cleanup.svelte';
import type { ScanState } from '../../state/scan.svelte';
import { visibleText } from '../overview/display';
import type { FlowItem } from './flow';

/*
 * What the Cleanup view works out from the stores before it draws anything: which step it is in, the numbers of
 * the hero and the result, and the groups of the flow chart. No document, no stores: plain values in and out.
 */

export type Step = 'scan' | 'review' | 'clean';

export const STEPS: readonly { id: Step; label: string }[] = [
  { id: 'scan', label: 'Scan' },
  { id: 'review', label: 'Review' },
  { id: 'clean', label: 'Clean' },
];

/**
 * No scan, or one still running: Scan. A scan that is ready: Review, until a cleanup runs or has run: Clean. Planning
 * and the review dialog are still Review: nothing has been cleaned yet.
 */
export function stepOf(scan: ScanState['status'], phase: CleanupPhase['kind']): Step {
  if (scan !== 'ready') {
    return 'scan';
  }

  return phase === 'running' || phase === 'done' ? 'clean' : 'review';
}

export type StepMark = 'done' | 'current' | 'todo';

/**
 * How the step bar marks each step when `step` is the current one. Once the cleanup is over (`complete`) there is
 * no current step: all three are ticked, as in the mockup.
 */
export function stepMarks(step: Step, complete = false): StepMark[] {
  if (complete) {
    return STEPS.map(() => 'done');
  }

  const current = STEPS.findIndex((candidate) => candidate.id === step);

  return STEPS.map((_, index) => (index < current ? 'done' : index === current ? 'current' : 'todo'));
}

/**
 * What the chart and the numbers are measured against. For a scan of a whole volume that is the disk's used
 * space, as the Capacity card shows it; for a folder it is what the scan counted, and nothing is said about the
 * disk around it.
 */
export interface Basis {
  /** Bytes in use: the volume's used space, or the scanned size. */
  used: number;
  /** The volume's size when the scan root is a whole volume. */
  capacity: number | undefined;
}

export function basisOf(volume: VolumeDto | undefined, scanned: number): Basis {
  return volume
    ? { used: Math.max(0, volume.total - volume.available), capacity: volume.total }
    : { used: scanned, capacity: undefined };
}

export interface After {
  /** What is left in use once `removed` is gone; never below 0. */
  used: number;
  /** Its share of the volume, 0 to 1; absent for a folder, whose disk is not known here. */
  share: number | undefined;
}

export function afterCleanup(basis: Basis, removed: number): After {
  const used = Math.max(0, basis.used - Math.max(0, removed));

  return {
    used,
    share: basis.capacity && basis.capacity > 0 ? Math.min(1, used / basis.capacity) : undefined,
  };
}

export interface ResultBar {
  used: number;
  /** Width of the bar, 0 to 1. */
  share: number;
}

export interface ResultBars {
  before: ResultBar;
  /** Absent when the backend could not measure what was freed: nothing is guessed. */
  after: ResultBar | undefined;
}

/** The before and after bars of the result, from real numbers only: the basis, and `freed` when it was measured. */
export function resultBars(basis: Basis, freed: number | undefined): ResultBars {
  const whole = basis.capacity && basis.capacity > 0 ? basis.capacity : Math.max(basis.used, 1);
  const bar = (used: number): ResultBar => ({ used, share: Math.min(1, used / whole) });

  return {
    before: bar(basis.used),
    after: freed === undefined ? undefined : bar(afterCleanup(basis, freed).used),
  };
}

/** The slice of the cleanup store the chart is built from. */
export interface ChartSource {
  suggestions: readonly SuggestionDto[];
  selected: ReadonlySet<string>;
  cleaned: ReadonlySet<string>;
  groups: ReadonlyMap<string, GroupState>;
  /** A cleanup runs or has run: the selection no longer says what is freed, its groups do. */
  ran: boolean;
  /** Planning, a review dialog, a run or an outdated scan: the selection cannot be changed now. */
  locked: boolean;
}

/**
 * The groups of the chart. Before a cleanup, a group is on the freed side when its rule is selected, and a click
 * moves it. Once a cleanup runs or has run, the groups it took in are on the freed side, each with how far it got,
 * and the others stay where they are.
 */
export function chartItems(source: ChartSource): FlowItem[] {
  return source.suggestions.map((suggestion): FlowItem => {
    const state =
      source.groups.get(suggestion.id) ?? (source.cleaned.has(suggestion.id) ? 'done' : undefined);
    const common = {
      id: suggestion.id,
      label: visibleText(suggestion.title),
      size: suggestion.allocated,
      verdict: suggestion.verdict,
    };

    if (suggestion.verdict === 'keep') {
      return { ...common, side: 'stays', clickable: false };
    }

    if (state !== undefined) {
      return { ...common, side: 'freed', progress: state, clickable: false };
    }

    if (source.ran) {
      return { ...common, side: 'stays', clickable: false };
    }

    return {
      ...common,
      side: source.selected.has(suggestion.id) ? 'freed' : 'stays',
      clickable: !source.locked,
    };
  });
}

/** Bytes the groups of the cleanup that runs or ran cover, from the suggestions' own sizes. */
export function runBytes(
  suggestions: readonly SuggestionDto[],
  groups: ReadonlyMap<string, GroupState>,
  only?: GroupState,
): number {
  return suggestions
    .filter((suggestion) => {
      const state = groups.get(suggestion.id);

      return state !== undefined && (only === undefined || state === only);
    })
    .reduce((sum, suggestion) => sum + suggestion.allocated, 0);
}
