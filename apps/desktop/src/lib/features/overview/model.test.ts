import { describe, expect, it } from 'vitest';
import type { Api } from '../../api/client';
import { createMockApi, MOCK_VOLUMES } from '../../api/mock';
import type { NodeDto, NodeId } from '../../api/types';
import { createFolderColorResolver } from '../../folderColor';
import { PALETTE, SLATE } from '../../palette';
import { CHILDREN_LIMIT, folderNames, loadOverview } from './model';

describe('loadOverview', () => {
  it('collects the summary, a path for every largest folder, and slabs that add up', async () => {
    const api = createMockApi({ durationMs: 0, now: () => 1_750_000_000 });
    const [volume] = MOCK_VOLUMES;
    const total = volume?.total ?? 0;
    const free = volume?.available ?? 0;

    const id = await new Promise<{ scanId: number; rootId: number }>((resolve) => {
      api.startScan({ root: '/', oneFileSystem: true }, (event) => {
        if (event.type === 'finished') {
          resolve({ scanId: event.scanId, rootId: event.rootId });
        }
      });
    });

    const data = await loadOverview(api, { ...id, total, free });

    expect(data.summary.largestDirs).toHaveLength(8);
    for (const dir of data.summary.largestDirs) {
      expect(data.paths.get(dir.id)).toMatch(/^\//);
    }

    expect(data.slabs.at(-1)).toMatchObject({ free: true, size: free });
    // The root directory's own block is the only thing no slab shows.
    expect(data.slabs.slice(0, -1).reduce((sum, slab) => sum + slab.size, 0)).toBe(
      data.summary.node.allocated - 4096,
    );
    expect(data.slabs[0]?.name).toBe('/home');
    expect(data.slabs[0]?.inside).not.toBe('');
  });
});

const GB = 1_000_000_000;

interface Spec {
  name: string;
  gb: number;
  kids?: Spec[];
  /** A file: no children, and it is not a folder. */
  file?: boolean;
}

/** An `Api` over a hand-made tree: only what the Overview reads. Ids are in listing order, the root is 0. */
function treeApi(rootSpec: Spec): { api: Api; ids: Map<string, NodeId>; childCalls: NodeId[] } {
  const nodes: { dto: NodeDto; parent: number; kids: number[] }[] = [];
  const ids = new Map<string, NodeId>();
  const childCalls: NodeId[] = [];

  const add = (spec: Spec, parent: number, path: string): number => {
    const id = nodes.length;
    const kind = spec.file ? 'file' : 'dir';

    nodes.push({
      parent,
      kids: [],
      dto: {
        id,
        name: spec.name,
        kind,
        allocated: spec.gb * GB,
        apparent: spec.gb * GB,
        files: 1,
        hasChildren: (spec.kids?.length ?? 0) > 0,
        category: 'other',
        mtime: 0,
      },
    });
    ids.set(path, id);
    (spec.kids ?? []).forEach((kid) => {
      (nodes[id] as (typeof nodes)[number]).kids.push(add(kid, id, `${path}/${kid.name}`));
    });

    return id;
  };

  add(rootSpec, -1, '');

  const chain = (id: NodeId): NodeId[] => {
    const node = nodes[id] as (typeof nodes)[number];

    return node.parent < 0 ? [id] : [...chain(node.parent), id];
  };

  const api = {
    async getSummary(_scan: number, id: NodeId) {
      return { node: (nodes[id] as (typeof nodes)[number]).dto, byCategory: [], byAge: [], largestDirs: [] };
    },
    async getChildren(_scan: number, id: NodeId, offset: number, limit: number) {
      // The shorter listings only read the names inside a slab; the long ones look for what to break down.
      if (limit === CHILDREN_LIMIT) {
        childCalls.push(id);
      }

      const items = (nodes[id] as (typeof nodes)[number]).kids
        .map((kid) => (nodes[kid] as (typeof nodes)[number]).dto)
        .sort((a, b) => b.allocated - a.allocated);

      return { parentId: id, total: items.length, items: items.slice(offset, offset + limit) };
    },
    async getPath(_scan: number, id: NodeId) {
      const segments = chain(id).map((entry) => ({
        id: entry,
        name: (nodes[entry] as (typeof nodes)[number]).dto.name,
      }));

      return {
        path: `/${segments
          .slice(1)
          .map((segment) => segment.name)
          .join('/')}`,
        segments,
      };
    },
  } as unknown as Api;

  return { api, ids, childCalls };
}

const request = { scanId: 1, rootId: 0, total: 1000 * GB };

const names = (data: { slabs: { name: string }[] }): string[] => data.slabs.map((slab) => slab.name);

describe('loadOverview breakdown of a dominant folder', () => {
  it('breaks one dominant folder down one level', async () => {
    const { api } = treeApi({
      name: '/',
      gb: 1000,
      kids: [
        {
          name: 'home',
          gb: 700,
          kids: [
            { name: 'a', gb: 300 },
            { name: 'b', gb: 250 },
            { name: 'c', gb: 100 },
            { name: 'd', gb: 50 },
          ],
        },
        { name: 'srv', gb: 300 },
      ],
    });

    expect(names(await loadOverview(api, request))).toEqual([
      '/srv',
      '/home/a',
      '/home/b',
      '/home/c',
      '/home/d',
    ]);
  });

  it('follows a chain of two dominant folders', async () => {
    const { api } = treeApi({
      name: '/',
      gb: 1000,
      kids: [
        {
          name: 'home',
          gb: 900,
          kids: [
            {
              name: 'sam',
              gb: 850,
              kids: [
                { name: 'Videos', gb: 400 },
                { name: 'Projekty', gb: 300 },
                { name: '.cache', gb: 100 },
                { name: 'misc', gb: 50 },
              ],
            },
            { name: 'shared', gb: 50 },
          ],
        },
        { name: 'var', gb: 100 },
      ],
    });
    const data = await loadOverview(api, request);

    expect(names(data)).toEqual([
      '/home/sam/Videos',
      '/home/sam/Projekty',
      '/var',
      '/home/sam/.cache',
      '/home/shared',
      '/home/sam/misc',
    ]);
    // Each folder has its own colour: Videos is the biggest below home and sam, so it has the colour of home
    // (the first), Projekty the next, and var is the second folder of the root, like home's own second.
    expect(data.slabs.slice(0, 3).map((slab) => slab.color)).toEqual([PALETTE[0], PALETTE[1], PALETTE[1]]);
  });

  /** Three dominant folders in a row; the biggest child of the last one is dominant too. */
  const deep: Spec = {
    name: '/',
    gb: 1000,
    kids: [
      {
        name: 'home',
        gb: 950,
        kids: [
          {
            name: 'sam',
            gb: 900,
            kids: [
              {
                name: 'data',
                gb: 800,
                kids: [
                  { name: 'x', gb: 500, kids: [{ name: 'deeper', gb: 480, kids: [{ name: 'f', gb: 100 }] }] },
                  { name: 'y', gb: 200 },
                  { name: 'z', gb: 100 },
                ],
              },
              { name: 'docs', gb: 100 },
            ],
          },
          { name: 'shared', gb: 50 },
        ],
      },
    ],
  };

  it('stops after three levels, and the last dominant folder stays a slab', async () => {
    const { api, ids, childCalls } = treeApi(deep);
    const data = await loadOverview(api, request);

    expect(names(data)).toEqual([
      '/home/sam/data/x',
      '/home/sam/data/y',
      '/home/sam/docs',
      '/home/sam/data/z',
      '/home/shared',
    ]);
    // x holds 62% of data, but a fourth level is not looked into.
    expect(childCalls).not.toContain(ids.get('/home/sam/data/x'));
    expect(data.slabs[0]?.nodeId).toBe(ids.get('/home/sam/data/x'));
  });

  it('keeps the folder as a slab when no folder is dominant', async () => {
    const { api } = treeApi({
      name: '/',
      gb: 1000,
      kids: [
        { name: 'home', gb: 550, kids: [{ name: 'a', gb: 540 }] },
        { name: 'srv', gb: 300 },
        { name: 'var', gb: 150 },
      ],
    });

    expect(names(await loadOverview(api, request))).toEqual(['/home', '/srv', '/var']);
  });

  it('does not break a dominant file down', async () => {
    const { api, childCalls } = treeApi({
      name: '/',
      gb: 1000,
      kids: [
        { name: 'swap.img', gb: 800, file: true },
        { name: 'home', gb: 200, kids: [{ name: 'a', gb: 190 }] },
      ],
    });
    const data = await loadOverview(api, request);

    expect(names(data)).toEqual(['swap.img', '/home']);
    expect(childCalls).toEqual([0]);
  });

  it('keeps a dominant folder whose own children are all too small to be slabs', async () => {
    const { api } = treeApi({
      name: '/',
      gb: 1000,
      kids: [
        {
          name: 'home',
          gb: 900,
          kids: Array.from({ length: 30 }, (_, index) => ({ name: `d${index}`, gb: 30 })),
        },
        { name: 'var', gb: 100 },
      ],
    });

    expect(names(await loadOverview(api, request))).toEqual(['/home', '/var']);
  });

  it('measures dominance against the scanned size, not the volume size', async () => {
    const { api } = treeApi({
      name: '/',
      gb: 100,
      kids: [
        {
          name: 'big',
          gb: 70,
          kids: [
            { name: 'a', gb: 40 },
            { name: 'b', gb: 30 },
          ],
        },
        { name: 'small', gb: 30 },
      ],
    });

    // The volume is 4 times as big: /big is 17% of it, and 70% of what was scanned.
    const data = await loadOverview(api, { ...request, total: 400 * GB });

    expect(names(data)).toEqual(['/big/a', '/small', '/big/b']);
  });

  it('colours "Largest folders" with the colour of the folder itself, the colour its slab has', async () => {
    const { api, ids } = treeApi({
      name: '/',
      gb: 1000,
      kids: [
        {
          name: 'home',
          gb: 900,
          kids: [
            { name: 'a', gb: 600, kids: [{ name: 'inner', gb: 500 }] },
            { name: 'b', gb: 250 },
            { name: 'c', gb: 50 },
          ],
        },
        { name: 'var', gb: 100, kids: [{ name: 'log', gb: 60 }] },
      ],
    });
    const base = api.getSummary.bind(api);

    api.getSummary = async (scan, id) => {
      const summary = await base(scan, id);
      const pick = (path: string): NodeDto => ({ ...summary.node, id: ids.get(path) as NodeId });

      return {
        ...summary,
        largestDirs: ['/home', '/home/a', '/home/a/inner', '/home/b', '/var/log'].map(pick),
      };
    };

    const data = await loadOverview(api, request);
    const colorOf = (path: string): string => data.folderColors.get(ids.get(path) as NodeId) as string;
    const slabColor = (path: string): string =>
      data.slabs.find((slab) => slab.nodeId === ids.get(path))?.color as string;

    // The largest folder below a folder keeps its colour, so a chain of largest folders is one colour.
    expect(colorOf('/home')).toBe(PALETTE[0]);
    expect(colorOf('/home/a')).toBe(PALETTE[0]);
    expect(colorOf('/home/a/inner')).toBe(PALETTE[0]);
    // The others take the palette in order, below their own parent: /home/b is the second in /home (cyan is taken
    // by its parent), /var the second in the root, and /var/log the largest in /var.
    expect(colorOf('/home/b')).toBe(PALETTE[1]);
    expect(colorOf('/var/log')).toBe(PALETTE[1]);

    // A folder that is a slab has the colour of its slab.
    expect(slabColor('/home/a/inner')).toBe(colorOf('/home/a/inner'));
    expect(slabColor('/home/b')).toBe(colorOf('/home/b'));
    expect(slabColor('/var')).toBe(PALETTE[1]);
  });
});

describe('loadOverview on the lopsided mock disk (?dominant)', () => {
  it('breaks /home and the home directory down, and the slab colours match the colours of the folders themselves', async () => {
    const api = createMockApi({ durationMs: 0, now: () => 1_750_000_000, dominant: true });
    const [volume] = MOCK_VOLUMES;

    const id = await new Promise<{ scanId: number; rootId: number }>((resolve) => {
      api.startScan({ root: '/', oneFileSystem: true }, (event) => {
        if (event.type === 'finished') {
          resolve({ scanId: event.scanId, rootId: event.rootId });
        }
      });
    });

    const data = await loadOverview(api, { ...id, total: volume?.total ?? 0, free: volume?.available });
    const folders = data.slabs.filter((slab) => slab.nodeId !== undefined);

    expect(folders.length).toBeGreaterThanOrEqual(4);
    expect(folders.length).toBeLessThanOrEqual(6);
    expect(folders.every((slab) => slab.name.startsWith('/home/sam/') || slab.name === '/home/shared')).toBe(
      true,
    );
    expect(data.slabs.find((slab) => slab.key === 'other')?.color).toBe(SLATE);

    // A slab is coloured like a folder is found to be coloured when asked about from the tree alone.
    const resolver = createFolderColorResolver(api, id.scanId);

    for (const slab of folders) {
      expect(slab.color).toBe(await resolver.colorOf(slab.nodeId as number));
    }
  });
});

describe('what a slab says is inside it', () => {
  const dir = (name: string, id: number): NodeDto => ({
    id,
    name,
    kind: 'dir',
    allocated: 1,
    apparent: 1,
    files: 1,
    hasChildren: true,
    category: 'other',
    mtime: 0,
  });

  it('lists sub-folder names only, biggest first, and leaves files out', () => {
    const file: NodeDto = { ...dir('delta-76.7z', 9), kind: 'file', hasChildren: false };

    expect(folderNames([file, dir('Projekty', 1), file, dir('.local', 2)])).toEqual(['Projekty', '.local']);
    expect(folderNames([file, file])).toEqual([]);
  });

  it('reaches the folders of a slab whose biggest children are archives', async () => {
    const { api } = treeApi({
      name: '/',
      gb: 1000,
      kids: [
        {
          name: 'archives',
          gb: 500,
          kids: [
            { name: 'a.7z', gb: 100, file: true },
            { name: 'b.tar', gb: 90, file: true },
            { name: 'c.zip', gb: 80, file: true },
            { name: 'd.iso', gb: 70, file: true },
            { name: 'old', gb: 60, kids: [{ name: 'x', gb: 50 }] },
          ],
        },
        { name: 'flat', gb: 400, kids: [{ name: 'e.7z', gb: 100, file: true }] },
      ],
    });

    const data = await loadOverview(api, request);

    expect(data.slabs.find((slab) => slab.name === '/archives')?.inside).toBe('old');
    // No sub-folders: no sub-label at all.
    expect(data.slabs.find((slab) => slab.name === '/flat')?.inside).toBe('');
  });
});
