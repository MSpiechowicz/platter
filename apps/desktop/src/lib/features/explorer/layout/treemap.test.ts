import { describe, expect, it } from 'vitest';
import { COLLAPSED_NODE_ID } from '../../../api/client';
import { leaf, lod, sample, skewed, smaller } from '../fixtures';
import { collapseChains } from './chains';
import { membersOf } from './legibility';
import { groupByTop, layoutTreemap, mergedTooltip, tileLabel, type Tile } from './treemap';
import type { Measurer } from './text';

const area = (tile: Tile): number => (tile.x1 - tile.x0) * (tile.y1 - tile.y0);
const inside = (inner: Tile, outer: Tile): boolean =>
  inner.x0 >= outer.x0 && inner.y0 >= outer.y0 && inner.x1 <= outer.x1 && inner.y1 <= outer.y1;
const overlap = (a: Tile, b: Tile): boolean => a.x0 < b.x1 && a.x1 > b.x0 && a.y0 < b.y1 && a.y1 > b.y0;

describe('layoutTreemap', () => {
  const root = sample([600, 300, 100]);
  const tiles = layoutTreemap(root, 900, 600);
  const top = tiles.filter((tile) => tile.depth === 1);

  it('draws one tile per child, then its children, three levels down, breadth first', () => {
    expect(top.map((tile) => tile.node.name)).toEqual(['dir0', 'dir1', 'dir2']);
    expect(Math.max(...tiles.map((tile) => tile.depth))).toBe(3);
    expect(tiles.map((tile) => tile.depth)).toEqual([...tiles.map((tile) => tile.depth)].sort());
  });

  it('gives top-level tiles an area in proportion to their size', () => {
    const [big, middle, small] = top.map(area) as [number, number, number];

    // The padding between tiles costs a little, so allow a few per cent.
    expect(big / middle).toBeGreaterThan(1.8);
    expect(big / middle).toBeLessThan(2.2);
    expect(middle / small).toBeGreaterThan(2.5);
    expect(middle / small).toBeLessThan(3.5);
  });

  it('keeps every tile inside its parent and every pair of siblings apart', () => {
    for (const tile of tiles.filter((candidate) => candidate.depth > 1)) {
      const parent = tiles.find(
        (candidate) =>
          candidate.depth === tile.depth - 1 &&
          candidate.top === tile.top &&
          tile.key.startsWith(`${candidate.key}/`),
      );

      expect(parent, tile.key).toBeDefined();
      expect(inside(tile, parent as Tile), tile.key).toBe(true);
    }

    for (const [index, a] of top.entries()) {
      for (const b of top.slice(index + 1)) {
        expect(overlap(a, b)).toBe(false);
      }
    }
  });

  it('stays inside the given size and leaves a header strip over folders with children', () => {
    expect(Math.max(...tiles.map((tile) => tile.x1))).toBeLessThanOrEqual(900);
    expect(Math.max(...tiles.map((tile) => tile.y1))).toBeLessThanOrEqual(600);

    const folder = top[0] as Tile;
    const firstChild = tiles.find((tile) => tile.depth === 2 && tile.top === folder.top) as Tile;

    expect(folder.header).toBe(true);
    expect(firstChild.y0 - folder.y0).toBeGreaterThanOrEqual(25);
  });

  it('has no header on the third level or on files', () => {
    expect(tiles.filter((tile) => tile.depth === 3).every((tile) => !tile.header)).toBe(true);
    expect(
      layoutTreemap(lod('f', 10, [leaf('a', 6), leaf('b', 4)]), 400, 300).every((tile) => !tile.header),
    ).toBe(true);
  });

  it('names the top-level folder on every tile', () => {
    for (const group of groupByTop(tiles)) {
      expect(group.tiles[0]?.depth).toBe(1);
      expect(group.tiles.every((tile) => tile.top === group.top)).toBe(true);
    }
  });

  it('draws the "smaller items" node as a tile of its own', () => {
    const withRest = lod('f', 100, [lod('a', 60, [leaf('x', 60)]), leaf('b', 25), smaller(30, 15)]);
    const rest = layoutTreemap(withRest, 600, 400).find((tile) => tile.node.id === COLLAPSED_NODE_ID) as Tile;

    expect(rest.depth).toBe(1);
    expect(rest.top).toBe(COLLAPSED_NODE_ID);
    expect(rest.header).toBe(false);
    expect(area(rest)).toBeGreaterThan(0);
  });

  it('draws nothing for an empty folder or an empty box', () => {
    expect(layoutTreemap(lod('empty', 0), 600, 400)).toEqual([]);
    expect(layoutTreemap(lod('f', 10, [leaf('a', 10)]), 0, 400)).toEqual([]);
    expect(layoutTreemap(lod('f', 0, [leaf('a', 0)]), 600, 400)).toEqual([]);
  });

  it('drops tiles too small to see', () => {
    const many = lod('f', 100_000, [
      leaf('big', 100_000),
      ...Array.from({ length: 5 }, (_, i) => leaf(`dust${i}`, 1)),
    ]);

    expect(
      layoutTreemap(many, 300, 200).every((tile) => tile.x1 - tile.x0 >= 1 && tile.y1 - tile.y0 >= 1),
    ).toBe(true);
  });
});

describe('layoutTreemap on a skewed folder', () => {
  const root = skewed();
  const tiles = layoutTreemap(root, 900, 600);
  const top = tiles.filter((tile) => tile.depth === 1);

  it('draws no tile narrower or shorter than 14 pixels, at any level', () => {
    expect(tiles.length).toBeGreaterThan(0);

    for (const tile of tiles) {
      expect(tile.x1 - tile.x0, tile.key).toBeGreaterThanOrEqual(14);
      expect(tile.y1 - tile.y0, tile.key).toBeGreaterThanOrEqual(14);
    }
  });

  it('puts the dust into one "smaller items" tile, as big as the items it replaced', () => {
    const merged = top.find((tile) => tile.node.id === COLLAPSED_NODE_ID) as Tile;
    const replaced = root.children.filter((child) => !top.some((tile) => tile.node.id === child.node.id));

    // Under 0.05% of the folder is dust: four of the ten items.
    expect(merged.node.name).toBe('4 smaller items');
    expect(replaced).toHaveLength(4);
    expect(merged.node.allocated).toBe(replaced.reduce((sum, child) => sum + child.node.allocated, 0));
    expect(membersOf(merged.node)).toEqual(replaced.map((child) => child.node.id));
  });

  it('fills the folder, with the big folder biggest and the small ones large enough to read', () => {
    const drawn = top.reduce((sum, tile) => sum + area(tile), 0);

    // 890 x 590 is inside the outer padding; the gaps between tiles take the rest.
    expect(drawn / (890 * 590)).toBeGreaterThan(0.97);

    const [big, ...small] = top as [Tile, ...Tile[]];

    expect(small.every((tile) => area(big) > area(tile))).toBe(true);

    // The 1.3% folders are not the slivers that their share would make (12 px wide) but readable tiles, at their
    // true size; only the 0.2% one is drawn larger than it is.
    for (const tile of small.filter(
      (candidate) => candidate.node.name === 'dir1' || candidate.node.name === 'dir2',
    )) {
      expect(tile.x1 - tile.x0).toBeGreaterThanOrEqual(80);
      expect(tile.y1 - tile.y0).toBeGreaterThanOrEqual(44);
      expect(tile.boosted, tile.node.name).toBe(false);
      expect(tile.share).toBeCloseTo(0.013, 3);
    }

    const tiny = small.find((tile) => tile.node.name === 'dir3') as Tile;

    expect(tiny.boosted).toBe(true);
    expect(tiny.share).toBeCloseTo(0.002, 3);
    expect(big.boosted).toBe(false);
  });

  it('draws a small folder as a plain tile, with nothing inside it', () => {
    const small = top.find((tile) => tile.node.name === 'dir1') as Tile;

    expect(small.header).toBe(false);
    expect(tiles.some((tile) => tile.depth > 1 && tile.top === small.top)).toBe(false);
  });

  it('merges the small siblings of a nested folder too', () => {
    const inside = tiles.filter((tile) => tile.depth === 2 && tile.top === top[0]?.top);

    expect(inside.length).toBeGreaterThan(1);
    expect(inside.every((tile) => tile.x1 - tile.x0 >= 14 && tile.y1 - tile.y0 >= 14)).toBe(true);
  });

  it('draws nothing for a folder with too little to see, instead of slivers', () => {
    const tiny = lod('f', 100, [leaf('a', 50), leaf('b', 50)]);
    const result = layoutTreemap(tiny, 20, 20);

    for (const tile of result) {
      expect(tile.x1 - tile.x0).toBeGreaterThanOrEqual(14);
    }
  });

  it('takes the backend\'s own "smaller items" node into the merged tile and counts its items', () => {
    const base = skewed();
    const withRest = { ...base, children: [...base.children, smaller(4, 5000)] };
    const merged = layoutTreemap(withRest, 900, 600).find(
      (tile) => tile.node.id === COLLAPSED_NODE_ID,
    ) as Tile;

    expect(merged.node.name).toBe('10 smaller items');
    expect(merged.node.allocated).toBe(20_000 + 5000);
  });

  it('does not list items without a size among the smaller items', () => {
    const withEmpty = lod('f', 100, [leaf('a', 60), leaf('b', 40), leaf('empty1', 0), leaf('empty2', 0)]);

    expect(layoutTreemap(withEmpty, 600, 400).map((tile) => tile.node.name)).toEqual(['a', 'b']);
  });
});

describe('layoutTreemap on a balanced folder', () => {
  it('is not changed by the legibility rules: every tile of the old layout is still there', () => {
    const tiles = layoutTreemap(sample([600, 300, 100]), 900, 600);

    // Three folders, each with two sub-folders and three leaves between them.
    expect(tiles).toHaveLength(18);
    expect(tiles.some((tile) => tile.node.id === COLLAPSED_NODE_ID)).toBe(false);
  });
});

/** The user's disk: `home / sam` holds 97% and has big folders inside, `var`, `usr` and `opt` are small. */
function realDisk() {
  const folders = [823, 336, 136, 102, 97, 14, 12, 7.1, 6.7, 5.7].map((size, index) =>
    lod(`f${index}`, size, [
      lod(`g${index}`, size * 0.6, [leaf(`x${index}`, size * 0.6)]),
      leaf(`y${index}`, size * 0.4),
    ]),
  );
  const home = lod('home', 1550, [lod('sam', 1550, folders)]);

  return lod('/', 1595.6, [
    home,
    lod('var', 21, [leaf('lib', 12), leaf('cache', 9)]),
    lod('usr', 21, [leaf('lib', 14), leaf('share', 7)]),
    lod('opt', 3.6, [leaf('a', 3.6)]),
    lod('etc', 0.017, [leaf('conf', 0.017)]),
    ...Array.from({ length: 14 }, (_, index) => leaf(`stub-${index}`, 0.000001)),
  ]);
}

describe.each([
  [900, 600],
  [1240, 760],
  [1640, 880],
])("layoutTreemap on the user's disk, %i x %i", (width, height) => {
  const tiles = layoutTreemap(collapseChains(realDisk()), width, height);
  const top = tiles.filter((tile) => tile.depth === 1);

  it('draws home / sam as one tile and the small folders beside it, each with room for a name and a size', () => {
    expect(top.map((tile) => tile.node.name).slice(0, 4)).toEqual(['home', 'var', 'usr', 'opt']);
    expect(top[0]?.chain?.map((link) => link.name)).toEqual(['home', 'sam']);

    for (const name of ['var', 'usr', 'opt']) {
      const tile = top.find((candidate) => candidate.node.name === name) as Tile;

      expect(tile.x1 - tile.x0, name).toBeGreaterThanOrEqual(80);
      expect(tile.y1 - tile.y0, name).toBeGreaterThanOrEqual(44);
      expect(tileLabel(tile, '21 GB'), name).not.toBeNull();
    }
  });

  it('folds etc and the tiny items into one "smaller items" tile that can be read as well', () => {
    const rest = top.find((tile) => tile.node.id === COLLAPSED_NODE_ID) as Tile;

    expect(rest.node.name).toBe('15 smaller items');
    expect(rest.y1 - rest.y0).toBeGreaterThanOrEqual(44);
    expect(rest.boosted).toBe(true);
  });

  it('spends the levels inside the chain: the big folders of sam have their own contents', () => {
    const inside = tiles.filter((tile) => tile.depth === 2 && tile.top === top[0]?.top);

    expect(inside.length).toBeGreaterThan(5);
    expect(tiles.some((tile) => tile.depth === 3 && tile.top === top[0]?.top)).toBe(true);
    expect(inside.every((tile) => tile.inner === tile.node.id)).toBe(true);
  });

  it('has no overlaps, everything inside the map, only finite numbers, and nothing under 14 pixels', () => {
    for (const tile of tiles) {
      expect(Number.isFinite(tile.x0 + tile.y0 + tile.x1 + tile.y1)).toBe(true);
      expect(tile.x0).toBeGreaterThanOrEqual(0);
      expect(tile.y0).toBeGreaterThanOrEqual(0);
      expect(tile.x1).toBeLessThanOrEqual(width);
      expect(tile.y1).toBeLessThanOrEqual(height);
      expect(tile.x1 - tile.x0).toBeGreaterThanOrEqual(14);
      expect(tile.y1 - tile.y0).toBeGreaterThanOrEqual(14);
    }

    for (const [index, a] of top.entries()) {
      for (const b of top.slice(index + 1)) {
        expect(overlap(a, b), `${a.node.name} / ${b.node.name}`).toBe(false);
      }
    }
  });

  it('says what a boosted tile really holds, and a tile drawn at its size is not boosted', () => {
    expect(top[0]?.boosted).toBe(false);
    expect(top[0]?.share).toBeCloseTo(1550 / 1595.6, 3);

    const opt = top.find((tile) => tile.node.name === 'opt') as Tile;

    expect(opt.boosted).toBe(true);
    expect(opt.share).toBeCloseTo(3.6 / 1595.6, 4);
  });

  it('is monotonic: a bigger folder never has a smaller tile than a smaller one', () => {
    const real = top.filter((tile) => tile.node.id !== COLLAPSED_NODE_ID);

    for (const a of real) {
      for (const b of real) {
        if (a.node.allocated > b.node.allocated * 1.5) {
          expect(area(a), `${a.node.name} / ${b.node.name}`).toBeGreaterThanOrEqual(area(b));
        }
      }
    }
  });
});

describe('layoutTreemap, a balanced disk with the readable minimum', () => {
  it('draws it exactly as proportional areas would, with nothing boosted', () => {
    const tiles = layoutTreemap(sample([300, 250, 200, 150, 100]), 900, 600).filter(
      (tile) => tile.depth === 1,
    );

    expect(tiles.map((tile) => tile.boosted)).toEqual([false, false, false, false, false]);
    expect(tiles.reduce((sum, tile) => sum + tile.share, 0)).toBeCloseTo(1, 9);
  });
});

/** Ten pixels a character, whatever the font: easy to count by hand. */
const ten: Measurer = (text) => text.length * 10;

describe('tileLabel', () => {
  const tile = (name: string, width: number, height: number, header: boolean): Tile => ({
    key: name,
    node: leaf(name, 1).node,
    depth: 1,
    top: 1,
    x0: 0,
    y0: 0,
    x1: width,
    y1: height,
    header,
    share: 0,
    boosted: false,
    parentName: 'sam',
    shareOfParent: 0.017,
  });
  const chained = (width: number, header = true): Tile => ({
    ...tile('.docker', width, 120, header),
    chain: ['.docker', 'desktop', 'vms', '0', 'data'].map((name) => leaf(name, 1).node),
  });
  const rest = (width: number, height: number): Tile => ({
    ...tile('x', width, height, false),
    node: smaller(285, 14).node,
  });

  describe('a header strip', () => {
    it('puts the size on the right when the name and the size both fit completely with a gap of 8 px', () => {
      // 8 characters, 8 px, 6 characters: 148 px, in 300 - 16.
      expect(tileLabel(tile('Projects', 300, 120, true), '214 GB', ten)).toEqual({
        name: 'Projects',
        size: '214 GB',
        where: 'header',
        sizeOnRight: true,
      });
      expect(tileLabel(tile('Projects', 164, 120, true), '214 GB', ten)?.size).toBe('214 GB');
    });

    it('keeps the name alone, whole, when the size would not fit with it', () => {
      expect(tileLabel(tile('Projects', 163, 120, true), '214 GB', ten)).toMatchObject({
        name: 'Projects',
        size: '',
      });
    });

    it('cuts a long name at its end, and has no size then', () => {
      const cut = tileLabel(tile('a-very-long-folder-name', 100, 120, true), '1 GB', ten);

      expect(cut).toMatchObject({ name: 'a-very-…', size: '' });
      expect((cut?.name.length ?? 0) * 10).toBeLessThanOrEqual(100 - 16);
    });

    it('writes a chain whole when it fits, else its first folder, else the start of that, never its tail', () => {
      // The whole chain is 34 characters: 340 px, and 408 with the size.
      expect(tileLabel(chained(500), '336 GB', ten)).toMatchObject({
        name: '.docker / desktop / vms / 0 / data',
        size: '336 GB',
      });
      expect(tileLabel(chained(400), '336 GB', ten)).toMatchObject({ name: '.docker', size: '336 GB' });
      expect(tileLabel(chained(360), '336 GB', ten)).toMatchObject({ name: '.docker', size: '336 GB' });
      // The size no longer fits next to the first folder: the name has the strip.
      expect(tileLabel(chained(120), '336 GB', ten)).toMatchObject({ name: '.docker', size: '' });
      expect(tileLabel(chained(70), '336 GB', ten)).toMatchObject({ name: '.doc…', size: '' });
      expect(tileLabel(chained(50), '336 GB', ten)).toBeNull();

      for (const width of [50, 70, 120, 200, 400]) {
        expect(tileLabel(chained(width), '336 GB', ten)?.name ?? '').not.toMatch(/data|…[^…]/);
      }
    });

    it('writes nothing in a strip with no room for four characters', () => {
      expect(tileLabel(tile('work', 20, 80, true), '19 GB', ten)).toBeNull();
    });
  });

  describe('a plain tile', () => {
    it('writes name over size when both fit', () => {
      expect(tileLabel(tile('work', 120, 80, false), '19 GB', ten)).toMatchObject({
        name: 'work',
        size: '19 GB',
        where: 'body',
      });
    });

    it('cuts the name at its end, and writes the size whole or not at all', () => {
      expect(tileLabel(tile('Car Mechanic Simulator', 150, 80, false), '7.0 GB', ten)).toMatchObject({
        name: 'Car Mechanic…',
        size: '7.0 GB',
      });
      // 54 px of room: the name fits, six characters of the size (60 px) do not, and it is not cut to "7.0…".
      expect(tileLabel(tile('work', 70, 80, false), '7.0 GB', ten)).toMatchObject({
        name: 'work',
        size: '',
      });
    });

    it('has no name when not even four characters and the ellipsis fit', () => {
      // 44 px of room: "Steam" (50) does not fit, and "Stea…" needs 50 as well.
      expect(tileLabel(tile('Steam', 60, 80, false), '1 GB', ten)).toBeNull();
    });

    it('writes a chain tile by its first folder, and never by its tail', () => {
      expect(tileLabel(chained(150, false), '336 GB', ten)).toMatchObject({
        name: '.docker',
        size: '336 GB',
      });
      expect(tileLabel(chained(100, false), '336 GB', ten)?.name).toBe('.docker');
      expect(tileLabel(chained(80, false), '336 GB', ten)?.name).toBe('.dock…');
    });

    it('writes nothing when there is no room', () => {
      expect(tileLabel(tile('work', 120, 30, false), '19 GB', ten)).toBeNull();
      expect(tileLabel(tile('work', 40, 80, false), '19 GB', ten)).toBeNull();
    });
  });

  describe('the "smaller items" tile', () => {
    it('writes "285 smaller" over the size when both fit', () => {
      expect(tileLabel(rest(130, 60), '14 GB', ten)).toEqual({
        name: '285 smaller',
        size: '14 GB',
        where: 'body',
        sizeOnRight: false,
      });
    });

    it('writes "+285" when only one line fits, because of the height or the width', () => {
      expect(tileLabel(rest(300, 30), '14 GB', ten)?.name).toBe('+285');
      expect(tileLabel(rest(100, 60), '14 GB', ten)).toMatchObject({ name: '+285', size: '' });
      // The size does not fit under the count: the count alone, short.
      expect(tileLabel(rest(120, 60), '14.0 GB', ten)).toMatchObject({ name: '+285', size: '' });
    });

    it('writes nothing when not even that fits', () => {
      expect(tileLabel(rest(40, 60), '14 GB', ten)).toBeNull();
      expect(tileLabel(rest(300, 20), '14 GB', ten)).toBeNull();
    });

    it('says all of it in the tooltip, with its share of the folder it is in', () => {
      expect(mergedTooltip(rest(40, 60), '14 GB')).toBe('285 smaller items · 14 GB · 1.7% of sam');
    });
  });
});

/** A folder `games` that holds six games, a long tail of small ones (more of them than six games), and tiny items. */
function gamesDisk() {
  const tail = Array.from({ length: 300 }, (_, index) => leaf(`small-${index}`, 0.3));
  const sized = [100, 90, 80, 70, 60, 50].map((size, index) =>
    lod(`game${index}`, size, [leaf(`data${index}`, size * 0.7), leaf(`save${index}`, size * 0.3)]),
  );
  // The biggest game has a tail of its own, so that the third level has one too.
  const first = lod('game0', 220, [
    ...Array.from({ length: 4 }, (_, index) => leaf(`pak${index}`, 50)),
    ...Array.from({ length: 200 }, (_, index) => leaf(`bit-${index}`, 0.1)),
  ]);
  const games = lod('games', 1000, [first, ...sized.slice(1), ...tail]);
  const stubs = Array.from({ length: 10 }, (_, index) => leaf(`stub-${index}`, 0.00001));

  return lod('home', 1100, [games, lod('docs', 30, [leaf('a', 30)]), ...stubs]);
}

describe('the "smaller items" tile is the last tile of its folder', () => {
  const tiles = layoutTreemap(collapseChains(gamesDisk()), 1400, 900);
  const merged = tiles.filter((tile) => tile.node.id === COLLAPSED_NODE_ID);
  const parentKey = (tile: Tile): string => tile.key.split('/').slice(0, -1).join('/');
  const siblingsOf = (tile: Tile): Tile[] =>
    tiles.filter(
      (other) => other !== tile && other.depth === tile.depth && parentKey(other) === parentKey(tile),
    );

  it('has one at the top level and inside folders, at more than one level', () => {
    expect(merged.some((tile) => tile.depth === 1)).toBe(true);
    expect(merged.some((tile) => tile.depth === 2)).toBe(true);
    expect(merged.some((tile) => tile.depth === 3)).toBe(true);
  });

  it('puts it in the bottom right corner of its folder, after every sibling, however big it is', () => {
    for (const tile of merged) {
      const siblings = siblingsOf(tile);

      expect(siblings.length, tile.key).toBeGreaterThan(0);
      expect(tile.x1, tile.key).toBe(Math.max(tile.x1, ...siblings.map((other) => other.x1)));
      expect(tile.y1, tile.key).toBe(Math.max(tile.y1, ...siblings.map((other) => other.y1)));
    }

    // The one under `games` is bigger than most of the games beside it: size does not decide where it goes.
    const inner = merged.find((tile) => tile.depth === 2) as Tile;
    const bigger = siblingsOf(inner).filter((other) => area(other) < area(inner));

    expect(bigger.length).toBeGreaterThan(1);
  });

  it('is the last tile of the top level, after the folders and in the corner of the band', () => {
    const top = tiles.filter((tile) => tile.depth === 1);
    const last = top.find((tile) => tile.node.id === COLLAPSED_NODE_ID) as Tile;

    expect(top.at(-1)).toBe(last);
    expect(last.y1).toBe(Math.max(...top.map((tile) => tile.y1)));
    expect(last.x1).toBe(Math.max(...top.map((tile) => tile.x1)));
  });

  it('keeps the rules: nothing under 14 pixels, nothing over another tile, everything inside its parent', () => {
    for (const tile of tiles) {
      expect(tile.x1 - tile.x0, tile.key).toBeGreaterThanOrEqual(14);
      expect(tile.y1 - tile.y0, tile.key).toBeGreaterThanOrEqual(14);
    }

    for (const tile of tiles.filter((candidate) => candidate.depth > 1)) {
      const parent = tiles.find(
        (candidate) => candidate.depth === tile.depth - 1 && tile.key.startsWith(`${candidate.key}/`),
      ) as Tile;

      expect(inside(tile, parent), tile.key).toBe(true);

      for (const other of siblingsOf(tile)) {
        expect(overlap(tile, other), `${tile.key} / ${other.key}`).toBe(false);
      }
    }
  });

  it('adds up: the tiles of a folder fill what the folder gives them, with the merged one included', () => {
    const games = tiles.find((tile) => tile.node.name === 'games') as Tile;
    const children = tiles.filter((tile) => tile.depth === 2 && tile.top === games.top);
    const content = (games.x1 - games.x0 - 10) * (games.y1 - games.y0 - 25 - 5);
    const drawn = children.reduce((sum, tile) => sum + area(tile), 0);

    expect(drawn / content).toBeGreaterThan(0.85);
    expect(drawn).toBeLessThanOrEqual(content);
  });

  it('knows its folder and how much of it is in it, for the tooltip', () => {
    const inner = merged.find((tile) => tile.depth === 2) as Tile;
    const top = merged.find((tile) => tile.depth === 1) as Tile;

    expect(inner.parentName).toBe('games');
    // 300 items of 0.3 in a folder of 220 + 350 + 90.
    expect(inner.shareOfParent).toBeCloseTo(90 / 660, 3);
    expect(top.parentName).toBe('home');
    expect(top.shareOfParent).toBeLessThan(0.001);
  });

  it('does not draw a merged tile that is under 14 px, and leaves its room to the folder', () => {
    // A narrow map: the dust is a sliver. Every tile that is drawn is still 14 px, and the rest do not move.
    const narrow = layoutTreemap(collapseChains(gamesDisk()), 300, 200);

    for (const tile of narrow) {
      expect(tile.x1 - tile.x0, tile.key).toBeGreaterThanOrEqual(14);
      expect(tile.y1 - tile.y0, tile.key).toBeGreaterThanOrEqual(14);
    }
  });
});
