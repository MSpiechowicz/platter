import { afterEach, describe, expect, it } from 'vitest';
import type { NodeDto } from '../../api/types';
import { PALETTE, SLATE } from '../../palette';
import { platform } from '../../state/platform.svelte';
import {
  buildSlabs,
  dominantChild,
  joinNames,
  levelColors,
  MAX_SLABS,
  type Slab,
  type SlabLevel,
} from './slabs';
import { layoutStack, STACK_VIEWBOX } from './stack';

const GB = 1_000_000_000;

function node(id: number, name: string, gb: number, kind: NodeDto['kind'] = 'dir'): NodeDto {
  return {
    id,
    name,
    kind,
    allocated: gb * GB,
    apparent: gb * GB,
    files: 10,
    hasChildren: kind === 'dir',
    category: 'video',
    mtime: 0,
  };
}

const none = (): string[] => [];

const root = (children: NodeDto[]): SlabLevel[] => [{ path: '', children }];

describe('buildSlabs', () => {
  const children = [
    node(1, 'home', 800),
    node(2, 'snapshots', 300),
    node(3, 'usr', 41),
    node(4, 'swap', 34, 'file'),
    node(5, 'opt', 18),
  ];

  it('keeps the big folders, merges the small ones into "other" and closes with free space', () => {
    const slabs = buildSlabs({ levels: root(children), insideOf: none, total: 1900 * GB, free: 500 * GB });

    expect(slabs.map((slab) => slab.name)).toEqual(['/home', '/snapshots', 'other', 'free space']);

    const other = slabs.find((slab) => slab.key === 'other');
    expect(other?.size).toBe(93 * GB);
    expect(other?.inside).toBe('usr · swap · opt');
    expect(other?.nodeId).toBeUndefined();

    expect(slabs.at(-1)).toMatchObject({ free: true, size: 500 * GB });
  });

  it('never gives a slab more than the whole when the folders add up to more than the disk (btrfs)', () => {
    // 1,193 GB of folders and 500 GB free on a 1,000 GB disk.
    const slabs = buildSlabs({ levels: root(children), insideOf: none, total: 1000 * GB, free: 500 * GB });
    const shares = slabs.map((slab) => slab.share);

    expect(Math.max(...shares)).toBeLessThan(1);
    expect(shares.reduce((sum, share) => sum + share, 0)).toBeCloseTo(1, 10);
  });

  it('keeps shares of the disk when the folders fit on it', () => {
    const slabs = buildSlabs({ levels: root(children), insideOf: none, total: 1900 * GB, free: 500 * GB });

    expect(slabs[0]?.share).toBeCloseTo(800 / 1900, 10);
  });

  it('adds no free slab without volume info', () => {
    const slabs = buildSlabs({ levels: root(children), insideOf: none, total: 1200 * GB });

    expect(slabs.some((slab) => slab.free)).toBe(false);
  });

  it('names folders with a slash, files plainly, and links slabs to their node', () => {
    const slabs = buildSlabs({
      levels: root([node(7, 'big.iso', 900, 'file'), node(8, 'docs', 100)]),
      insideOf: none,
      total: 1000 * GB,
    });

    expect(slabs.map((slab) => slab.name)).toEqual(['big.iso', '/docs']);
    expect(slabs.map((slab) => slab.nodeId)).toEqual([7, 8]);
  });

  it('shows what is inside a slab', () => {
    const slabs = buildSlabs({
      levels: root(children),
      insideOf: (id) => (id === 1 ? ['Projekty', '.local', 'Videos', 'Pictures'] : []),
      total: 1900 * GB,
    });

    expect(slabs[0]?.inside).toBe('Projekty · .local · Videos · …');
  });

  it('never makes more than the maximum number of slabs from folders', () => {
    const many = Array.from({ length: 12 }, (_, i) => node(i + 1, `d${i}`, 100 - i));
    const slabs = buildSlabs({ levels: root(many), insideOf: none, total: 1000 * GB });

    expect(slabs.filter((slab) => slab.nodeId !== undefined)).toHaveLength(MAX_SLABS);
    expect(slabs.at(-1)?.key).toBe('other');
  });

  it('skips empty entries', () => {
    expect(buildSlabs({ levels: root([node(1, 'empty', 0)]), insideOf: none, total: GB })).toEqual([]);
  });

  it('joins names with a trailing ellipsis only when some are left out', () => {
    expect(joinNames(['a', 'b'], 3)).toBe('a · b');
    expect(joinNames(['a', 'b', 'c', 'd'], 3)).toBe('a · b · c · …');
  });
});

describe('slab colours', () => {
  const children = [
    node(1, 'home', 800),
    node(2, 'snapshots', 300),
    node(3, 'var', 164),
    node(4, 'swap', 34),
  ];

  it('colours folders by size rank from the Explorer palette, never green', () => {
    const slabs = buildSlabs({ levels: root(children), insideOf: none, total: 1400 * GB });
    const folders = slabs.filter((slab) => slab.nodeId !== undefined);

    expect(folders.map((slab) => slab.color)).toEqual([PALETTE[0], PALETTE[1], PALETTE[2]]);
    expect(new Set(folders.map((slab) => slab.color)).size).toBe(folders.length);
  });

  it('gives a folder the colour of the folder, whatever order the folders arrive in', () => {
    const shuffled = [children[2], children[0], children[3], children[1]] as NodeDto[];
    const slabs = buildSlabs({ levels: root(shuffled), insideOf: none, total: 1400 * GB });
    const colors = levelColors(root(children));

    for (const slab of slabs.filter((entry) => entry.nodeId !== undefined)) {
      expect(slab.color).toBe(colors.get(slab.nodeId as number));
    }
  });

  it('keeps "other" slate and free space as the muted outline', () => {
    const slabs = buildSlabs({ levels: root(children), insideOf: none, total: 1400 * GB, free: 100 * GB });

    expect(slabs.find((slab) => slab.key === 'other')?.color).toBe(SLATE);
    expect(slabs.find((slab) => slab.free)?.color).toBe('var(--muted)');
  });

  it('does not depend on the category of the folder', () => {
    const slabs = buildSlabs({
      levels: root(children.map((child) => ({ ...child, category: 'other' as const }))),
      insideOf: none,
      total: 1400 * GB,
    });

    expect(slabs[0]?.color).toBe(PALETTE[0]);
    expect(slabs[1]?.color).toBe(PALETTE[1]);
  });
});

describe('dominantChild', () => {
  it('is a folder with more than 60% of its parent, and nothing at exactly 60%', () => {
    expect(dominantChild([node(1, 'a', 61), node(2, 'b', 39)], 100 * GB)?.name).toBe('a');
    expect(dominantChild([node(1, 'a', 60), node(2, 'b', 40)], 100 * GB)).toBeUndefined();
  });

  it('is never a file or an empty folder', () => {
    expect(dominantChild([node(1, 'a.iso', 90, 'file')], 100 * GB)).toBeUndefined();
    expect(dominantChild([{ ...node(1, 'a', 90), hasChildren: false }], 100 * GB)).toBeUndefined();
  });
});

describe('buildSlabs with a broken down folder', () => {
  const levels: SlabLevel[] = [
    { path: '', children: [node(1, 'home', 900), node(2, 'var', 100)] },
    { path: '/home', parent: 1, children: [node(3, 'sam', 700), node(4, 'shared', 200)] },
    { path: '/home/sam', parent: 3, children: [node(5, 'Videos', 400), node(6, 'Projekty', 300)] },
  ];

  it('replaces the dominant folders by their children, labelled by path from the scan root', () => {
    const slabs = buildSlabs({ levels, insideOf: none, total: 1000 * GB });

    expect(slabs.map((slab) => slab.name)).toEqual([
      '/home/sam/Videos',
      '/home/sam/Projekty',
      '/home/shared',
      '/var',
    ]);
    expect(slabs.map((slab) => slab.nodeId)).toEqual([5, 6, 4, 2]);
  });

  it('colours a slab as its folder is coloured in the tree: the largest child keeps the colour of its parent', () => {
    const slabs = buildSlabs({ levels, insideOf: none, total: 1000 * GB });
    const colorOf = (id: number): string => slabs.find((slab) => slab.nodeId === id)?.color as string;

    // /home is the biggest folder of the root (cyan), /home/sam the biggest in it, Videos the biggest in that.
    expect(colorOf(5)).toBe(PALETTE[0]);
    // var is the second in the root. shared, the second in /home, skips var's colour, which is used above it.
    expect(colorOf(2)).toBe(PALETTE[1]);
    expect(colorOf(4)).toBe(PALETTE[2]);
    // Projekty, the second in /home/sam, skips the colours of var and shared too: no two slabs look alike.
    expect(colorOf(6)).toBe(PALETTE[3]);
    expect(new Set(slabs.map((slab) => slab.color)).size).toBe(slabs.length);
  });

  it('keeps at most six data slabs and merges the rest into "other"', () => {
    const wide: SlabLevel[] = [
      { path: '', children: [node(1, 'home', 900), node(2, 'var', 100)] },
      {
        path: '/home',
        parent: 1,
        children: Array.from({ length: 10 }, (_, index) => node(10 + index, `d${index}`, 90 - index)),
      },
    ];
    const slabs = buildSlabs({ levels: wide, insideOf: none, total: 1000 * GB });

    expect(slabs.filter((slab) => slab.nodeId !== undefined)).toHaveLength(MAX_SLABS);
    expect(slabs.at(-1)).toMatchObject({ key: 'other', color: SLATE });
    // "var" and d0 to d4 are the six biggest; d5 to d9 are merged, biggest first.
    expect(slabs.at(-1)?.inside).toBe('d5 · d6 · d7 · d8 · …');
  });

  it('shows a file below a folder by its path', () => {
    const slabs = buildSlabs({
      levels: [
        { path: '', children: [node(1, 'home', 900), node(2, 'var', 100)] },
        { path: '/home', parent: 1, children: [node(3, 'swap.img', 500, 'file'), node(4, 'docs', 400)] },
      ],
      insideOf: none,
      total: 1000 * GB,
    });

    expect(slabs.map((slab) => slab.name)).toEqual(['/home/swap.img', '/home/docs', '/var']);
  });
});

describe('levelColors', () => {
  const levels: SlabLevel[] = [
    { path: '', children: [node(1, 'home', 900), node(2, 'var', 60), node(7, 'opt', 40)] },
    { path: '/home', parent: 1, children: [node(3, 'sam', 500), node(4, 'shared', 400)] },
    { path: '/home/sam', parent: 3, children: [node(8, '.local', 300), node(9, '.docker', 150)] },
  ];

  it('colours the folders of the root with the palette, then passes a colour down to the largest child', () => {
    const colors = levelColors(levels);

    expect([1, 2, 7].map((id) => colors.get(id))).toEqual([PALETTE[0], PALETTE[1], PALETTE[2]]);
    expect([3, 8].map((id) => colors.get(id))).toEqual([PALETTE[0], PALETTE[0]]);
    // The others skip the colours used above them: those of var and opt, then of shared too.
    expect(colors.get(4)).toBe(PALETTE[3]);
    expect(colors.get(9)).toBe(PALETTE[4]);
  });
});

describe('layoutStack', () => {
  const slabs = buildSlabs({
    levels: root([node(1, 'home', 800), node(2, 'snapshots', 300), node(3, 'var', 164), node(4, 'usr', 41)]),
    insideOf: none,
    total: 1900 * GB,
    free: 500 * GB,
  });

  it('stacks the first slab at the bottom and thicker slabs are taller', () => {
    const layout = layoutStack(slabs, 1900 * GB);
    const top = (index: number): number => Number(layout.slabs[index]?.top.split(' ')[3]?.split(',')[1]);

    expect(layout.slabs).toHaveLength(slabs.length);
    expect(top(0)).toBeGreaterThan(top(1));
    expect(top(1)).toBeGreaterThan(top(2));
  });

  it('puts labels on alternating sides, anchored away from the stack', () => {
    const layout = layoutStack(slabs, 1900 * GB);

    expect(layout.slabs.map((placed) => placed.text.anchor)).toEqual([
      'start',
      'end',
      'start',
      'end',
      'start',
    ]);
  });

  it('keeps every slab inside the viewBox, even with many slabs', () => {
    const many = Array.from({ length: 8 }, (_, i) => node(i + 1, `d${i}`, 120));
    const layout = layoutStack(
      buildSlabs({ levels: root(many), insideOf: none, total: 1000 * GB, free: 40 * GB }),
      1000 * GB,
    );

    for (const placed of layout.slabs) {
      for (const pair of placed.top.split(' ')) {
        const [x, y] = pair.split(',').map(Number);

        expect(x).toBeGreaterThanOrEqual(0);
        expect(x).toBeLessThanOrEqual(STACK_VIEWBOX.width);
        expect(y).toBeGreaterThanOrEqual(0);
      }
    }
  });

  it('cuts a long path at its start, at a folder boundary, so the folder name stays', () => {
    const named = (name: string) => {
      const slabs = buildSlabs({
        levels: [{ path: '/home/sampson', children: [node(1, name, 10)] }],
        insideOf: none,
        total: 10 * GB,
      });

      return layoutStack(slabs, 10 * GB).slabs[0]?.name;
    };

    expect(named('Downloads')).toBe('…/sampson/Downloads');
    expect(named('a'.repeat(40))).toHaveLength(20);
    expect(named('a'.repeat(40))?.endsWith('aaaa')).toBe(true);
  });

  it('shortens long inside text', () => {
    const long = buildSlabs({
      levels: root([node(1, 'a', 10)]),
      insideOf: () => ['x'.repeat(80)],
      total: 10 * GB,
    });

    expect(layoutStack(long, 10 * GB).slabs[0]?.inside.length).toBeLessThanOrEqual(30);
  });
});

describe('a Windows scan', () => {
  afterEach(() => {
    platform.os = 'linux';
  });

  it('names a slab by its path with backslashes, and cuts the long ones at the start so the folder name stays', () => {
    platform.os = 'windows';

    const levels: SlabLevel[] = [
      { path: '', children: [node(1, 'Users', 700)] },
      { path: '\\Users', parent: 1, children: [node(2, 'demo', 700)] },
      { path: '\\Users\\demo', parent: 2, children: [node(3, 'Downloads', 90), node(4, 'Photos-2019', 400)] },
    ];
    const slabs = buildSlabs({ levels, insideOf: none, total: 1000 * GB, free: 100 * GB });

    expect(slabs.map((slab) => slab.name)).toContain('\\Users\\demo\\Photos-2019');

    const placed = layoutStack(slabs, 1000 * GB).slabs.find((entry) =>
      entry.slab.name.endsWith('Photos-2019'),
    );

    expect(placed?.name).toBe('…\\demo\\Photos-2019');
    expect(placed?.name.length).toBeLessThanOrEqual(20);
  });
});

describe('layoutStack height budget', () => {
  const plain = (size: number, free = false): Slab => ({
    key: `s${size}`,
    name: 'x',
    size,
    share: 0,
    color: SLATE,
    inside: '',
    ...(free ? { free } : {}),
  });

  // The vertical extent of a slab's side face is its height.
  const heightsOf = (layout: ReturnType<typeof layoutStack>): number[] =>
    layout.slabs.map((placed) => {
      const ys = placed.left.split(' ').map((pair) => Number(pair.split(',')[1]));

      return (ys[3] as number) - (ys[0] as number);
    });
  const sum = (values: number[]): number => values.reduce((total, value) => total + value, 0);

  const GAP_TOTAL = (count: number): number => count * 10;

  it('leaves the heights proportional to the disk when every slab is thick enough', () => {
    const slabs = [plain(500 * GB), plain(300 * GB), plain(200 * GB)];
    const heights = heightsOf(layoutStack(slabs, 1000 * GB));

    // The disk is full: 304 units for all of it.
    expect(heights[0]).toBeCloseTo(152, 6);
    expect(heights[1]).toBeCloseTo(91.2, 6);
    expect(heights[2]).toBeCloseTo(60.8, 6);
  });

  it('keeps a half-empty disk shorter than a full one', () => {
    const heights = heightsOf(layoutStack([plain(250 * GB), plain(250 * GB)], 1000 * GB));

    expect(sum(heights)).toBeCloseTo(152, 6);
  });

  it('never grows taller than the budget when the folders add up to more than the disk (btrfs)', () => {
    const slabs = [plain(900 * GB), plain(800 * GB), plain(700 * GB), plain(300 * GB, true)];
    const heights = heightsOf(layoutStack(slabs, 1000 * GB));

    expect(sum(heights)).toBeLessThanOrEqual(304 + 1e-6);
    // They keep their proportions to each other.
    expect((heights[0] as number) / (heights[1] as number)).toBeCloseTo(900 / 800, 6);
  });

  it('never grows taller than the budget with many slabs at the minimum height', () => {
    const slabs = [plain(900 * GB), ...Array.from({ length: 7 }, () => plain(GB))];
    const layout = layoutStack(slabs, 1000 * GB);
    const heights = heightsOf(layout);
    const budget = Math.min(304, 350 - GAP_TOTAL(slabs.length));

    expect(sum(heights)).toBeLessThanOrEqual(budget + 1e-6);
    expect(heights.slice(1).every((height) => height >= 14 - 1e-6)).toBe(true);
    expect(heights[0]).toBeGreaterThan(14);
  });

  it('holds the budget when thin slabs and an overcounted disk come together', () => {
    const slabs = [plain(100 * GB), ...Array.from({ length: 7 }, () => plain(GB))];
    const heights = heightsOf(layoutStack(slabs, 50 * GB));

    expect(sum(heights)).toBeLessThanOrEqual(Math.min(304, 350 - GAP_TOTAL(8)) + 1e-6);
  });

  it('shares a budget too small for the minimum height equally', () => {
    const slabs = Array.from({ length: 30 }, () => plain(GB));
    const heights = heightsOf(layoutStack(slabs, 30 * GB));

    expect(sum(heights)).toBeLessThanOrEqual(Math.max(0, Math.min(304, 350 - GAP_TOTAL(30))) + 1e-6);
  });

  it('gives every slab the minimum when the disk size is unknown', () => {
    const heights = heightsOf(layoutStack([plain(GB), plain(GB)], 0));

    expect(heights.every((height) => height >= 14 - 1e-6)).toBe(true);
  });
});
