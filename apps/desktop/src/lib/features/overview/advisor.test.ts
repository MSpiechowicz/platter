import { describe, expect, it } from 'vitest';
import type { SuggestionDto, SuggestionsDto, Verdict } from '../../api/types';
import { biggestWins, groupSuggestions, itemCount, reclaimSegments, suggestionCount } from './advisor';

const GB = 1_000_000_000;

function suggestion(id: string, verdict: Verdict, gb: number): SuggestionDto {
  return {
    id,
    title: id,
    path: `/${id}`,
    verdict,
    reason: `Because of ${id}.`,
    allocated: gb * GB,
    items: 1,
    nodes: [1],
  };
}

/** What the API sends: safe first, then review, then keep; largest first within each. */
function response(...suggestions: SuggestionDto[]): SuggestionsDto {
  const totals = { safe: 0, review: 0, keep: 0 };

  for (const entry of suggestions) {
    totals[entry.verdict] += entry.allocated;
  }

  return { suggestions, totals };
}

const DATA = response(
  suggestion('target', 'safe', 88),
  suggestion('modules', 'safe', 61),
  suggestion('trash', 'safe', 17),
  suggestion('snapshots', 'review', 268),
  suggestion('docker', 'review', 54),
  suggestion('steam', 'keep', 142),
);

describe('groupSuggestions', () => {
  it('groups by verdict in the order safe, review, kept, with the size and rows of each', () => {
    const groups = groupSuggestions(DATA);

    expect(groups.map((group) => [group.verdict, group.title])).toEqual([
      ['safe', 'Safe to clean'],
      ['review', 'Needs your review'],
      ['keep', 'Kept'],
    ]);
    expect(groups.map((group) => group.suggestions.map((entry) => entry.id))).toEqual([
      ['target', 'modules', 'trash'],
      ['snapshots', 'docker'],
      ['steam'],
    ]);
    expect(groups.map((group) => group.allocated)).toEqual([166 * GB, 322 * GB, 142 * GB]);
  });

  it('leaves out a group that has nothing in it', () => {
    const groups = groupSuggestions(response(suggestion('steam', 'keep', 142)));

    expect(groups.map((group) => group.verdict)).toEqual(['keep']);
    expect(groupSuggestions(response())).toEqual([]);
  });
});

describe('reclaimSegments', () => {
  it('has one segment per verdict, and their sizes are the totals', () => {
    const segments = reclaimSegments(DATA, 1408 * GB);

    expect(segments.map((segment) => [segment.verdict, segment.label, segment.allocated])).toEqual([
      ['safe', 'Safe to clean', DATA.totals.safe],
      ['review', 'Needs your review', DATA.totals.review],
      ['keep', 'Kept', DATA.totals.keep],
    ]);
  });

  it('measures the bar against the scanned space and never overfills it', () => {
    const segments = reclaimSegments(DATA, 1000 * GB);

    expect(segments.map((segment) => segment.share)).toEqual([0.166, 0.322, 0.142]);
    expect(segments.reduce((sum, segment) => sum + segment.share, 0)).toBeLessThanOrEqual(1);

    // Totals that exceed the scan (they cannot, but the bar must not break if they do) still fit the track.
    const overfull = reclaimSegments(DATA, 100 * GB);

    expect(overfull.reduce((sum, segment) => sum + segment.share, 0)).toBeCloseTo(1);
  });

  it('paints only "safe" green', () => {
    const colors = reclaimSegments(DATA, 1000 * GB).map((segment) => segment.color);

    expect(colors.filter((color) => color.includes('green'))).toHaveLength(1);
    expect(colors[0]).toContain('green');
  });

  it('shows zeros, not nothing, when there are no suggestions or they have not arrived', () => {
    for (const data of [response(), undefined]) {
      const segments = reclaimSegments(data, 500 * GB);

      expect(segments).toHaveLength(3);
      expect(segments.map((segment) => segment.allocated)).toEqual([0, 0, 0]);
      expect(segments.map((segment) => segment.share)).toEqual([0, 0, 0]);
    }

    expect(reclaimSegments(response(), 0).map((segment) => segment.share)).toEqual([0, 0, 0]);
  });
});

describe('biggestWins', () => {
  it('lists the largest safe or review suggestions first, and never a kept one', () => {
    expect(biggestWins(DATA).map((entry) => entry.id)).toEqual(['snapshots', 'target', 'modules', 'docker']);
  });

  it('shows at most four, or the number asked for', () => {
    expect(biggestWins(DATA)).toHaveLength(4);
    expect(biggestWins(DATA, 2).map((entry) => entry.id)).toEqual(['snapshots', 'target']);
  });

  it('keeps the given order for equal sizes, and is empty without suggestions', () => {
    const tie = response(suggestion('a', 'safe', 10), suggestion('b', 'review', 10));

    expect(biggestWins(tie).map((entry) => entry.id)).toEqual(['a', 'b']);
    expect(biggestWins(response(suggestion('steam', 'keep', 142)))).toEqual([]);
    expect(biggestWins(undefined)).toEqual([]);
  });
});

describe('counts in words', () => {
  it('says how many suggestions and items there are', () => {
    expect(suggestionCount(0)).toBe('No suggestions');
    expect(suggestionCount(1)).toBe('1 suggestion');
    expect(suggestionCount(6)).toBe('6 suggestions');
    expect(itemCount(1)).toBe('1 item');
    expect(itemCount(3)).toBe('3 items');
  });
});
