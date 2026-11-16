import { describe, expect, it } from 'vitest';
import type { Category, SummaryDto } from '../../api/types';
import { SLATE } from '../../palette';
import { MAX_TYPE_ROWS, typeRows } from './fileTypes';

const ORDER: Category[] = [
  'video',
  'app',
  'archive',
  'image',
  'code',
  'package',
  'document',
  'audio',
  'diskImage',
  'build',
  'other',
];

const entries = (...sizes: number[]): SummaryDto['byCategory'] =>
  sizes.map((allocated, index) => ({ category: ORDER[index] as Category, allocated, files: 1 }));

describe('typeRows', () => {
  it('lists the categories biggest first with their share of the total', () => {
    const rows = typeRows(entries(100, 300, 600));

    expect(rows.map((row) => row.label)).toEqual(['Archives', 'Apps & games', 'Video']);
    expect(rows.map((row) => row.share)).toEqual([0.6, 0.3, 0.1]);
  });

  it('keeps a tiny category as a row of its own', () => {
    const rows = typeRows(entries(1_000_000, 4));

    expect(rows).toHaveLength(2);
    expect(rows[1]?.share).toBeLessThan(0.005);
  });

  it('drops categories with nothing in them', () => {
    expect(typeRows(entries(10, 0))).toHaveLength(1);
  });

  it('folds everything after the eight biggest into one "Smaller types" row at the end', () => {
    const sizes = Array.from({ length: 11 }, (_, index) => 1100 - index * 100);
    const rows = typeRows(entries(...sizes));

    expect(rows).toHaveLength(MAX_TYPE_ROWS + 1);
    expect(rows.slice(0, MAX_TYPE_ROWS).map((row) => row.allocated)).toEqual(sizes.slice(0, MAX_TYPE_ROWS));

    const smaller = rows.at(-1);
    expect(smaller).toMatchObject({ label: 'Smaller types', color: SLATE, allocated: 300 + 200 + 100 });
    expect(smaller?.folded?.map((entry) => entry.label)).toEqual(['Disk images', 'Build artifacts', 'Other']);
  });

  it('keeps the total share at 100% after folding', () => {
    const rows = typeRows(entries(...Array.from({ length: 11 }, (_, index) => 50 - index)));

    expect(rows.reduce((sum, row) => sum + row.share, 0)).toBeCloseTo(1);
  });

  it('does not fold a single category, which would not save a row', () => {
    const rows = typeRows(entries(...Array.from({ length: 9 }, (_, index) => 90 - index)));

    expect(rows).toHaveLength(9);
    expect(rows.some((row) => row.folded)).toBe(false);
  });
});
