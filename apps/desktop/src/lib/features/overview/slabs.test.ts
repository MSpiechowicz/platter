import { describe, expect, it } from 'vitest';
import type { NodeDto } from '../../api/types';
import { PALETTE, SLATE } from '../../palette';
import { buildSlabs, dominantChild, joinNames, levelColors, MAX_SLABS, type SlabLevel } from './slabs';
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
    // Projekty is the second in /home/sam, so it takes the first palette colour that is not its parent's.
    expect(colorOf(6)).toBe(PALETTE[1]);
    // shared is the second in /home; var is the second in the root.
    expect(colorOf(4)).toBe(PALETTE[1]);
    expect(colorOf(2)).toBe(PALETTE[1]);
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
    expect(colors.get(4)).toBe(PALETTE[1]);
    expect(colors.get(9)).toBe(PALETTE[1]);
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
