import { describe, expect, it, vi } from 'vitest';
import { COLLAPSED_NODE_ID, type Api } from './api/client';
import { createMockApi } from './api/mock';
import type { NodeDto, ScanEvent, ScanId } from './api/types';
import { childColors, COLOR_CHILDREN, createFolderColorResolver, usedBelow } from './folderColor';
import { PALETTE, SLATE } from './palette';

const kids = (...sizes: number[]) => sizes.map((allocated, index) => ({ id: index + 1, allocated }));

describe('childColors', () => {
  it('gives the children of the scan root the palette colours in order, biggest first', () => {
    const colors = childColors(null, kids(10, 50, 30));

    expect([colors.get(2), colors.get(3), colors.get(1)]).toEqual([PALETTE[0], PALETTE[1], PALETTE[2]]);
  });

  it('lets the largest child keep the colour of its parent', () => {
    const parent = PALETTE[3] as string;
    const colors = childColors(parent, kids(10, 50, 30));

    expect(colors.get(2)).toBe(parent);
  });

  it('gives the others the palette in order and skips the colour of the parent', () => {
    const parent = PALETTE[0] as string;
    const colors = childColors(parent, kids(10, 50, 30, 20));

    expect([colors.get(3), colors.get(4), colors.get(1)]).toEqual([PALETTE[1], PALETTE[2], PALETTE[3]]);
    expect([...colors.values()].filter((color) => color === parent)).toEqual([parent]);
  });

  it('uses the whole palette below a slate parent, and the largest child stays slate', () => {
    const colors = childColors(SLATE, kids(80, 70, 60));

    expect([colors.get(1), colors.get(2), colors.get(3)]).toEqual([SLATE, PALETTE[0], PALETTE[1]]);
  });

  it('turns slate when the palette is used up', () => {
    const colors = childColors(
      null,
      Array.from({ length: 11 }, (_, index) => ({ id: index + 1, allocated: 100 - index })),
    );

    expect([...colors.values()].slice(0, 8)).toEqual([...PALETTE]);
    expect([...colors.values()].slice(8)).toEqual([SLATE, SLATE, SLATE]);

    // Below a parent with a palette colour, seven colours are left after the one it passes on.
    const below = childColors(
      PALETTE[0] as string,
      Array.from({ length: 10 }, (_, index) => ({ id: index + 1, allocated: 100 - index })),
    );

    expect([...below.values()]).toEqual([PALETTE[0], ...PALETTE.slice(1), SLATE, SLATE]);
  });

  it('reuses the colour of the smallest folder above when the palette is used up (the macOS root)', () => {
    // The macOS root: Users, Applications, System, Library, opt, private, usr, then tiny ones. Every colour is used.
    const GB = 1e9;
    const root = kids(
      480 * GB,
      67 * GB,
      49 * GB,
      30 * GB,
      8.6 * GB,
      8.4 * GB,
      1.7 * GB,
      0.01 * GB,
      0.001 * GB,
    );
    const rootColors = childColors(null, root);
    const users = rootColors.get(1) as string;
    const applications = rootColors.get(2) as string;
    const above = usedBelow([], rootColors, root);

    // Inside /Users/sam (cyan, like /Users): Library, Projects, Parallels.
    const home = childColors(users, kids(255 * GB, 147 * GB, 20 * GB), above);

    expect(home.get(1)).toBe(users);
    expect(home.get(2)).not.toBe(applications);
    // Projects takes the colour of the smallest folder of the root that has one, then Parallels the next smallest.
    expect(home.get(2)).toBe(rootColors.get(8));
    expect(home.get(3)).toBe(rootColors.get(7));
  });

  it('breaks ties by id, whatever order the children come in', () => {
    const colors = childColors(null, [
      { id: 9, allocated: 5 },
      { id: 3, allocated: 5 },
      { id: 6, allocated: 5 },
    ]);

    expect([colors.get(3), colors.get(6), colors.get(9)]).toEqual([PALETTE[0], PALETTE[1], PALETTE[2]]);
  });

  it('leaves zero-size children and the "smaller items" node slate, without using up a colour', () => {
    const colors = childColors(null, [
      { id: 1, allocated: 0 },
      { id: COLLAPSED_NODE_ID, allocated: 500 },
      { id: 2, allocated: 40 },
      { id: 3, allocated: 30 },
    ]);

    expect(colors.get(1)).toBe(SLATE);
    expect(colors.get(COLLAPSED_NODE_ID)).toBe(SLATE);
    expect([colors.get(2), colors.get(3)]).toEqual([PALETTE[0], PALETTE[1]]);
  });

  it('does not let a parent hand its colour to a zero-size child', () => {
    expect(childColors(PALETTE[2] as string, kids(0)).get(1)).toBe(SLATE);
    expect(childColors(null, []).size).toBe(0);
  });
});

function scan(api: Api): Promise<ScanId> {
  return new Promise((resolve, reject) => {
    api
      .startScan({ root: '/', oneFileSystem: true }, (event: ScanEvent) => {
        if (event.type === 'finished') {
          resolve(event.scanId);
        }
      })
      .catch(reject);
  });
}

async function children(api: Api, scanId: ScanId, id: number): Promise<NodeDto[]> {
  return (await api.getChildren(scanId, id, 0, 1000, 'sizeDesc')).items;
}

describe('createFolderColorResolver', () => {
  const setup = async () => {
    const api = createMockApi({ durationMs: 0, now: () => 1_750_000_000 });
    const scanId = await scan(api);

    return { api, scanId };
  };

  it('colours the folders of the scan root with the palette, biggest first, and the root has no colour', async () => {
    const { api, scanId } = await setup();
    const resolver = createFolderColorResolver(api, scanId);
    const top = (await children(api, scanId, 0)).filter((node) => node.allocated > 0);

    for (const [index, node] of top.slice(0, 8).entries()) {
      expect(await resolver.colorOf(node.id)).toBe(PALETTE[index]);
    }

    expect(await resolver.colorOf(0)).toBe(SLATE);
    // The root's slate is a stand-in: it must not be handed down as the colour of a parent.
    expect(await resolver.colorOf((top[0] as NodeDto).id)).toBe(PALETTE[0]);
  });

  it('passes the colour of a folder on to its largest child, however deep it is asked for', async () => {
    const { api, scanId } = await setup();
    const home = (await children(api, scanId, 0))[0] as NodeDto;
    const demo = (await children(api, scanId, home.id))[0] as NodeDto;
    const inDemo = await children(api, scanId, demo.id);
    const largest = inDemo[0] as NodeDto;
    const second = inDemo[1] as NodeDto;
    const deepest = (await children(api, scanId, largest.id))[0] as NodeDto;

    // Asked about the deep folder first, so nothing above it is known yet.
    const resolver = createFolderColorResolver(api, scanId);

    expect(await resolver.colorOf(deepest.id)).toBe(PALETTE[0]);
    expect(await resolver.colorOf(largest.id)).toBe(PALETTE[0]);
    // The second takes the first colour that no folder above it uses: not those of the root's other children.
    const top = await children(api, scanId, 0);
    const aboveColors = childColors(null, top);
    const expected = PALETTE.find((color) => ![...aboveColors.values()].includes(color));
    expect(await resolver.colorOf(second.id)).toBe(expected);
  });

  it('gives the same answers whatever order the folders are asked in', async () => {
    const { api, scanId } = await setup();
    const home = (await children(api, scanId, 0))[0] as NodeDto;
    const demo = (await children(api, scanId, home.id))[0] as NodeDto;
    const ids = (await children(api, scanId, demo.id)).slice(0, 5).map((node) => node.id);
    const forward = createFolderColorResolver(api, scanId);
    const backward = createFolderColorResolver(api, scanId);

    const first = await Promise.all(ids.map((id) => forward.colorOf(id)));
    const last = await Promise.all([...ids].reverse().map((id) => backward.colorOf(id)));

    expect(first).toEqual(last.reverse());
  });

  it('makes one request per ancestor, however many folders are asked about at once', async () => {
    const { api, scanId } = await setup();
    const home = (await children(api, scanId, 0))[0] as NodeDto;
    const demo = (await children(api, scanId, home.id))[0] as NodeDto;
    const ids = (await children(api, scanId, demo.id)).slice(0, 5).map((node) => node.id);
    const getChildren = vi.spyOn(api, 'getChildren');
    const resolver = createFolderColorResolver(api, scanId);

    await Promise.all(ids.map((id) => resolver.colorOf(id)));
    await Promise.all(ids.map((id) => resolver.colorOf(id)));

    const parents = getChildren.mock.calls.map((call) => call[1]);

    expect(parents).toEqual([0, home.id, demo.id]);
    expect(getChildren.mock.calls.every((call) => call[3] === COLOR_CHILDREN && call[4] === 'sizeDesc')).toBe(
      true,
    );
  });

  it('needs no lookup for a folder whose colour was seeded, and never replaces a known colour', async () => {
    const { api, scanId } = await setup();
    const resolver = createFolderColorResolver(api, scanId);
    const getPath = vi.spyOn(api, 'getPath');
    const home = (await children(api, scanId, 0))[0] as NodeDto;

    resolver.seed(home.id, '#123456');
    resolver.seed(home.id, '#abcdef');

    expect(await resolver.colorOf(home.id)).toBe('#123456');
    expect(getPath).not.toHaveBeenCalled();

    // Folders below it start from the seeded colour: the largest one inherits it.
    const largest = (await children(api, scanId, home.id))[0] as NodeDto;

    expect(await resolver.colorOf(largest.id)).toBe('#123456');
  });

  it('remembers the colours it hands out for children', async () => {
    const { api, scanId } = await setup();
    const resolver = createFolderColorResolver(api, scanId);
    const getPath = vi.spyOn(api, 'getPath');
    const top = await children(api, scanId, 0);
    const colors = resolver.childrenOf(0, null, top);

    expect(colors.get((top[0] as NodeDto).id)).toBe(PALETTE[0]);
    expect(await resolver.colorOf((top[1] as NodeDto).id)).toBe(PALETTE[1]);
    expect(getPath).not.toHaveBeenCalled();
  });

  it('makes a folder that is not among the biggest children slate', async () => {
    const { api, scanId } = await setup();
    const usr = (await children(api, scanId, 0)).find((node) => node.name === 'usr') as NodeDto;
    const real = api.getChildren.bind(api);
    // An API that only ever reports the first child of a folder.
    const narrow: Api = {
      ...api,
      getChildren: async (...args) => {
        const page = await real(...args);

        return { ...page, items: page.items.slice(0, 1) };
      },
    };

    expect(await createFolderColorResolver(narrow, scanId).colorOf(usr.id)).toBe(SLATE);
  });

  it('retries a failed lookup instead of remembering the failure', async () => {
    const { api, scanId } = await setup();
    const top = (await children(api, scanId, 0))[1] as NodeDto;
    const real = api.getChildren.bind(api);
    let fail = true;
    const flaky: Api = {
      ...api,
      getChildren: async (...args) => {
        if (fail) {
          throw new Error('offline');
        }

        return real(...args);
      },
    };
    const resolver = createFolderColorResolver(flaky, scanId);

    await expect(resolver.colorOf(top.id)).rejects.toThrow('offline');
    fail = false;

    expect(await resolver.colorOf(top.id)).toBe(PALETTE[1]);
  });
});

/**
 * A chain of folders. Every folder has two children: the next folder of the chain, and a bigger sibling, so the chain
 * never takes the colour of its parent and its colour changes at every level.
 */
function chainApi(depth: number): Api {
  const SIBLING = 1000;
  const node = (id: number, allocated: number): NodeDto => ({
    id,
    name: `d${id}`,
    kind: 'dir',
    allocated,
    apparent: 0,
    files: 0,
    hasChildren: id < depth,
    category: 'other',
    mtime: 0,
  });

  return {
    getPath: async (_scan: ScanId, id: number) => ({
      path: '/',
      segments: Array.from({ length: id + 1 }, (_, index) => ({ id: index, name: `d${index}` })),
    }),
    getChildren: async (_scan: ScanId, id: number) => ({
      parentId: id,
      total: 2,
      items: id < depth ? [node(SIBLING + id + 1, 200), node(id + 1, 100)] : [],
    }),
  } as unknown as Api;
}

describe('folders deep in the tree', () => {
  const DEPTH = 20;

  it('have the same colour from a deep link as from walking down one folder at a time', async () => {
    const api = chainApi(DEPTH);
    const walking = createFolderColorResolver(api, 1);
    const walked: string[] = [];
    let color: string | null = null;

    for (let id = 0; id < DEPTH; id++) {
      const page = await api.getChildren(1, id, 0, COLOR_CHILDREN, 'sizeDesc');
      const colors = walking.childrenOf(id, color, page.items);

      color = colors.get(id + 1) as string;
      walked.push(color);
    }

    // Each deep link comes from a resolver of its own, so nothing is shared with the walk.
    for (const level of [5, 12, 13, 14, DEPTH]) {
      const linked = await createFolderColorResolver(api, 1).colorOf(level);

      expect(linked, `level ${level}`).toBe(walked[level - 1]);
    }

    // The colour keeps changing past level 12, so this is not a cap that happens to agree.
    expect(new Set(walked.slice(12)).size).toBeGreaterThan(1);
  });
});
