import { describe, expect, it } from 'vitest';
import { formatShare, MAX_BOOSTED, minFraction, MIN_SHARE, readableWeights, splitReadable } from './scale';

const GB = 1_000_000_000;

/** The user's disk: 97% in one folder, 1.3%, 1.3% and 0.2% beside it, a 17 MB one and fourteen of a kilobyte. */
const DISK = [1550 * GB, 21 * GB, 21 * GB, 3.6 * GB, 17_000_000, ...Array.from({ length: 14 }, () => 1000)];

/** What the map does with a folder's sizes: folds the dust, then gives the rest their weights. */
function plan(sizes: number[], width: number, height: number) {
  const min = minFraction(width, height);
  const { kept, folded } = splitReadable(sizes, min);
  const dust = folded.reduce((sum, index) => sum + (sizes[index] as number), 0);
  const drawn = [...kept.map((index) => sizes[index] as number), ...(folded.length > 0 ? [dust] : [])];

  return { min, kept, folded, drawn, ...readableWeights(drawn, min) };
}

describe.each([
  [900, 600],
  [1240, 760],
])('the readable minimum on a %i x %i map', (width, height) => {
  const result = plan(DISK, width, height);

  it('gives every folder that is drawn at least the area of a tile that can hold a name and a size', () => {
    expect(result.min * width * height).toBeGreaterThanOrEqual(110 * 46);

    for (const weight of result.weights) {
      expect(weight).toBeGreaterThanOrEqual(result.min - 1e-12);
    }
  });

  it('folds the 17 MB folder and the fourteen tiny ones into one tile, and keeps the four real folders', () => {
    expect(result.kept).toEqual([0, 1, 2, 3]);
    expect(result.folded).toHaveLength(15);
    expect(result.weights).toHaveLength(5);
  });

  it('adds up to the whole map', () => {
    expect(result.weights.reduce((sum, weight) => sum + weight, 0)).toBeCloseTo(1, 9);
  });

  it('never draws a bigger folder smaller than a smaller one', () => {
    const order = result.drawn.map((size, index) => ({ size, weight: result.weights[index] as number }));

    for (const a of order) {
      for (const b of order) {
        if (a.size > b.size) {
          expect(a.weight).toBeGreaterThanOrEqual(b.weight - 1e-12);
        }
      }
    }
  });

  it('marks exactly the folders that are drawn larger than they are', () => {
    // 0.2% and the dust are lifted; 1.3% is already above the minimum on these maps; the big one is scaled down.
    expect(result.boosted).toEqual([false, result.min > 0.013, result.min > 0.013, true, true]);
    expect(result.weights[0]).toBeLessThan(0.97);
  });
});

describe('the scale on a balanced folder', () => {
  it('returns the true shares and flags nothing', () => {
    const sizes = [300, 250, 200, 150, 100];
    const { weights, boosted } = readableWeights(sizes, minFraction(900, 600));

    expect(weights).toEqual([0.3, 0.25, 0.2, 0.15, 0.1]);
    expect(boosted).toEqual([false, false, false, false, false]);
  });

  it('keeps items without a size at nothing', () => {
    const { weights, boosted } = readableWeights([100, 0, 50], 0.01);

    expect(weights[1]).toBe(0);
    expect(boosted[1]).toBe(false);
  });

  it('does not make everything bigger when every folder is small', () => {
    const { weights, boosted } = readableWeights([1, 1, 1, 1], 0.9);

    expect(weights).toEqual([0.25, 0.25, 0.25, 0.25]);
    expect(boosted.some(Boolean)).toBe(false);
  });

  it('spends at most 40% of the map on lifted folders, however small the map is', () => {
    const sizes = [1000, ...Array.from({ length: 8 }, () => 1)];
    const { weights } = readableWeights(sizes, minFraction(300, 200));
    const lifted = weights.slice(1).reduce((sum, weight) => sum + weight, 0);

    expect(lifted).toBeLessThanOrEqual(0.4 + 1e-9);
    expect(weights.reduce((sum, weight) => sum + weight, 0)).toBeCloseTo(1, 9);
  });
});

describe('splitReadable', () => {
  const small = (count: number, share: number): number[] => Array.from({ length: count }, () => share * 1000);

  it('boosts at most eight small folders; the rest go into the "smaller items" tile', () => {
    const sizes = [80_000, ...small(12, 1)];
    const { kept, folded } = splitReadable(sizes, 0.02);

    expect(kept).toHaveLength(1 + MAX_BOOSTED);
    expect(folded).toHaveLength(4);
  });

  it('folds the biggest of the small ones last', () => {
    const { folded } = splitReadable([80_000, 900, 800, 700, 600, 500, 400, 300, 200, 100, 50], 0.02);

    expect(folded).toEqual([9, 10]);
  });

  it('folds anything under 0.05% of the folder, however few there are', () => {
    const { kept, folded } = splitReadable([1_000_000, 600, 100], 0.01);

    expect(kept).toEqual([0, 1]);
    expect(folded).toEqual([2]);
    expect(MIN_SHARE).toBe(0.0005);
  });

  it('never folds the backend\'s own "smaller items" node, which already is that tile', () => {
    const { kept, folded } = splitReadable([1_000_000, 100, 50], 0.01, 2);

    expect(kept).toContain(2);
    expect(folded).toEqual([1]);
  });

  it('leaves out items without a size', () => {
    expect(splitReadable([10, 0, 5], 0.01).kept).toEqual([0, 2]);
  });
});

describe('formatShare', () => {
  it('reads 97%, 1.3% and under a tenth of a per cent', () => {
    expect(formatShare(0.97)).toBe('97%');
    expect(formatShare(0.013)).toBe('1.3%');
    expect(formatShare(0.0004)).toBe('<0.1%');
    expect(formatShare(0.002)).toBe('0.2%');
  });
});
