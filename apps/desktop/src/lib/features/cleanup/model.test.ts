import { describe, expect, it } from 'vitest';
import type { PlanGroupDto, SuggestionDto, Verdict, VolumeDto } from '../../api/types';
import type { GroupState } from '../../state/cleanup.svelte';
import { afterCleanup, basisOf, chartItems, resultBars, runBytes, stepMarks, stepOf } from './model';

const GB = 1_000_000_000;

const volume: VolumeDto = {
  id: 'nvme0n1p2',
  mountPoint: '/',
  label: '',
  device: '/dev/nvme0n1p2',
  fsType: 'btrfs',
  total: 1900 * GB,
  available: 492 * GB,
  removable: false,
};

function suggestion(id: string, verdict: Verdict, gb: number): SuggestionDto {
  return {
    id,
    title: `Title ${id}`,
    path: `~/${id}`,
    verdict,
    reason: 'because',
    allocated: gb * GB,
    items: 1,
    nodes: [1],
  };
}

describe('stepOf', () => {
  it('is Scan with no scan or a running one, whatever the cleanup says', () => {
    for (const status of ['idle', 'scanning', 'cancelled', 'failed'] as const) {
      expect(stepOf(status, 'idle')).toBe('scan');
      expect(stepOf(status, 'done')).toBe('scan');
    }
  });

  it('is Review for a ready scan until a cleanup runs, planning and the review dialog included', () => {
    for (const phase of ['idle', 'planning', 'review'] as const) {
      expect(stepOf('ready', phase)).toBe('review');
    }
  });

  it('is Clean while a cleanup runs and after it', () => {
    expect(stepOf('ready', 'running')).toBe('clean');
    expect(stepOf('ready', 'done')).toBe('clean');
  });
});

describe('stepMarks', () => {
  it('marks the steps before the current one as done and the ones after as still to come', () => {
    expect(stepMarks('scan')).toEqual(['current', 'todo', 'todo']);
    expect(stepMarks('review')).toEqual(['done', 'current', 'todo']);
    expect(stepMarks('clean')).toEqual(['done', 'done', 'current']);
  });

  it('ticks all three once the cleanup is over', () => {
    expect(stepMarks('clean', true)).toEqual(['done', 'done', 'done']);
  });
});

describe('numbers', () => {
  it('measures a whole volume by its used space and size, and a folder by what the scan counted', () => {
    expect(basisOf(volume, 5 * GB)).toEqual({ used: 1408 * GB, capacity: 1900 * GB });
    expect(basisOf(undefined, 5 * GB)).toEqual({ used: 5 * GB, capacity: undefined });
  });

  it('takes the selection off the disk, with its share of the volume', () => {
    const after = afterCleanup(basisOf(volume, 0), 252 * GB);

    expect(after.used).toBe(1156 * GB);
    expect(after.share).toBeCloseTo(0.608, 3);
  });

  it('says no share for a folder, and never goes below 0 or above the disk', () => {
    expect(afterCleanup(basisOf(undefined, 100 * GB), 30 * GB)).toEqual({ used: 70 * GB, share: undefined });
    expect(afterCleanup(basisOf(undefined, 100 * GB), 300 * GB).used).toBe(0);
    expect(afterCleanup(basisOf(volume, 0), -5).used).toBe(1408 * GB);
  });

  it('has before and after bars for a volume against its size', () => {
    const bars = resultBars(basisOf(volume, 0), 252 * GB);

    expect(bars.before.share).toBeCloseTo(1408 / 1900, 6);
    expect(bars.after?.used).toBe(1156 * GB);
    expect(bars.after?.share).toBeCloseTo(1156 / 1900, 6);
  });

  it('has a full before bar for a folder, and an after bar of what is left of it', () => {
    const bars = resultBars(basisOf(undefined, 100 * GB), 25 * GB);

    expect(bars.before.share).toBe(1);
    expect(bars.after?.share).toBe(0.75);
  });

  it('has no after bar when the freed bytes were not measured, and one with nothing freed when they were 0', () => {
    expect(resultBars(basisOf(volume, 0), undefined).after).toBeUndefined();
    expect(resultBars(basisOf(volume, 0), 0).after?.used).toBe(1408 * GB);
  });

  it('copes with an empty scan', () => {
    const bars = resultBars(basisOf(undefined, 0), 0);

    expect(bars.before.share).toBe(0);
    expect(bars.after?.share).toBe(0);
  });
});

describe('chartItems', () => {
  const suggestions = [
    suggestion('target', 'safe', 88),
    suggestion('snapshots', 'review', 268),
    suggestion('steam', 'keep', 142),
  ];
  const none = new Set<string>();
  const empty = new Map<string, GroupState>();

  const items = (extra: Partial<Parameters<typeof chartItems>[0]> = {}) =>
    Object.fromEntries(
      chartItems({
        suggestions,
        selected: new Set(['target']),
        cleaned: none,
        groups: empty,
        ran: false,
        locked: false,
        ...extra,
      }).map((item) => [item.id, item]),
    );

  it('puts the selected rules on the freed side, and lets a click move them', () => {
    const result = items();

    expect(result['target']).toMatchObject({ side: 'freed', clickable: true });
    expect(result['snapshots']).toMatchObject({ side: 'stays', clickable: true });
    expect(result['target']?.progress).toBeUndefined();
  });

  it('keeps what is kept on the stay side and never offers it to a click, even when selected', () => {
    expect(items({ selected: new Set(['steam']) })['steam']).toMatchObject({
      side: 'stays',
      clickable: false,
    });
  });

  it('keeps the selection but takes the clicks away while the flow is busy or the scan outdated', () => {
    const result = items({ locked: true });

    expect(result['target']).toMatchObject({ side: 'freed', clickable: false });
    expect(result['snapshots']).toMatchObject({ side: 'stays', clickable: false });
  });

  it('shows a running cleanup by its groups, whatever the selection says', () => {
    const result = items({
      selected: new Set(['snapshots']),
      locked: true,
      groups: new Map<string, GroupState>([
        ['target', 'done'],
        ['snapshots', 'queued'],
      ]),
    });

    expect(result['target']).toMatchObject({ side: 'freed', progress: 'done', clickable: false });
    expect(result['snapshots']).toMatchObject({ side: 'freed', progress: 'queued', clickable: false });
    expect(result['steam']).toMatchObject({ side: 'stays' });
  });

  it('leaves a selected rule on the stay side once a cleanup has run without it', () => {
    const result = items({
      selected: new Set(['target', 'snapshots']),
      ran: true,
      locked: true,
      groups: new Map<string, GroupState>([['target', 'done']]),
    });

    expect(result['target']).toMatchObject({ side: 'freed', progress: 'done' });
    expect(result['snapshots']).toMatchObject({ side: 'stays', clickable: false });
  });

  it('puts a group that had a problem on the freed side, marked, and not clickable', () => {
    const result = items({ locked: true, groups: new Map<string, GroupState>([['target', 'problem']]) });

    expect(result['target']).toMatchObject({ side: 'freed', progress: 'problem', clickable: false });
    expect(result['snapshots']).toMatchObject({ side: 'stays' });
  });

  it('shows a cleaned rule as done even when no run is remembered', () => {
    expect(items({ cleaned: new Set(['target']), locked: true })['target']).toMatchObject({
      side: 'freed',
      progress: 'done',
    });
  });

  it('shows control and bidi characters of a title as text', () => {
    const [shown] = chartItems({
      suggestions: [{ ...suggestion('x', 'safe', 1), title: 'evil‮doc.exe' }],
      selected: none,
      cleaned: none,
      groups: empty,
      ran: false,
      locked: false,
    });

    expect(shown?.label).toBe('evil\\u{202E}doc.exe');
  });
});

const markedPlanGroup: PlanGroupDto = {
  ruleId: 'marked',
  title: 'Marked in the Explorer',
  verdict: 'review',
  action: 'trash',
  items: 3,
  allocated: 40 * GB,
  needsElevation: false,
  samplePaths: [],
};

describe('chartItems with a hand-marked group', () => {
  const suggestions = [suggestion('target', 'safe', 88)];
  const source = (extra: Partial<Parameters<typeof chartItems>[0]> = {}) => ({
    suggestions,
    selected: new Set<string>(),
    cleaned: new Set<string>(),
    groups: new Map<string, GroupState>(),
    marked: markedPlanGroup,
    ran: false,
    locked: false,
    ...extra,
  });

  it('adds nothing without a marked group', () => {
    expect(chartItems(source({ marked: undefined })).map((item) => item.id)).toEqual(['target']);
  });

  it('adds one group of its own, in the review colour, on the freed side and never clickable', () => {
    const items = chartItems(source());

    expect(items.map((item) => item.id)).toEqual(['target', 'marked']);
    expect(items[1]).toEqual({
      id: 'marked',
      label: 'Marked in the Explorer',
      size: 40 * GB,
      verdict: 'review',
      side: 'freed',
      progress: undefined,
      clickable: false,
    });
  });

  it('carries how far the run got with it', () => {
    const groups = new Map<string, GroupState>([['marked', 'done']]);

    expect(chartItems(source({ groups, ran: true }))[1]).toMatchObject({ side: 'freed', progress: 'done' });
  });

  it('leaves it on the stay side when the run ended without reaching it', () => {
    expect(chartItems(source({ ran: true }))[1]).toMatchObject({ side: 'stays', clickable: false });
  });
});

describe('runBytes', () => {
  const suggestions = [
    suggestion('a', 'safe', 10),
    suggestion('b', 'safe', 20),
    suggestion('c', 'review', 40),
  ];
  const groups = new Map<string, GroupState>([
    ['a', 'done'],
    ['c', 'queued'],
  ]);

  it('adds up the rules of the cleanup, finished or not, and only those', () => {
    expect(runBytes(suggestions, groups)).toBe(50 * GB);
  });

  it('counts the hand-marked group with the rules, when it is one of the groups of the run', () => {
    const withMarked = new Map<string, GroupState>([...groups, ['marked', 'done']]);

    expect(runBytes(suggestions, withMarked, undefined, markedPlanGroup)).toBe(90 * GB);
    expect(runBytes(suggestions, withMarked, 'done', markedPlanGroup)).toBe(50 * GB);
    expect(runBytes(suggestions, groups, undefined, markedPlanGroup)).toBe(50 * GB);
    expect(runBytes([], new Map([['marked', 'done' as const]]), undefined, markedPlanGroup)).toBe(40 * GB);
    expect(runBytes([], new Map([['marked', 'done' as const]]))).toBe(0);
  });

  it('can count one state only', () => {
    expect(runBytes(suggestions, groups, 'done')).toBe(10 * GB);
    expect(runBytes(suggestions, groups, 'problem')).toBe(0);
  });
});
