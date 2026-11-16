import { describe, expect, it } from 'vitest';
import { COLLAPSED_NODE_ID } from '../../../api/client';
import type { LodNode } from '../../../api/types';
import { leaf, lod, sample, skewed, smaller } from '../fixtures';
import { collapseChains } from './chains';
import {
  BLOCK_TOP,
  capTopLevel,
  COLUMN_HEIGHT,
  GAP,
  layoutColumns,
  MAX_COLUMNS,
  MIN_FOOTPRINT_SIDE,
  type Box,
  type Column,
} from './columns';

const volume = (box: Box): number => (box.x1 - box.x0) * (box.y1 - box.y0) * (box.z1 - box.z0);
const footprint = (column: Column): number => (column.x1 - column.x0) * (column.z1 - column.z0);
const floorsOf = (column: Column): [number, number][] => {
  const floors = new Map<number, number>();

  for (const box of column.boxes.filter((candidate) => !candidate.lid)) {
    floors.set(box.y0, box.y1);
  }

  return [...floors].sort((a, b) => a[0] - b[0]);
};

describe('layoutColumns', () => {
  const layout = layoutColumns(sample([600, 300, 100]));

  it('makes one column per child, biggest first, on a footprint in proportion to its size', () => {
    expect(layout.columns.map((column) => column.node.name)).toEqual(['dir0', 'dir1', 'dir2']);

    const [big, middle, small] = layout.columns.map(footprint) as [number, number, number];

    // The 3-unit gap between columns takes a bigger share from the small ones, so the ratios are loose.
    expect(big / middle).toBeGreaterThan(1.6);
    expect(big / middle).toBeLessThan(2.4);
    expect(middle / small).toBeGreaterThan(2);
    expect(middle / small).toBeLessThan(5);
  });

  it('keeps every column the same height: its floors and the gaps between them add up', () => {
    for (const column of layout.columns) {
      const floors = floorsOf(column);
      const sum = floors.reduce((total, [from, to]) => total + (to - from), 0) + GAP * (floors.length - 1);

      expect(floors).toHaveLength(2);
      expect(sum).toBeCloseTo(COLUMN_HEIGHT, 6);
    }
  });

  it('gives each floor and part a volume in proportion to its size', () => {
    const column = layout.columns[0] as Column;
    const [lower, upper] = floorsOf(column);
    const heightShare =
      ((lower as [number, number])[1] - (lower as [number, number])[0]) /
      ((upper as [number, number])[1] - (upper as [number, number])[0]);

    // dir0 is 600: floor a is 360, floor b is 240.
    expect(heightShare).toBeCloseTo(360 / 240, 6);

    // Parts of floor a: x (240) and y (120) split the floor's length two to one.
    const parts = column.boxes.filter(
      (box) => !box.lid && box.y0 === (lower as [number, number])[0] && box.level === 3,
    );

    expect(parts).toHaveLength(2);
    expect(volume(parts[0] as Box) / volume(parts[1] as Box)).toBeCloseTo(2, 6);
  });

  it('adds one lid over each column and marks levels 1 to 3', () => {
    for (const column of layout.columns) {
      expect(column.boxes.filter((box) => box.lid)).toHaveLength(1);
      expect(column.boxes.every((box) => [1, 2, 3].includes(box.level))).toBe(true);
    }
  });

  it('keeps the columns on the square and apart', () => {
    for (const column of layout.columns) {
      expect(column.x0).toBeGreaterThanOrEqual(-42);
      expect(column.x1).toBeLessThanOrEqual(42);
      expect(column.z0).toBeGreaterThanOrEqual(-42);
      expect(column.z1).toBeLessThanOrEqual(42);
    }

    const [a, b] = layout.columns as [Column, Column];

    expect(a.x1 <= b.x0 || b.x1 <= a.x0 || a.z1 <= b.z0 || b.z1 <= a.z0).toBe(true);
  });

  it('draws a child without children as one plain column', () => {
    const flat = layoutColumns(lod('f', 10, [leaf('a', 6), leaf('b', 4)]));

    for (const column of flat.columns) {
      expect(floorsOf(column)).toHaveLength(1);
      expect(column.boxes.filter((box) => !box.lid)).toHaveLength(1);
      expect(column.boxes.find((box) => !box.lid)?.level).toBe(1);
    }
  });

  it('keeps the "smaller items" node as a column with no floors', () => {
    const withRest = layoutColumns(lod('f', 100, [lod('a', 80, [leaf('x', 80)]), smaller(9, 20)]));
    const rest = withRest.columns.find((column) => column.top === COLLAPSED_NODE_ID) as Column;

    expect(rest.boxes.filter((box) => !box.lid)).toHaveLength(1);
  });

  it('has no columns for an empty folder', () => {
    expect(layoutColumns(lod('f', 0)).columns).toEqual([]);
    expect(layoutColumns(lod('f', 0, [leaf('a', 0)])).columns).toEqual([]);
  });

  it('puts the label anchor above the middle of the lid', () => {
    const column = layout.columns[1] as Column;

    expect(column.anchor.x).toBeCloseTo((column.x0 + column.x1) / 2, 9);
    expect(column.anchor.z).toBeCloseTo((column.z0 + column.z1) / 2, 9);
    expect(column.anchor.y).toBeGreaterThan(COLUMN_HEIGHT);
  });
});

describe('capTopLevel', () => {
  const many = (count: number, extra: LodNode[] = []): LodNode =>
    lod('f', 1000, [
      ...Array.from({ length: count }, (_, index) => leaf(`n${index}`, 100 - index)),
      ...extra,
    ]);

  it('leaves a folder with few enough children alone', () => {
    const root = many(MAX_COLUMNS);

    expect(capTopLevel(root)).toBe(root);
  });

  it('keeps the biggest children and merges the rest into one slate column that holds their size', () => {
    const root = many(MAX_COLUMNS + 6);
    const capped = capTopLevel(root);
    const rest = capped.children.at(-1) as LodNode;

    expect(capped.children).toHaveLength(MAX_COLUMNS);
    expect(rest.node.id).toBe(COLLAPSED_NODE_ID);
    expect(rest.node.name).toBe('7 smaller items');
    expect(capped.children.reduce((sum, child) => sum + child.node.allocated, 0)).toBe(
      root.children.reduce((sum, child) => sum + child.node.allocated, 0),
    );
    expect(capped.children.slice(0, -1).map((child) => child.node.name)).toEqual(
      root.children.slice(0, MAX_COLUMNS - 1).map((child) => child.node.name),
    );
  });

  it('folds the existing "smaller items" node into the merged one', () => {
    const capped = capTopLevel(many(MAX_COLUMNS + 2, [smaller(40, 500)]));
    const rests = capped.children.filter((child) => child.node.id === COLLAPSED_NODE_ID);

    expect(rests).toHaveLength(1);
    expect(rests[0]?.node.name).toBe(`${3 + 40} smaller items`);
  });
});

/** The folder with thousands of tiny things: every column has hundreds of floors and parts. */
function crowded(): LodNode {
  const column = (name: string, size: number, floors: number, parts: number): LodNode =>
    lod(
      name,
      size,
      Array.from({ length: floors }, (_, floor) =>
        lod(
          `${name}-${floor}`,
          size / floors,
          Array.from({ length: parts }, (_, part) => leaf(`${name}-${floor}-${part}`, size / floors / parts)),
        ),
      ),
    );

  return lod('focus', 1000, [
    column('big', 700, 300, 20),
    column('mid', 180, 120, 40),
    column('small', 60, 90, 10),
    column('thin', 30, 400, 5),
    column('dust1', 12, 10, 5),
    column('dust2', 10, 10, 5),
    column('dust3', 8, 10, 5),
  ]);
}

const sides = (box: Box): number[] => [box.x1 - box.x0, box.y1 - box.y0, box.z1 - box.z0];
const apart = (a: Box, b: Box): boolean =>
  a.x1 <= b.x0 || b.x1 <= a.x0 || a.y1 <= b.y0 || b.y1 <= a.y0 || a.z1 <= b.z0 || b.z1 <= a.z0;

describe('layoutColumns on a skewed folder', () => {
  const layout = layoutColumns(skewed());

  it('gives the small folders a column of their own that can be seen, and folds the dust into one', () => {
    const names = layout.columns.map((column) => column.node.name);

    // 97%, 1.3%, 1.3%, 0.2%, 0.05% twice, then four of a few hundredths in the "smaller items" column.
    expect(names.slice(0, 3)).toEqual(['dir0', 'dir1', 'dir2']);
    expect(layout.columns.find((column) => column.top === COLLAPSED_NODE_ID)?.node.name).toBe(
      '4 smaller items',
    );
    expect(layout.omitted).toBe(0);

    for (const column of layout.columns) {
      expect(Math.min(column.x1 - column.x0, column.z1 - column.z0), column.node.name).toBeGreaterThanOrEqual(
        MIN_FOOTPRINT_SIDE,
      );
    }
  });

  it('says which columns are drawn larger than they are, and what their true share is', () => {
    const [big, small] = layout.columns as [Column, Column];

    expect(layout.boosted).toBe(true);
    expect(big.boosted).toBe(false);
    expect(small.boosted).toBe(true);
    expect(small.share).toBeCloseTo(0.013, 3);
  });

  it('draws a bigger folder with a footprint at least as big as a smaller one', () => {
    const ordered = layout.columns.filter((column) => column.top !== COLLAPSED_NODE_ID);

    for (const [index, column] of ordered.entries()) {
      for (const next of ordered.slice(index + 1)) {
        expect(footprint(column) + 1e-6, `${column.node.name} / ${next.node.name}`).toBeGreaterThanOrEqual(
          footprint(next),
        );
      }
    }
  });

  it('boosts at most eight small columns; the rest of them are folded into a "smaller items" column', () => {
    const crowded = layoutColumns(skewed([60, 25, ...Array.from({ length: 12 }, () => 1.25)]));
    const rest = crowded.columns.find((column) => column.top === COLLAPSED_NODE_ID) as Column;

    expect(
      crowded.columns.filter((column) => column.boosted && column.top !== COLLAPSED_NODE_ID),
    ).toHaveLength(8);
    expect(rest.members).toHaveLength(4);
    expect(crowded.omitted).toBe(0);
  });

  it('has no columns, and nothing to omit, for a folder of items without a size', () => {
    expect(layoutColumns(lod('f', 0, [leaf('a', 0), leaf('b', 0)]))).toEqual({
      columns: [],
      omitted: 0,
      boosted: false,
    });
  });

  it('leaves a balanced folder as it is: nothing is drawn larger than it is', () => {
    const balanced = layoutColumns(sample([600, 300, 100]));

    expect(balanced.boosted).toBe(false);
    expect(balanced.columns.every((column) => !column.boosted)).toBe(true);
  });
});

describe('layoutColumns with chains', () => {
  const chained = collapseChains(
    lod('f', 100, [
      lod('home', 97, [lod('me', 97, [lod('a', 60, [leaf('x', 60)]), lod('b', 37, [leaf('y', 37)])])]),
      lod('var', 2, [leaf('z', 2)]),
      lod('opt', 1, [leaf('w', 1)]),
    ]),
  );
  const layout = layoutColumns(chained);

  it('draws a chain as one column, labelled with the joined path, whose floors are the real sub-folders', () => {
    const column = layout.columns.find((candidate) => candidate.node.name === 'home') as Column;
    const floors = new Set(column.boxes.filter((box) => !box.lid).map((box) => box.y0));

    expect(column.label).toBe('home / me');
    expect(column.chain?.map((link) => link.name)).toEqual(['home', 'me']);
    expect(floors.size).toBe(2);
  });

  it('gives the floors of the dominant column colours of their own when it is asked to', () => {
    const framed = layoutColumns(chained, undefined, chained.children[0]?.node.id);
    const column = framed.columns.find((candidate) => candidate.node.name === 'home') as Column;
    const hues = new Set(column.boxes.filter((box) => !box.lid).map((box) => box.hue));

    expect(hues.size).toBe(2);
    expect(column.boxes.find((box) => box.lid)?.hue).toBeUndefined();
    expect(layout.columns[0]?.boxes.every((box) => box.hue === undefined)).toBe(true);
  });

  it('keeps the small columns visible next to the big one, inside the block, without overlaps', () => {
    expect(layout.columns.map((column) => column.node.name)).toEqual(
      expect.arrayContaining(['home', 'var', 'opt']),
    );

    for (const column of layout.columns) {
      expect(Number.isFinite(column.x0 + column.x1 + column.z0 + column.z1)).toBe(true);
      expect(Math.min(column.x1 - column.x0, column.z1 - column.z0)).toBeGreaterThanOrEqual(
        MIN_FOOTPRINT_SIDE,
      );
      expect(Math.abs(column.x0)).toBeLessThanOrEqual(42 + 1e-9);
      expect(Math.abs(column.x1)).toBeLessThanOrEqual(42 + 1e-9);
    }

    const boxes = layout.columns.flatMap((column) => column.boxes);

    for (const [index, a] of boxes.entries()) {
      for (const b of boxes.slice(index + 1)) {
        expect(apart(a, b)).toBe(true);
      }
    }
  });

  it('never makes more columns than the cap', () => {
    const many = lod(
      'f',
      1000,
      Array.from({ length: 40 }, (_, index) => lod(`d${index}`, 25, [leaf(`f${index}`, 25)])),
    );

    expect(layoutColumns(collapseChains(many)).columns.length).toBeLessThanOrEqual(MAX_COLUMNS);
  });
});

describe('the boxes of a crowded block', () => {
  const layout = layoutColumns(crowded());
  const boxes = layout.columns.flatMap((column) => column.boxes.map((box) => ({ column, box })));

  it('has at most 12 floors in a column and 8 parts in a floor', () => {
    expect(layout.columns.length).toBeGreaterThan(1);

    for (const column of layout.columns) {
      const floors = floorsOf(column);

      expect(floors.length, column.node.name).toBeLessThanOrEqual(12);

      for (const [from] of floors) {
        const parts = column.boxes.filter((box) => !box.lid && box.y0 === from);

        expect(parts.length, column.node.name).toBeLessThanOrEqual(8);
      }
    }
  });

  it('puts no floor thinner than 1/40 of the column on its own, and no part under 1/30 of its floor', () => {
    for (const column of layout.columns) {
      const total = floorsOf(column).reduce((sum, [from, to]) => sum + (to - from), 0);
      const [, ...above] = floorsOf(column).reverse();

      for (const [from, to] of above) {
        expect((to - from) / total, column.node.name).toBeGreaterThanOrEqual(1 / 40 - 1e-9);
      }
    }

    const column = layout.columns[0] as Column;
    const floor = column.boxes.filter((box) => !box.lid && box.y0 === floorsOf(column)[0]?.[0]);
    const length = (box: Box): number => Math.max(box.x1 - box.x0, box.z1 - box.z0);
    const along = floor.reduce((sum, box) => sum + length(box), 0);

    for (const box of floor.slice(0, -1)) {
      expect(length(box) / along).toBeGreaterThanOrEqual(1 / 30 - 1e-9);
    }
  });

  it('asks for no edge lines on a box thinner than 0.6 on its smallest side, and for them on the others', () => {
    for (const { box } of boxes) {
      expect(box.edges).toBe(Math.min(...sides(box)) >= 0.6);
    }

    expect(boxes.some(({ box }) => box.edges)).toBe(true);
  });

  it('makes only boxes with finite sides of at least 0.05, inside their own column and the block', () => {
    for (const { column, box } of boxes) {
      expect(sides(box).every((side) => Number.isFinite(side) && side >= 0.05)).toBe(true);
      expect(box.x0).toBeGreaterThanOrEqual(column.x0 - 1e-9);
      expect(box.x1).toBeLessThanOrEqual(column.x1 + 1e-9);
      expect(box.z0).toBeGreaterThanOrEqual(column.z0 - 1e-9);
      expect(box.z1).toBeLessThanOrEqual(column.z1 + 1e-9);
      expect(box.y0).toBeGreaterThanOrEqual(0);
      expect(box.y1).toBeLessThanOrEqual(BLOCK_TOP);
      expect(Math.abs(box.x0)).toBeLessThanOrEqual(42 + 1e-9);
      expect(Math.abs(box.x1)).toBeLessThanOrEqual(42 + 1e-9);
      expect(Math.abs(box.z0)).toBeLessThanOrEqual(42 + 1e-9);
      expect(Math.abs(box.z1)).toBeLessThanOrEqual(42 + 1e-9);
    }
  });

  it('never lets a box overlap a box of another column, or another box of its own column', () => {
    for (const [index, a] of boxes.entries()) {
      for (const b of boxes.slice(index + 1)) {
        expect(apart(a.box, b.box), `${a.column.node.name} / ${b.column.node.name}`).toBe(true);
      }
    }
  });
});

describe('layoutColumns, the "smaller items" floor and part', () => {
  /** `games` holds four games and a long tail of small items; the first game has a tail of its own. */
  const disk = (): LodNode => {
    const first = lod('game0', 100, [
      leaf('pak0', 40),
      leaf('pak1', 30),
      ...Array.from({ length: 30 }, (_, index) => leaf(`bit-${index}`, 1)),
    ]);
    const others = [100, 90, 80].map((size, index) =>
      lod(`game${index + 1}`, size, [leaf(`d${index}`, size)]),
    );
    const tail = Array.from({ length: 60 }, (_, index) => leaf(`small-${index}`, 2));
    const games = lod('games', 490, [first, ...others, ...tail]);

    return lod('home', 560, [games, lod('docs', 70, [leaf('a', 70)])]);
  };
  const layout = layoutColumns(collapseChains(disk()));
  const games = layout.columns.find((column) => column.node.name === 'games') as Column;
  const floors = (column: Column): Box[][] => {
    const ys = [...new Set(column.boxes.filter((box) => !box.lid).map((box) => box.y0))].sort(
      (a, b) => a - b,
    );

    return ys.map((y) => column.boxes.filter((box) => !box.lid && box.y0 === y));
  };

  it('has the rest floor last, on top of the column, and only that floor is a rest floor', () => {
    const stack = floors(games);

    expect(stack.length).toBeGreaterThan(2);
    expect(stack.slice(0, -1).every((floor) => floor.some((box) => !box.rest))).toBe(true);
    expect(stack.at(-1)?.every((box) => box.rest)).toBe(true);
  });

  it('has the rest part last in its floor, after every other part, whatever its size', () => {
    const [first] = floors(games);
    const parts = [...(first ?? [])].sort((a, b) => a.x0 - b.x0 || a.z0 - b.z0);

    expect(parts.length).toBeGreaterThan(2);
    expect(parts.at(-1)?.rest).toBe(true);
    expect(parts.slice(0, -1).some((part) => part.rest)).toBe(false);
  });

  it("gives the rest floor of a framed column the column's own colour, not a floor colour of its own", () => {
    const model = collapseChains(disk());
    const frame = model.children[0]?.node.id as number;
    const column = layoutColumns(model, undefined, frame).columns.find(
      (candidate) => candidate.top === frame,
    ) as Column;
    const stack = floors(column);

    expect(stack.slice(0, -1).every((floor) => floor.every((box) => box.hue !== undefined))).toBe(true);
    expect(stack.at(-1)?.every((box) => box.rest && box.hue === undefined)).toBe(true);
  });

  it('puts the "smaller items" column after every other column', () => {
    const crowded = layoutColumns(
      lod('f', 1000, [
        ...Array.from({ length: 12 }, (_, index) => lod(`n${index}`, 100 - index, [leaf('x', 100)])),
        smaller(500, 300),
      ]),
    );

    expect(crowded.columns.at(-1)?.top).toBe(COLLAPSED_NODE_ID);
  });
});
