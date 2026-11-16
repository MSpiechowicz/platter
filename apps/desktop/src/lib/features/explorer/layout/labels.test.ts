import { describe, expect, it } from 'vitest';
import { pathHitsRect, rectsOverlap, route, segmentHitsRect, type Point, type Rect } from './geometry';
import { placeLabels, type Bounds, type LabelRequest, type PlacedLabel } from './labels';

const BOUNDS: Bounds = { width: 900, height: 560 };

function request(id: number, x: number, y: number, width = 90, height = 34): LabelRequest {
  return { id, x, y, ax: x, ay: y + 14, width, height };
}

/** Deterministic pseudo-random numbers in [0, 1). */
function random(seed: number): () => number {
  let state = seed;

  return () => {
    state = (state * 16807) % 2147483647;

    return state / 2147483647;
  };
}

/**
 * Labels never overlap and stay inside the card. `labels` adds that no connector passes behind a label;
 * `everything` adds that none passes over a dot either. A very crowded cluster cannot promise that much.
 */
function expectClean(
  placed: PlacedLabel[],
  bounds: Bounds,
  strict: 'overlap' | 'labels' | 'everything' = 'everything',
): void {
  for (const [index, label] of placed.entries()) {
    const [x0, y0, x1, y1] = label.rect;

    expect(x0, `label ${label.id} left`).toBeGreaterThanOrEqual(0);
    expect(y0, `label ${label.id} top`).toBeGreaterThanOrEqual(0);
    expect(x1, `label ${label.id} right`).toBeLessThanOrEqual(bounds.width);
    expect(y1, `label ${label.id} bottom`).toBeLessThanOrEqual(bounds.height);

    for (const other of placed.filter((_, at) => at !== index)) {
      expect(rectsOverlap(label.rect, other.rect), `labels ${label.id} and ${other.id} overlap`).toBe(false);

      if (strict !== 'overlap') {
        expect(
          pathHitsRect(label.path, other.rect),
          `connector of ${label.id} crosses label ${other.id}`,
        ).toBe(false);
      }

      if (strict === 'everything') {
        const dot: Rect = [other.dot[0] - 3, other.dot[1] - 3, other.dot[0] + 3, other.dot[1] + 3];

        expect(pathHitsRect(label.path, dot), `connector of ${label.id} crosses dot of ${other.id}`).toBe(
          false,
        );
      }
    }
  }
}

describe('geometry helpers', () => {
  it('finds overlapping rectangles, with or without air between them', () => {
    expect(rectsOverlap([0, 0, 10, 10], [5, 5, 15, 15])).toBe(true);
    expect(rectsOverlap([0, 0, 10, 10], [10, 0, 20, 10])).toBe(false);
    expect(rectsOverlap([0, 0, 10, 10], [12, 0, 20, 10], 3)).toBe(true);
  });

  it('finds a segment through a rectangle, also when both ends are outside', () => {
    const box: Rect = [10, 10, 20, 20];

    expect(segmentHitsRect([0, 15], [30, 15], box)).toBe(true);
    expect(segmentHitsRect([15, 0], [15, 30], box)).toBe(true);
    expect(segmentHitsRect([0, 0], [30, 30], box)).toBe(true);
    expect(segmentHitsRect([0, 25], [30, 25], box)).toBe(false);
    expect(segmentHitsRect([25, 0], [25, 30], box)).toBe(false);
    expect(segmentHitsRect([0, 0], [8, 30], box)).toBe(false);
  });
});

describe('route', () => {
  const rect: Rect = [100, 100, 200, 130];

  it('goes down, across, down into the dot', () => {
    expect(route(rect, [260, 220])).toEqual([
      [150, 130],
      [150, 139],
      [260, 139],
      [260, 220],
    ]);
  });

  it('is a single vertical line when the dot is right below', () => {
    expect(route(rect, [151, 200])).toEqual([
      [150, 130],
      [151, 200],
    ]);
  });

  it('never runs diagonally, even when the dot is close or higher than the label', () => {
    for (const dot of [
      [300, 135],
      [300, 100],
      [20, 131],
      [150, 90],
    ] as Point[]) {
      const path = route(rect, dot);

      for (const [index, point] of path.slice(1).entries()) {
        const from = path[index] as Point;

        expect(
          from[0] === point[0] || from[1] === point[1] || Math.abs(from[0] - point[0]) < 2,
          `${dot}`,
        ).toBe(true);
      }
    }
  });
});

describe('placeLabels', () => {
  it('puts a lone label straight above its dot with a straight connector', () => {
    const [placed] = placeLabels([request(1, 400, 300)], BOUNDS);

    expect(placed?.rect[0]).toBeCloseTo(355, 6);
    expect(placed?.rect[3]).toBeCloseTo(300 - 16, 6);
    expect(placed?.path).toHaveLength(2);
  });

  it('moves a label that would cover another to the nearest free spot, keeping both inside', () => {
    const placed = placeLabels([request(1, 400, 300), request(2, 410, 304)], BOUNDS);

    expectClean(placed, BOUNDS);
    expect(placed[0]?.rect[1]).toBeCloseTo(300 - 16 - 34, 6);
    expect(placed[1]?.rect).not.toEqual([365, 254, 455, 288]);
  });

  it('keeps a label inside the card when its column is near an edge', () => {
    const placed = placeLabels([request(1, 20, 300), request(2, 890, 300), request(3, 450, 20)], BOUNDS);

    expectClean(placed, BOUNDS);
  });

  it('separates a crowd of nine labels, like the mock-up, from each other and from every connector', () => {
    const next = random(42);

    for (let round = 0; round < 30; round++) {
      const requests = Array.from({ length: 9 }, (_, id) =>
        request(id, 150 + next() * 600, 200 + next() * 250, 60 + next() * 90, 34),
      );
      const placed = placeLabels(requests, BOUNDS);

      expect(placed).toHaveLength(9);
      expectClean(placed, BOUNDS, 'labels');
    }
  });

  it('copes with a full block: 16 labels over a 4 by 4 grid of columns', () => {
    const requests = Array.from({ length: 16 }, (_, id) =>
      request(id, 300 + (id % 4) * 90 + (Math.floor(id / 4) % 2) * 30, 260 + Math.floor(id / 4) * 40, 90, 34),
    );

    expectClean(placeLabels(requests, BOUNDS), BOUNDS, 'labels');
  });

  it('never overlaps two labels even when the dots are almost on top of each other', () => {
    const next = random(7);
    const requests = Array.from({ length: 24 }, (_, id) =>
      request(id, 380 + next() * 140, 250 + next() * 100, 70 + next() * 60, 34),
    );

    expectClean(placeLabels(requests, BOUNDS), BOUNDS, 'overlap');
  });

  it('keeps labels where they were while the block turns, and does so cheaply', () => {
    const base = Array.from({ length: 16 }, (_, id) =>
      request(id, 300 + (id % 4) * 90 + (Math.floor(id / 4) % 2) * 30, 260 + Math.floor(id / 4) * 40, 90, 34),
    );
    const shifted = (frame: number): LabelRequest[] =>
      base.map((label) => ({ ...label, x: label.x + frame, ax: label.ax + frame }));

    let previous = placeLabels(base, BOUNDS);
    let moved = 0;
    const started = performance.now();

    for (let frame = 1; frame <= 20; frame++) {
      const hints = new Map(previous.map((label) => [label.id, label.hint]));
      const placed = placeLabels(shifted(frame), BOUNDS, hints);

      moved += placed.filter(
        (label, index) => label.hint.offset.join() !== previous[index]?.hint.offset.join(),
      ).length;
      previous = placed;
    }

    // A few labels may swap places as the block shifts; the rest stay where they are.
    expect(moved).toBeLessThan(10);
    expect((performance.now() - started) / 20).toBeLessThan(5);
  });

  it('answers the first frame of a crowded block in a fraction of a second', () => {
    const next = random(11);
    const requests = Array.from({ length: 16 }, (_, id) =>
      request(id, 300 + next() * 300, 250 + next() * 150, 70 + next() * 60, 34),
    );
    const started = performance.now();

    placeLabels(requests, BOUNDS);

    expect(performance.now() - started).toBeLessThan(250);
  });

  it('gives the first label the spot right above its dot, and the next one a different place', () => {
    const placed = placeLabels([request(1, 400, 300), request(2, 404, 303)], BOUNDS);

    expect(placed[0]?.rect[1]).toBeCloseTo(300 - 16 - 34, 6);
    expect(placed[1]?.rect).not.toEqual(placed[0]?.rect);
    expectClean(placed, BOUNDS);
  });

  it('starts every connector at the middle of its label bottom and ends it on the dot', () => {
    const placed = placeLabels([request(1, 400, 300), request(2, 405, 305), request(3, 395, 310)], BOUNDS);

    for (const label of placed) {
      const first = label.path[0] as Point;
      const last = label.path.at(-1) as Point;

      expect(first[0]).toBeCloseTo((label.rect[0] + label.rect[2]) / 2, 6);
      expect(first[1]).toBeCloseTo(label.rect[3], 6);
      expect(last).toEqual(label.dot);
    }
  });
});

describe('placeLabels: how many, and where', () => {
  const inside = (rect: Rect, bounds: Bounds): boolean =>
    rect[0] >= 0 && rect[1] >= 0 && rect[2] <= bounds.width && rect[3] <= bounds.height;

  it('places at most 10 labels: the first ten asked for, which are the biggest columns', () => {
    const requests = Array.from({ length: 16 }, (_, index) => request(index + 1, 80 + index * 50, 200));
    const placed = placeLabels(requests, BOUNDS);

    expect(placed).toHaveLength(10);
    expect(placed.map((label) => label.id)).toEqual(requests.slice(0, 10).map((label) => label.id));
  });

  it('adds the pinned label (hovered or selected) in place of the last of the ten, so it is still ten', () => {
    const requests = Array.from({ length: 16 }, (_, index) => ({
      ...request(index + 1, 80 + index * 50, 200),
      pinned: index === 13,
    }));
    const ids = placeLabels(requests, BOUNDS).map((label) => label.id);

    expect(ids).toHaveLength(10);
    expect(ids).toContain(14);
    expect(ids).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 14]);
  });

  it('never puts a label outside the box, wherever its column is, even far outside', () => {
    const wild = random(7);
    const requests = Array.from({ length: 30 }, (_, index) =>
      request(index + 1, wild() * 3000 - 1000, wild() * 2000 - 700, 60 + wild() * 120, 30 + wild() * 10),
    );

    for (const label of placeLabels(requests, BOUNDS)) {
      expect(inside(label.rect, BOUNDS), `label ${label.id}`).toBe(true);
    }
  });

  it('keeps a label that would hang off the bottom edge inside', () => {
    const [label] = placeLabels([{ ...request(1, 450, 600), ay: 540 }], BOUNDS);

    expect(label).toBeDefined();
    expect(inside((label as PlacedLabel).rect, BOUNDS)).toBe(true);
  });

  it('leaves out a label whose dot is outside the box, and one that cannot fit in the box at all', () => {
    const placed = placeLabels(
      [
        { ...request(1, 450, 200), ax: -40 },
        request(2, 450, 200, 2000, 34),
        request(3, 450, 200, 90, 900),
        request(4, 300, 200),
      ],
      BOUNDS,
    );

    expect(placed.map((label) => label.id)).toEqual([4]);
  });
});
