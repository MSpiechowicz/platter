import { describe, expect, it } from 'vitest';
import { COLLAPSED_NODE_ID } from '../../api/client';
import { childColors } from '../../folderColor';
import {
  assignMapColors,
  BACKGROUND,
  blend,
  colorOf,
  dominantTile,
  ownChildren,
  FLAT_MIX,
  MERGED_STEP,
  PALETTE,
  SLATE,
  tileStyle,
} from './colors';
import { leaf, lod, smaller } from './fixtures';
import { collapseChains } from './layout/chains';
import { layoutTreemap, type Tile } from './layout/treemap';

/** Hue in degrees of a `#rrggbb` colour. */
function hue(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((at) => Number.parseInt(hex.slice(at, at + 2), 16) / 255) as [
    number,
    number,
    number,
  ];
  const max = Math.max(r, g, b);
  const delta = max - Math.min(r, g, b);

  if (delta === 0) {
    return 0;
  }

  const raw = max === r ? ((g - b) / delta) % 6 : max === g ? (b - r) / delta + 2 : (r - g) / delta + 4;

  return (raw * 60 + 360) % 360;
}

describe('blend', () => {
  it('is the background at 0% and the colour at 100%', () => {
    expect(blend('#3ec5d8', 0)).toBe(BACKGROUND);
    expect(blend('#3ec5d8', 100)).toBe('#3ec5d8');
  });

  it('mixes towards the background like CSS color-mix in sRGB', () => {
    // 22% of #3ec5d8 over #0a0e17, channel by channel.
    expect(blend('#3ec5d8', 22)).toBe('#153641');
    expect(blend('#ffffff', 50)).toBe('#85878b');
  });

  it('keeps deeper levels lighter', () => {
    const light = (hex: string) => Number.parseInt(hex.slice(1, 3), 16);

    expect(light(blend('#d6739f', 22))).toBeLessThan(light(blend('#d6739f', 38)));
    expect(light(blend('#d6739f', 38))).toBeLessThan(light(blend('#d6739f', 56)));
  });
});

describe('palette', () => {
  it('has no green, because green means "safe"', () => {
    for (const color of PALETTE) {
      const degrees = hue(color);

      expect(degrees < 80 || degrees > 175, `${color} has hue ${Math.round(degrees)}`).toBe(true);
    }
  });

  it('has distinct colours', () => {
    expect(new Set(PALETTE).size).toBe(PALETTE.length);
  });
});

const topOf = (root: ReturnType<typeof lod>) => collapseChains(root).children;
const idOf = (item: { node: { id: number } } | undefined): number => item?.node.id as number;

describe('assignMapColors, one rule for every folder', () => {
  it('gives the children of the scan root the palette in order, biggest first', () => {
    const small = lod('small', 10);
    const big = lod('big', 500);
    const middle = lod('middle', 50);
    const colors = assignMapColors(null, topOf(lod('/', 560, [small, big, middle])));

    expect(colors.get(idOf(big))).toBe(PALETTE[0]);
    expect(colors.get(idOf(middle))).toBe(PALETTE[1]);
    expect(colors.get(idOf(small))).toBe(PALETTE[2]);
  });

  it('lets the largest child inherit the colour of the open folder, and skips that colour for the others', () => {
    const [big, next, last] = [lod('big', 500), lod('next', 300), lod('last', 100)];
    const colors = assignMapColors(PALETTE[1] as string, topOf(lod('f', 900, [last, next, big])));

    expect(colors.get(idOf(big))).toBe(PALETTE[1]);
    expect(colors.get(idOf(next))).toBe(PALETTE[0]);
    expect(colors.get(idOf(last))).toBe(PALETTE[2]);
  });

  it('is the colours that childColors gives, which the Overview uses too', () => {
    const children = [lod('a', 400), lod('b', 300), lod('c', 200)];
    const colors = assignMapColors(PALETTE[3] as string, topOf(lod('f', 900, children)));
    const shared = childColors(
      PALETTE[3] as string,
      children.map((child) => ({ id: child.node.id, allocated: child.node.allocated })),
    );

    for (const child of children) {
      expect(colors.get(child.node.id)).toBe(shared.get(child.node.id));
    }
  });

  it('uses no green, and slate after the palette is used up, for "smaller items" and for empty folders', () => {
    const many = Array.from({ length: PALETTE.length + 2 }, (_, index) => lod(`n${index}`, 1000 - index));
    const rest = smaller(40, 900);
    const colors = assignMapColors(null, topOf(lod('f', 9000, [...many, rest, leaf('empty', 0)])));

    expect(colors.get(idOf(many[PALETTE.length]))).toBe(SLATE);
    expect(colors.get(COLLAPSED_NODE_ID)).toBe(SLATE);

    for (const color of colors.values()) {
      expect([...PALETTE, SLATE]).toContain(color);
    }
  });

  it('falls back to slate for an id it does not know', () => {
    expect(colorOf(assignMapColors(null, []), 12345)).toBe(SLATE);
  });

  it("gives a chain tile one colour, the first folder's, which its last folder has when it is opened", () => {
    const root = lod('/', 100, [
      lod('home', 97, [lod('me', 97, [lod('a', 60), lod('b', 37)])]),
      lod('var', 3),
    ]);
    const [home] = topOf(root);
    const colors = assignMapColors(null, topOf(root));
    const afterOpening = assignMapColors(colors.get(idOf(home)) as string, [
      { node: root.children[0]?.children[0]?.node as never, children: [] },
    ]);

    expect(colors.get(idOf(home))).toBe(PALETTE[0]);
    expect(afterOpening.get(root.children[0]?.children[0]?.node.id as number)).toBe(PALETTE[0]);
  });
});

describe('assignMapColors with one tile that holds most of the map', () => {
  const root = lod('/', 100, [
    lod('home', 97, [
      lod('me', 97, [
        lod('a', 60, [leaf('x', 60)]),
        lod('b', 25, [leaf('y', 25)]),
        lod('c', 12, [leaf('z', 12)]),
      ]),
    ]),
    lod('var', 2, [leaf('v', 2)]),
    lod('usr', 1, [leaf('u', 1)]),
  ]);
  const top = topOf(root);
  const [home] = top;
  const [a, b, c] = (home?.children ?? []).map(idOf) as [number, number, number];

  it("colours the folders inside it by the same rule, with the tile's colour as the parent's", () => {
    const colors = assignMapColors(null, top);

    expect(dominantTile(top)).toBe(home);
    // The largest folder inside keeps the tile's colour; the others take the palette, skipping it.
    expect(colors.get(a)).toBe(colors.get(idOf(home)));
    expect(colors.get(b)).toBe(PALETTE[1]);
    expect(colors.get(c)).toBe(PALETTE[2]);
  });

  it('gives the folders inside the colours they have once the tile is opened', () => {
    const colors = assignMapColors(null, top);
    const opened = assignMapColors(colors.get(idOf(home)) as string, home?.children ?? []);

    for (const id of [a, b, c]) {
      expect(opened.get(id), String(id)).toBe(colors.get(id));
    }
  });

  it('colours the other top-level folders by the rule too, whatever the inside uses', () => {
    const colors = assignMapColors(null, top);

    expect(colors.get(idOf(home))).toBe(PALETTE[0]);
    expect(colors.get(idOf(top[1]))).toBe(PALETTE[1]);
    expect(colors.get(idOf(top[2]))).toBe(PALETTE[2]);
  });

  it('is only for a tile above 60% of the map', () => {
    const balanced = topOf(
      lod('/', 100, [
        lod('p', 55, [lod('q', 55, [lod('r', 30), lod('s', 25)])]),
        lod('t', 45, [leaf('w', 45)]),
      ]),
    );

    expect(dominantTile(balanced)).toBeUndefined();
  });

  it('leaves out the folders that only the other folders of the chain hold, as they are not in the opened folder', () => {
    const chain = topOf(
      lod('/', 100, [
        lod('home', 100, [lod('me', 95, [lod('a', 60), lod('b', 35)]), lod('guest', 5, [leaf('g', 5)])]),
      ]),
    )[0];

    expect(ownChildren(chain as NonNullable<typeof chain>).map((child) => child.node.name)).toEqual([
      'a',
      'b',
    ]);
  });
});

describe('tileStyle of the "smaller items" tile', () => {
  /** `games` holds a few games and a long tail; beside it there is `docs`, and dust that is merged at the top level. */
  const build = () => {
    const tail = Array.from({ length: 300 }, (_, index) => leaf(`small-${index}`, 0.1));
    const games = [100, 90, 80, 70, 60, 50].map((size, index) =>
      lod(`game${index}`, size, [leaf(`d${index}`, size)]),
    );
    const stubs = Array.from({ length: 10 }, (_, index) => leaf(`stub-${index}`, 0.00001));
    const root = lod('home', 1100, [
      lod('games', 1000, [...games, ...tail]),
      lod('docs', 30, [leaf('a', 30)]),
      ...stubs,
    ]);
    const model = collapseChains(root);
    const colors = assignMapColors(null, model.children);

    return { tiles: layoutTreemap(model, 1400, 900), colors };
  };
  const { tiles, colors } = build();
  const merged = (depth: number): Tile =>
    tiles.find((tile) => tile.node.id === COLLAPSED_NODE_ID && tile.depth === depth) as Tile;
  const gamesColor =
    (tiles.find((tile) => tile.node.name === 'games') as Tile) &&
    colors.get(tiles.find((tile) => tile.node.name === 'games')?.node.id as number);

  it('is slate at the top level, where it mixes folders of different colours', () => {
    expect(tileStyle(merged(1), colors).fill).toBe(blend(SLATE, FLAT_MIX[1] as number));
  });

  it("has its folder's hue inside a folder, one step quieter than its siblings, with a dashed outline", () => {
    const style = tileStyle(merged(2), colors);
    const sibling = tileStyle(
      tiles.find((tile) => tile.depth === 2 && tile.node.name === 'game0') as Tile,
      colors,
    );

    expect(gamesColor).toBeDefined();
    expect(gamesColor).not.toBe(SLATE);
    expect(style.fill).toBe(blend(gamesColor as string, (FLAT_MIX[2] as number) - MERGED_STEP));
    expect(style.fill).not.toBe(blend(SLATE, FLAT_MIX[2] as number));
    expect(style.stroke).toBe(sibling.stroke);
    expect(style.merged).toBe(true);
    expect(sibling.merged).toBe(false);
    // Quieter: closer to the background than its siblings.
    expect(Number.parseInt(style.fill.slice(1, 3), 16)).toBeLessThan(
      Number.parseInt(sibling.fill.slice(1, 3), 16),
    );
  });

  it("is in no tile of a coloured folder slate or grey: its hue is the folder's", () => {
    for (const tile of tiles.filter(
      (candidate) => candidate.depth > 1 && candidate.node.id === COLLAPSED_NODE_ID,
    )) {
      const fill = tileStyle(tile, colors).fill;

      expect(fill).not.toBe(blend(SLATE, FLAT_MIX[tile.depth] as number));
      expect(fill).not.toBe(blend(SLATE, (FLAT_MIX[tile.depth] as number) - MERGED_STEP));
      expect(Math.abs(hue(fill) - hue(blend(gamesColor as string, 50)))).toBeLessThan(6);
    }
  });

  it('keeps the colour of a real folder whose own colour is slate', () => {
    const real = { node: leaf('x', 1).node, depth: 2 as const, top: 7, inner: 8 };

    expect(
      tileStyle(
        real,
        new Map([
          [7, PALETTE[0] as string],
          [8, SLATE],
        ]),
      ).fill,
    ).toBe(blend(SLATE, FLAT_MIX[2] as number));
  });
});
