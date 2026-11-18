import { describe, expect, it } from 'vitest';
import type { HistoryCleanupDto } from '../../api/types';
import { cleanedMost, timesText } from './cleanedMost';

const cleanup = (id: number, day: string, what: string[]): HistoryCleanupDto => ({
  id,
  at: Date.parse(`${day}T12:00:00Z`) / 1000,
  day,
  what,
  freed: 1,
  methods: [],
  status: 'permanent',
  restorable: 0,
});

describe('cleanedMost', () => {
  it('ranks the items by how many cleanups had them, and says when each was cleaned last', () => {
    const rows = cleanedMost([
      cleanup(1, '2026-09-01', ['Cache', 'Logs']),
      cleanup(2, '2026-10-01', ['Cache']),
      cleanup(3, '2026-10-29', ['Cache', 'Docker']),
    ]);

    expect(rows.map((row) => [row.name, row.count, row.last])).toEqual([
      ['Cache', 3, '2026-10-29'],
      ['Docker', 1, '2026-10-29'],
      ['Logs', 1, '2026-09-01'],
    ]);
  });

  it('scales the bars to the item cleaned most', () => {
    const rows = cleanedMost([
      cleanup(1, '2026-10-01', ['A', 'B']),
      cleanup(2, '2026-10-02', ['A']),
      cleanup(3, '2026-10-03', ['A']),
      cleanup(4, '2026-10-04', ['A', 'C']),
    ]);

    expect(rows.map((row) => row.share)).toEqual([1, 0.25, 0.25]);
  });

  it('puts the item cleaned last first on a tie, and the name after that', () => {
    const rows = cleanedMost([
      cleanup(1, '2026-10-01', ['Zed', 'Old']),
      cleanup(2, '2026-10-02', ['Beta', 'Alpha']),
    ]);

    expect(rows.map((row) => row.name)).toEqual(['Alpha', 'Beta', 'Old', 'Zed']);
  });

  it('counts an item listed twice in one cleanup once', () => {
    expect(cleanedMost([cleanup(1, '2026-10-01', ['Cache', 'Cache'])])[0]?.count).toBe(1);
  });

  it('is empty when nothing was cleaned', () => {
    expect(cleanedMost([])).toEqual([]);
    expect(cleanedMost([cleanup(1, '2026-10-01', [])])).toEqual([]);
  });
});

describe('timesText', () => {
  it('says once, twice, then the number', () => {
    expect([1, 2, 3, 12].map(timesText)).toEqual(['once', 'twice', '3 times', '12 times']);
  });
});
