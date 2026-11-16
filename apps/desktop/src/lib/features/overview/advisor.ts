import type { SuggestionDto, SuggestionsDto, Verdict } from '../../api/types';

export const VERDICTS: readonly Verdict[] = ['safe', 'review', 'keep'];

/** The group header in the Advisor card. */
export const GROUP_TITLES: Record<Verdict, string> = {
  safe: 'Safe to clean',
  review: 'Needs your review',
  keep: 'Kept',
};

/** The legend row in the Reclaimable card. */
export const LEGEND_LABELS: Record<Verdict, string> = {
  safe: 'Safe to clean',
  review: 'Needs your review',
  keep: 'Kept',
};

/**
 * Colour of a verdict in the Reclaimable bar. Green is only for "safe"; what is kept is neutral.
 * (The Advisor's group header paints "kept" red, as in the mockup.)
 */
export const VERDICT_COLORS: Record<Verdict, string> = {
  safe: 'var(--green)',
  review: 'var(--amber)',
  keep: 'var(--slate)',
};

/** Rows shown under "Biggest wins". */
export const MAX_WINS = 4;

export interface SuggestionGroup {
  verdict: Verdict;
  title: string;
  suggestions: SuggestionDto[];
  /** Bytes the suggestions of this group cover. */
  allocated: number;
}

/** The non-empty groups in the order safe, review, keep. Each keeps the order the API gave (largest first). */
export function groupSuggestions(data: SuggestionsDto): SuggestionGroup[] {
  return VERDICTS.map((verdict) => {
    const suggestions = data.suggestions.filter((suggestion) => suggestion.verdict === verdict);

    return {
      verdict,
      title: GROUP_TITLES[verdict],
      suggestions,
      allocated: suggestions.reduce((sum, suggestion) => sum + suggestion.allocated, 0),
    };
  }).filter((group) => group.suggestions.length > 0);
}

/** "1 suggestion", "6 suggestions", "No suggestions". */
export function suggestionCount(count: number): string {
  if (count === 0) {
    return 'No suggestions';
  }

  return `${count} ${count === 1 ? 'suggestion' : 'suggestions'}`;
}

/** "1 item", "6 items": the count in a group header. */
export function itemCount(count: number): string {
  return `${count} ${count === 1 ? 'item' : 'items'}`;
}

export interface ReclaimSegment {
  verdict: Verdict;
  label: string;
  color: string;
  allocated: number;
  /** Share of the scanned space, 0 to 1. The three shares never add up to more than 1. */
  share: number;
}

/**
 * The three rows of the Reclaimable card, always in the order safe, review, kept, with the sizes of `totals`.
 * `scanned` is the space the scan covers; the rest of the bar, what no rule covers, stays empty.
 */
export function reclaimSegments(data: SuggestionsDto | undefined, scanned: number): ReclaimSegment[] {
  const totals = data?.totals ?? { safe: 0, review: 0, keep: 0 };
  const covered = totals.safe + totals.review + totals.keep;
  // The totals never exceed the scan; if they somehow do, the bar still stays inside its track.
  const whole = Math.max(scanned, covered);

  return VERDICTS.map((verdict) => ({
    verdict,
    label: LEGEND_LABELS[verdict],
    color: VERDICT_COLORS[verdict],
    allocated: totals[verdict],
    share: whole > 0 ? totals[verdict] / whole : 0,
  }));
}

/** The largest suggestions that could be cleaned (safe or review), largest first. */
export function biggestWins(data: SuggestionsDto | undefined, limit: number = MAX_WINS): SuggestionDto[] {
  return (data?.suggestions ?? [])
    .filter((suggestion) => suggestion.verdict !== 'keep')
    .map((suggestion, index) => ({ suggestion, index }))
    .sort((a, b) => b.suggestion.allocated - a.suggestion.allocated || a.index - b.index)
    .slice(0, limit)
    .map(({ suggestion }) => suggestion);
}
