import { describe, expect, it } from 'vitest';
import type { HistoryCleanupDto, HistorySampleDto } from '../../api/types';
import { usageDays } from './usageDays';

const GB = 1_000_000_000;

const sample = (day: string, used: number): HistorySampleDto => ({ day, used: used * GB, total: 1000 * GB });

const cleanup = (day: string, freed: number, what: string[] = ['Docker data']): HistoryCleanupDto => ({
  id: 1,
  at: 0,
  day,
  what,
  freed: freed * GB,
  methods: [{ kind: 'deleted' }],
  status: 'permanent',
  restorable: 0,
});

const TODAY = '2026-11-18';

describe('usageDays', () => {
  it('gives one entry for each calendar day of the range, ending today', () => {
    const { days } = usageDays({ samples: [], cleanups: [], today: TODAY }, 30);

    expect(days).toHaveLength(30);
    expect(days[0]?.day).toBe('2026-10-20');
    expect(days.at(-1)?.day).toBe(TODAY);
  });

  it('reads the change of a day against the day before, and calls a rise or no change "grew"', () => {
    const { days, bars } = usageDays(
      {
        samples: [sample('2026-11-15', 100), sample('2026-11-16', 103), sample('2026-11-17', 103)],
        cleanups: [],
        today: '2026-11-17',
      },
      3,
    );

    expect(days.map((day) => day.delta)).toEqual([undefined, 3 * GB, 0]);
    expect(days.map((day) => day.kind)).toEqual([undefined, 'grew', 'grew']);
    expect(bars).toBe(2);
  });

  it('leaves a gap where a day or the day before it has no sample', () => {
    const { days } = usageDays(
      {
        samples: [sample('2026-11-14', 100), sample('2026-11-16', 105), sample('2026-11-17', 106)],
        cleanups: [],
        today: '2026-11-17',
      },
      4,
    );

    // 14th: first sample. 15th: none. 16th: the day before is missing. 17th: a normal day.
    expect(days.map((day) => day.delta)).toEqual([undefined, undefined, undefined, 1 * GB]);
    expect(days.map((day) => day.used)).toEqual([100 * GB, undefined, 105 * GB, 106 * GB].map((v) => v));
  });

  it('tells a fall on a cleanup day from a fall with no cleanup', () => {
    const { days } = usageDays(
      {
        samples: [
          sample('2026-11-14', 100),
          sample('2026-11-15', 70),
          sample('2026-11-16', 60),
          sample('2026-11-17', 65),
        ],
        cleanups: [cleanup('2026-11-15', 30, ['Docker data'])],
        today: '2026-11-17',
      },
      4,
    );

    expect(days.map((day) => day.kind)).toEqual([undefined, 'cleanup', 'shrank', 'grew']);
    expect(days[1]?.freed).toBe(30 * GB);
    expect(days[1]?.what).toEqual(['Docker data']);
    expect(days[2]?.cleanups).toEqual([]);
    expect(days.map((day) => [day.up, day.down])).toEqual([
      [0, 0],
      [0, 30 * GB],
      [0, 10 * GB],
      [5 * GB, 0],
    ]);
  });

  it('splits a cleanup day into what it freed and what grew besides, even when the net is a rise', () => {
    const { days, maxUp, maxDown } = usageDays(
      {
        samples: [sample('2026-11-16', 100), sample('2026-11-17', 110)],
        cleanups: [cleanup('2026-11-17', 25)],
        today: '2026-11-17',
      },
      2,
    );

    expect(days[1]?.delta).toBe(10 * GB);
    expect(days[1]?.kind).toBe('cleanup');
    expect(days[1]?.down).toBe(25 * GB);
    expect(days[1]?.up).toBe(35 * GB);
    expect(maxUp).toBe(35 * GB);
    expect(maxDown).toBe(25 * GB);
  });

  it('still shows a cleanup on a day with no sample before it, as a fall with nothing written', () => {
    const { days, bars, maxDown } = usageDays(
      {
        samples: [sample('2026-11-15', 100), sample('2026-11-17', 90)],
        cleanups: [cleanup('2026-11-17', 20), cleanup('2026-11-16', 3)],
        today: '2026-11-17',
      },
      3,
    );

    expect(days.map((day) => day.delta)).toEqual([undefined, undefined, undefined]);
    expect(days.map((day) => day.kind)).toEqual([undefined, 'cleanup', 'cleanup']);
    expect(days[1]).toMatchObject({ up: 0, down: 3 * GB });
    expect(days[2]).toMatchObject({ up: 0, down: 20 * GB });
    expect(bars).toBe(2);
    expect(maxDown).toBe(20 * GB);
  });

  it('adds the cleanups of one day together and keeps their names', () => {
    const { days } = usageDays(
      {
        samples: [sample('2026-11-16', 100), sample('2026-11-17', 60)],
        cleanups: [
          cleanup('2026-11-17', 10, ['npm cache']),
          cleanup('2026-11-17', 25, ['Rust build output']),
        ],
        today: '2026-11-17',
      },
      2,
    );

    expect(days[1]?.freed).toBe(35 * GB);
    expect(days[1]?.what).toEqual(['npm cache', 'Rust build output']);
  });

  it('totals the net change, what the cleanups freed, and what was added with the rate per month', () => {
    const model = usageDays(
      {
        samples: [
          sample('2026-11-08', 100),
          sample('2026-11-09', 110),
          sample('2026-11-10', 70),
          sample('2026-11-11', 80),
        ],
        cleanups: [cleanup('2026-11-10', 60)],
        today: '2026-11-11',
      },
      4,
    );

    expect(model.net).toBe(-20 * GB);
    expect(model.freed).toBe(60 * GB);
    expect(model.added).toBe(40 * GB);
    expect(model.perMonth).toBeCloseTo(((40 * GB) / 4) * 30);
    // The 10th: 60 GB freed and 20 GB written besides.
    expect(model.maxUp).toBe(20 * GB);
    expect(model.maxDown).toBe(60 * GB);
  });

  it('never reports a negative amount added', () => {
    const model = usageDays(
      { samples: [sample('2026-11-16', 100), sample('2026-11-17', 40)], cleanups: [], today: '2026-11-17' },
      2,
    );

    expect(model.net).toBe(-60 * GB);
    expect(model.added).toBe(0);
    expect(model.perMonth).toBe(0);
  });

  it('counts only what lies inside the range when it is sliced', () => {
    const input = {
      samples: [
        sample('2026-11-10', 100),
        sample('2026-11-11', 150),
        sample('2026-11-16', 160),
        sample('2026-11-17', 161),
      ],
      cleanups: [cleanup('2026-11-11', 5)],
      today: '2026-11-17',
    };

    const wide = usageDays(input, 8);
    const narrow = usageDays(input, 2);

    expect(wide.days).toHaveLength(8);
    expect(wide.net).toBe(61 * GB);
    expect(wide.freed).toBe(5 * GB);
    expect(narrow.days).toHaveLength(2);
    expect(narrow.net).toBe(1 * GB);
    expect(narrow.freed).toBe(0);
    expect(narrow.maxUp).toBe(1 * GB);
  });

  it('has no net change and nothing added when fewer than two days of the range have a sample', () => {
    const model = usageDays(
      { samples: [sample('2026-11-17', 100)], cleanups: [cleanup('2026-11-17', 10)], today: '2026-11-17' },
      5,
    );

    expect(model.net).toBe(0);
    expect(model.added).toBe(0);
    expect(model.perMonth).toBe(0);
    expect(model.freed).toBe(0);
  });

  it('counts as freed only the cleanups inside the net change, so added − freed = net', () => {
    const model = usageDays(
      {
        samples: [sample('2026-11-10', 100), sample('2026-11-11', 104)],
        // One before the first measured day, one on it: neither is in the net change. One on the last day is.
        cleanups: [cleanup('2026-11-08', 50), cleanup('2026-11-10', 7), cleanup('2026-11-11', 3)],
        today: '2026-11-11',
      },
      4,
    );

    expect(model.net).toBe(4 * GB);
    expect(model.freed).toBe(3 * GB);
    expect(model.added).toBe(7 * GB);
    expect(model.added - model.freed).toBe(model.net);
  });

  it('does not count a cleanup that freed nothing as a bar, nor as a cleanup day', () => {
    const gap = usageDays(
      { samples: [sample('2026-11-10', 100)], cleanups: [cleanup('2026-11-11', 0)], today: '2026-11-11' },
      2,
    );

    expect(gap.bars).toBe(0);

    const fall = usageDays(
      {
        samples: [sample('2026-11-10', 100), sample('2026-11-11', 90)],
        cleanups: [cleanup('2026-11-11', 0)],
        today: '2026-11-11',
      },
      2,
    );

    expect(fall.days[1]).toMatchObject({ kind: 'shrank', down: 10 * GB });
  });
});
