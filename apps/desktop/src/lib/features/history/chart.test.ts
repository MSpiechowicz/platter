import { describe, expect, it } from 'vitest';
import type { HistoryCleanupDto, HistorySampleDto } from '../../api/types';
import {
  CHART,
  dayNumber,
  dayText,
  layoutChart,
  LABEL_GAP,
  niceStep,
  spreadLabels,
  tickLabels,
  ticksBetween,
} from './chart';

const GB = 1_000_000_000;

const sample = (day: string, used: number): HistorySampleDto => ({ day, used: used * GB, total: 2000 * GB });

const cleanup = (id: number, day: string, freed: number): HistoryCleanupDto => ({
  id,
  at: 0,
  day,
  what: ['Cache'],
  freed: freed * GB,
  methods: [{ kind: 'deleted' }],
  status: 'permanent',
  restorable: 0,
});

describe('the axis', () => {
  it('uses a step of 1, 2 or 5 times a power of ten', () => {
    expect(niceStep(0, 100 * GB, GB)).toBe(20 * GB);
    expect(niceStep(1200 * GB, 1600 * GB, GB)).toBe(100 * GB);
    expect(niceStep(0, 7 * GB, GB)).toBe(2 * GB);
  });

  it('lists the multiples of the step inside the range', () => {
    expect(ticksBetween(1150 * GB, 1620 * GB, 100 * GB)).toEqual(
      [1200, 1300, 1400, 1500, 1600].map((v) => v * GB),
    );
  });

  it('writes the ticks in GB when the size format would repeat itself', () => {
    const size = (bytes: number): string => `${(bytes / 1e12).toFixed(2)} TB`;
    const values = [1405, 1410, 1415].map((v) => v * GB);

    expect(tickLabels(values, size, GB, 'GB')).toEqual(['1405 GB', '1410 GB', '1415 GB']);
    expect(tickLabels([1.2e12, 1.3e12], size, GB, 'GB')).toEqual(['1.20 TB', '1.30 TB']);
  });
});

describe('spreadLabels', () => {
  it('leaves labels that have room where they are', () => {
    expect(spreadLabels([40, 120], LABEL_GAP, 0, 300)).toEqual([40, 120]);
  });

  it('moves labels that are too close apart, keeping their order and the order of the input', () => {
    const [low, high] = spreadLabels([110, 100], LABEL_GAP, 0, 300);

    expect(high).toBeLessThan(low as number);
    expect((low as number) - (high as number)).toBeGreaterThanOrEqual(LABEL_GAP - 1e-6);
  });

  it('keeps the labels inside the range', () => {
    const [first, second] = spreadLabels([5, 8], LABEL_GAP, 0, 300);

    expect(first).toBeGreaterThanOrEqual(0);
    expect((second as number) - (first as number)).toBeGreaterThanOrEqual(LABEL_GAP - 1e-6);

    const [top, bottom] = spreadLabels([295, 298], LABEL_GAP, 0, 300);

    expect(bottom).toBeLessThanOrEqual(300);
    expect((bottom as number) - (top as number)).toBeGreaterThanOrEqual(LABEL_GAP - 1e-6);
  });
});

describe('day numbers', () => {
  it('count whole days and come back to the same text', () => {
    expect(dayNumber('2026-03-01') - dayNumber('2026-02-28')).toBe(1);
    expect(dayNumber('2026-11-02') - dayNumber('2026-10-25')).toBe(8);
    expect(dayText(dayNumber('2026-11-17'))).toBe('2026-11-17');
  });
});

describe('layoutChart', () => {
  const days = ['2026-11-10', '2026-11-11', '2026-11-12', '2026-11-13'] as const;

  it('has nothing to draw without a sample', () => {
    expect(layoutChart({ samples: [], without: [], cleanups: [] }, GB)).toBeUndefined();
  });

  it('draws one line and no gap when nothing was saved', () => {
    const model = layoutChart(
      {
        samples: days.map((day, index) => sample(day, 1400 + index)),
        without: [1400, 1401, 1402, 1403].map((v) => v * GB),
        cleanups: [],
      },
      GB,
    );

    expect(model?.saved).toBe(0);
    expect(model?.savedBracket).toBeUndefined();
    expect(model?.markers).toEqual([]);
    expect(model?.used.x).toBe(CHART.right);
  });

  it('steps down on the day space was freed, and counts what was saved at the right end', () => {
    // 50 GB freed on the third day: without keeps growing, the disk drops.
    const model = layoutChart(
      {
        samples: [sample(days[0], 1400), sample(days[1], 1401), sample(days[2], 1352), sample(days[3], 1353)],
        without: [1400, 1401, 1402, 1403].map((v) => v * GB),
        cleanups: [cleanup(1, days[2], 50)],
      },
      GB,
    );

    expect(model?.saved).toBe(50 * GB);

    // The line goes to where it would have been, then straight down, at the same x.
    const points = [...(model?.usedPath.matchAll(/[ML]([\d.]+),([\d.]+)/g) ?? [])].map((m) => [
      Number(m[1]),
      Number(m[2]),
    ]);
    expect(points).toHaveLength(5);
    expect(points[2]?.[0]).toBe(points[3]?.[0]);
    expect(points[3]?.[1]).toBeGreaterThan(points[2]?.[1] as number);

    expect(model?.markers).toHaveLength(1);
    expect(model?.markers[0]?.x).toBeCloseTo(points[3]?.[0] as number, 0);
    expect(model?.savedBracket?.x).toBe(CHART.right);
  });

  it('puts the two end labels apart even when the lines end close together', () => {
    const model = layoutChart(
      {
        samples: [sample(days[0], 1400), sample(days[3], 1399)],
        without: [1400, 1401].map((v) => v * GB),
        cleanups: [],
      },
      GB,
    );

    expect(Math.abs((model?.without.labelY ?? 0) - (model?.used.labelY ?? 0))).toBeGreaterThanOrEqual(
      LABEL_GAP - 1e-6,
    );
  });

  it('places days by date, so a day with no sample leaves a gap', () => {
    const model = layoutChart(
      {
        samples: [sample('2026-11-10', 1400), sample('2026-11-11', 1401), sample('2026-11-20', 1410)],
        without: [1400, 1401, 1410].map((v) => v * GB),
        cleanups: [],
      },
      GB,
    );
    const xs = [...(model?.usedPath.matchAll(/[ML]([\d.]+),/g) ?? [])].map((m) => Number(m[1]));

    expect((xs[1] ?? 0) - (xs[0] ?? 0)).toBeCloseTo((CHART.right - CHART.left) / 10, 0);
    expect(xs[2]).toBe(CHART.right);
  });

  it('draws a single sample as a dot at the right end, and a flat line inside the plot', () => {
    const one = layoutChart({ samples: [sample(days[0], 1400)], without: [1400 * GB], cleanups: [] }, GB);

    expect(one?.used.x).toBe(CHART.right);
    expect(one?.ticks.length).toBeGreaterThan(1);

    const flat = layoutChart(
      { samples: days.map((day) => sample(day, 1408)), without: days.map(() => 1408 * GB), cleanups: [] },
      GB,
    );

    expect(flat?.used.y).toBeGreaterThan(CHART.top);
    expect(flat?.used.y).toBeLessThan(CHART.bottom);
  });

  it('writes dates under the axis, none on the last day', () => {
    const many = Array.from({ length: 90 }, (_, index) =>
      sample(dayText(dayNumber('2026-08-20') + index), 1400 + index),
    );
    const model = layoutChart({ samples: many, without: many.map((s) => s.used), cleanups: [] }, GB);

    expect(model?.dates[0]?.day).toBe('2026-08-20');
    expect(model?.dates.length).toBeLessThanOrEqual(7);
    expect(model?.dates.every((date) => date.x < CHART.right)).toBe(true);
  });
});
