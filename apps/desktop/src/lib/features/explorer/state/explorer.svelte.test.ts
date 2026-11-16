import { flushSync, tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { COLLAPSED_NODE_ID, type Api } from '../../../api/client';
import { createMockApi } from '../../../api/mock';
import type { ChildrenPage, LodNode, NodeDto, NodeId, SortKey } from '../../../api/types';
import { PALETTE, SLATE } from '../colors';
import { ExplorerStore, PAGE_SIZE, resolveStart, type ExplorerStart } from './explorer.svelte';

interface Mock {
  api: Api;
  scanId: number;
  rootId: number;
}

async function scanMock(root = '/'): Promise<Mock> {
  const api = createMockApi({ durationMs: 0, now: () => 1_750_000_000 });

  return new Promise((resolve, reject) => {
    api
      .startScan({ root, oneFileSystem: true }, (event) => {
        if (event.type === 'finished') {
          resolve({ api, scanId: event.scanId, rootId: event.rootId });
        } else if (event.type === 'failed' || event.type === 'cancelled') {
          reject(new Error(event.type));
        }
      })
      .catch(reject);
  });
}

const cleanups: (() => void)[] = [];

afterEach(() => {
  cleanups.splice(0).forEach((stop) => stop());
});

/** A store as the page makes it: inside a component-like root, so its loads are live. */
function open(mock: Mock, start: ExplorerStart = { focus: mock.rootId }, api: Api = mock.api): ExplorerStore {
  let store!: ExplorerStore;

  cleanups.push(
    $effect.root(() => {
      store = new ExplorerStore(mock.scanId, mock.rootId, start, api);
    }),
  );
  flushSync();

  return store;
}

/** Waits until the shown folder is `focus` and nothing is loading. */
async function settled(store: ExplorerStore, focus: NodeId = store.focus): Promise<void> {
  await tick();
  await vi.waitFor(() => {
    expect(store.frame.loading).toBe(false);
    expect(store.page.loading).toBe(false);
    expect(store.shown?.focus).toBe(focus);
  });
}

/** Wraps an API so single calls can be held back and released in any order. */
function gated(base: Api) {
  const waiting: { focus: NodeId; release: () => void }[] = [];
  let hold = false;

  const api: Api = {
    ...base,
    getLod: async (scan, node, maxNodes, maxDepth) => {
      if (hold) {
        await new Promise<void>((release) => waiting.push({ focus: node, release }));
      }

      return base.getLod(scan, node, maxNodes, maxDepth);
    },
  };

  return {
    api,
    holdFromNow: () => (hold = true),
    release: (focus: NodeId) => waiting.find((entry) => entry.focus === focus)?.release(),
  };
}

describe('ExplorerStore', () => {
  it('opens on the scan root with its children coloured', async () => {
    const mock = await scanMock();
    const store = open(mock);

    await settled(store);

    expect(store.node?.id).toBe(mock.rootId);
    expect(store.crumbs).toHaveLength(1);
    expect(store.parent).toBeUndefined();
    expect(store.topLevel.length).toBeGreaterThan(3);
    expect(store.items[0]?.id).toBe(store.topLevel[0]?.id);
    expect(store.colorOf(store.topLevel[0]?.id as number)).not.toBe(
      store.colorOf(store.topLevel[1]?.id as number),
    );
  });

  it('opens a folder, goes back one level, then back to the whole scan', async () => {
    const mock = await scanMock();
    const store = open(mock);

    await settled(store);

    const home = store.topLevel.find((node) => node.name === 'var') as NodeDto;

    store.open(home.id);
    await settled(store, home.id);
    expect(store.crumbs.map((crumb) => crumb.id)).toEqual([mock.rootId, home.id]);
    expect(store.parent?.id).toBe(mock.rootId);

    const inner = store.topLevel[0] as NodeDto;

    store.open(inner.id);
    await settled(store, inner.id);
    expect(store.crumbs).toHaveLength(3);

    store.up();
    await settled(store, home.id);
    expect(store.focus).toBe(home.id);

    store.open(inner.id);
    await settled(store, inner.id);
    store.toRoot();
    await settled(store, mock.rootId);
    expect(store.crumbs).toHaveLength(1);

    store.up();
    await tick();
    expect(store.focus).toBe(mock.rootId);
  });

  it('goes up two levels on two quick calls, before the first level has loaded', async () => {
    const mock = await scanMock();
    const slow = gated(mock.api);
    const store = open(mock, { focus: mock.rootId }, slow.api);

    await settled(store);

    const home = store.topLevel.find((node) => node.name === 'var') as NodeDto;

    store.open(home.id);
    await settled(store, home.id);

    const inner = store.topLevel[0] as NodeDto;

    store.open(inner.id);
    await settled(store, inner.id);

    slow.holdFromNow();
    store.up();
    store.up();

    expect(store.focus).toBe(mock.rootId);
    await tick();
    slow.release(mock.rootId);
    await settled(store, mock.rootId);
    expect(store.crumbs).toHaveLength(1);

    store.up();
    expect(store.focus).toBe(mock.rootId);
  });

  it('names the root crumb like the backend and gives every crumb its full path', async () => {
    const mock = await scanMock();
    const store = open(mock);

    await settled(store);

    const home = store.topLevel.find((node) => node.name === 'var') as NodeDto;

    store.open(home.id);
    await settled(store, home.id);

    expect(store.crumbs).toEqual([
      { id: mock.rootId, name: '/', path: '/' },
      { id: home.id, name: home.name, path: `/${home.name}` },
    ]);
  });

  it('first click selects, second click opens; the smaller-items node does nothing', async () => {
    const mock = await scanMock();
    const store = open(mock);

    await settled(store);

    const folder = store.topLevel.find((node) => node.name === 'var') as NodeDto;

    store.choose(folder.id);
    expect(store.selected).toBe(folder.id);
    expect(store.focus).toBe(mock.rootId);
    expect(store.selectedNode?.name).toBe(folder.name);

    store.choose(folder.id);
    expect(store.focus).toBe(folder.id);
    expect(store.selected).toBeNull();
    await settled(store, folder.id);

    store.choose(COLLAPSED_NODE_ID);
    store.select(COLLAPSED_NODE_ID);
    expect(store.selected).not.toBe(COLLAPSED_NODE_ID);
  });

  it('keeps hover and clearing the selection separate from navigation', async () => {
    const mock = await scanMock();
    const store = open(mock);

    await settled(store);

    const [first, second] = store.topLevel as [NodeDto, NodeDto];

    store.hover(first.id);
    expect(store.hovered).toBe(first.id);
    store.hover(COLLAPSED_NODE_ID);
    expect(store.hovered).toBeNull();

    store.select(second.id);
    store.select(null);
    expect(store.selected).toBeNull();
    expect(store.focus).toBe(mock.rootId);
  });

  it('ignores an answer for a folder the user already left', async () => {
    const mock = await scanMock();
    const slow = gated(mock.api);
    const store = open(mock, { focus: mock.rootId }, slow.api);

    await settled(store);

    const [first, second] = store.topLevel as [NodeDto, NodeDto];

    slow.holdFromNow();
    store.open(first.id);
    await tick();
    store.open(second.id);
    await tick();

    // The newer request answers first; the older one arrives late and must not replace it.
    slow.release(second.id);
    await vi.waitFor(() => expect(store.shown?.focus).toBe(second.id));
    slow.release(first.id);
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(store.shown?.focus).toBe(second.id);
    expect(store.crumbs.at(-1)?.id).toBe(second.id);
    expect(store.frame.loading).toBe(false);
  });

  it('keeps showing the old folder while the next one loads', async () => {
    const mock = await scanMock();
    const slow = gated(mock.api);
    const store = open(mock, { focus: mock.rootId }, slow.api);

    await settled(store);
    slow.holdFromNow();

    const target = store.topLevel[1] as NodeDto;

    store.open(target.id);
    await tick();

    expect(store.frame.loading).toBe(true);
    expect(store.shown?.focus).toBe(mock.rootId);

    slow.release(target.id);
    await settled(store, target.id);
  });

  it('shows an error and recovers with retry', async () => {
    const mock = await scanMock();
    let failing = true;
    const flaky: Api = {
      ...mock.api,
      getLod: async (scan, node, maxNodes, maxDepth) => {
        if (failing && node !== mock.rootId) {
          throw new Error('Permission denied');
        }

        return mock.api.getLod(scan, node, maxNodes, maxDepth);
      },
    };
    const store = open(mock, { focus: mock.rootId }, flaky);

    await settled(store);
    store.open(store.topLevel[0]?.id as number);
    await tick();
    await vi.waitFor(() => expect(store.frame.error).toBeInstanceOf(Error));

    failing = false;
    store.retry();
    await tick();
    await vi.waitFor(() => expect(store.frame.error).toBeUndefined());
    expect(store.shown?.focus).toBe(store.focus);
  });
});

describe('the list', () => {
  /** An API whose root has `count` children, served in pages like the real one. */
  function wide(base: Api, count: number): Api {
    const rows = (sort: SortKey): NodeDto[] =>
      Array.from({ length: count }, (_, index) => ({
        id: 1000 + index,
        name: `n${String(index).padStart(4, '0')}`,
        kind: 'file' as const,
        allocated: 10_000 - index,
        apparent: 10_000 - index,
        files: 1,
        hasChildren: false,
        category: 'other' as const,
        mtime: 1_700_000_000 + index,
      })).sort((a, b) =>
        sort === 'nameAsc' ? b.name.localeCompare(a.name) * -1 : b.allocated - a.allocated,
      );

    return {
      ...base,
      getChildren: async (_scan, node, offset, limit, sort): Promise<ChildrenPage> => ({
        parentId: node,
        total: count,
        items: rows(sort).slice(offset, offset + limit),
      }),
    };
  }

  it('loads the next rows on request and stops when everything is in', async () => {
    const mock = await scanMock();
    const store = open(mock, { focus: mock.rootId }, wide(mock.api, PAGE_SIZE * 2 + 50));

    await settled(store);
    expect(store.items).toHaveLength(PAGE_SIZE);
    expect(store.total).toBe(PAGE_SIZE * 2 + 50);
    expect(store.hasMore).toBe(true);

    await store.loadMore();
    expect(store.items).toHaveLength(PAGE_SIZE * 2);
    expect(store.items[PAGE_SIZE]?.name).toBe(`n${String(PAGE_SIZE).padStart(4, '0')}`);

    await store.loadMore();
    expect(store.items).toHaveLength(PAGE_SIZE * 2 + 50);
    expect(store.hasMore).toBe(false);
    expect(new Set(store.items.map((item) => item.id)).size).toBe(store.items.length);

    await store.loadMore();
    expect(store.items).toHaveLength(PAGE_SIZE * 2 + 50);
  });

  it('only selects a file, however often it is clicked', async () => {
    const mock = await scanMock();
    const store = open(mock, { focus: mock.rootId }, wide(mock.api, 5));

    await settled(store);

    const file = store.items[0] as NodeDto;

    store.choose(file.id);
    store.choose(file.id);

    expect(store.selected).toBe(file.id);
    expect(store.focus).toBe(mock.rootId);
  });

  it('starts over when the sort changes, and drops rows loaded for the old sort', async () => {
    const mock = await scanMock();
    const store = open(mock, { focus: mock.rootId }, wide(mock.api, PAGE_SIZE + 30));

    await settled(store);
    await store.loadMore();
    expect(store.items).toHaveLength(PAGE_SIZE + 30);

    store.setSort('nameAsc');
    await tick();
    await vi.waitFor(() => expect(store.page.loading).toBe(false));

    expect(store.sort).toBe('nameAsc');
    expect(store.items).toHaveLength(PAGE_SIZE);
    expect(store.hasMore).toBe(true);
    expect(store.items[0]?.name).toBe('n0000');
  });

  it('does not add rows that arrive after the folder changed', async () => {
    const mock = await scanMock();
    let release: () => void = () => {};
    const base = wide(mock.api, PAGE_SIZE + 30);
    const api: Api = {
      ...base,
      getChildren: async (scan, node, offset, limit, sort) => {
        if (offset > 0) {
          await new Promise<void>((resolve) => (release = resolve));
        }

        return base.getChildren(scan, node, offset, limit, sort);
      },
    };
    const store = open(mock, { focus: mock.rootId }, api);

    await settled(store);

    const pending = store.loadMore();

    store.setSort('nameAsc');
    await tick();
    release();
    await pending;
    await vi.waitFor(() => expect(store.page.loading).toBe(false));

    expect(store.items).toHaveLength(PAGE_SIZE);
  });

  describe('"Load more" while the sort changes', () => {
    it('refuses to load more until the page for the new sort has arrived', async () => {
      const mock = await scanMock();
      const base = wide(mock.api, PAGE_SIZE + 30);
      const asked: { offset: number; sort: SortKey }[] = [];
      let releaseFirstPage: () => void = () => {};
      let hold = false;
      const api: Api = {
        ...base,
        getChildren: async (scan, node, offset, limit, sort) => {
          asked.push({ offset, sort });

          if (hold && offset === 0) {
            await new Promise<void>((resolve) => (releaseFirstPage = resolve));
          }

          return base.getChildren(scan, node, offset, limit, sort);
        },
      };
      const store = open(mock, { focus: mock.rootId }, api);

      await settled(store);
      expect(store.pageCurrent).toBe(true);
      asked.length = 0;
      hold = true;

      // The sort changed, but the page in `store.page.data` is still the one for the old sort.
      store.setSort('nameAsc');
      expect(store.pageCurrent).toBe(false);

      await store.loadMore();

      expect(asked.filter((request) => request.offset > 0)).toEqual([]);
      expect(store.items).toHaveLength(PAGE_SIZE);
      expect(store.loadingMore).toBe(false);

      await tick();
      releaseFirstPage();
      await vi.waitFor(() => expect(store.page.loading).toBe(false));

      expect(store.pageCurrent).toBe(true);

      await store.loadMore();
      expect(asked.filter((request) => request.offset > 0)).toEqual([{ offset: PAGE_SIZE, sort: 'nameAsc' }]);
      expect(store.items).toHaveLength(PAGE_SIZE + 30);
    });

    it('is not current while the next folder loads, and is again once its page has arrived', async () => {
      const mock = await scanMock();
      const store = open(mock);

      await settled(store);

      const home = store.topLevel.find((node) => node.name === 'var') as NodeDto;

      store.open(home.id);
      expect(store.pageCurrent).toBe(false);

      await settled(store, home.id);
      expect(store.pageCurrent).toBe(true);
    });
  });
});

describe('the list while the user moves on', () => {
  function wideApi(
    base: Api,
    count: number,
    hook: (node: NodeId, offset: number) => Promise<void> = async () => {},
  ): Api {
    const rows: NodeDto[] = Array.from({ length: count }, (_, index) => ({
      id: 1000 + index,
      name: `n${String(index).padStart(4, '0')}`,
      kind: 'file',
      allocated: 10_000 - index,
      apparent: 10_000 - index,
      files: 1,
      hasChildren: false,
      category: 'other',
      mtime: 1_700_000_000 + index,
    }));

    return {
      ...base,
      getChildren: async (_scan, node, offset, limit): Promise<ChildrenPage> => {
        await hook(node, offset);

        return { parentId: node, total: count, items: rows.slice(offset, offset + limit) };
      },
    };
  }

  it('clears a "load more" error when the user opens another folder or changes the sort', async () => {
    const mock = await scanMock();
    let failing = true;
    const api = wideApi(mock.api, PAGE_SIZE + 30, async (_node, offset) => {
      if (failing && offset > 0) {
        throw new Error('Read error');
      }
    });
    const store = open(mock, { focus: mock.rootId }, api);

    await settled(store);
    await store.loadMore();
    expect(store.moreError).toBe('Read error');

    store.setSort('nameAsc');
    expect(store.moreError).toBeNull();
    await settled(store);

    await store.loadMore();
    expect(store.moreError).toBe('Read error');

    store.open(store.topLevel[0]?.id as number);
    expect(store.moreError).toBeNull();

    failing = false;
    await settled(store);
    await store.loadMore();
    expect(store.moreError).toBeNull();
    expect(store.items).toHaveLength(PAGE_SIZE + 30);
  });

  it('loads more in a new folder while the request of the folder it left is still pending', async () => {
    const mock = await scanMock();
    const held: (() => void)[] = [];
    const api = wideApi(mock.api, PAGE_SIZE + 30, async (node, offset) => {
      if (offset > 0 && node === mock.rootId) {
        await new Promise<void>((release) => held.push(release));
      }
    });
    const store = open(mock, { focus: mock.rootId }, api);

    await settled(store);

    const leaving = store.loadMore();

    expect(store.loadingMore).toBe(true);

    store.open(store.topLevel[0]?.id as number);
    expect(store.loadingMore).toBe(false);
    await settled(store);

    await store.loadMore();
    expect(store.items).toHaveLength(PAGE_SIZE + 30);

    held.forEach((release) => release());
    await leaving;

    // The late answer for the folder that was left neither adds rows nor switches the spinner.
    expect(store.items).toHaveLength(PAGE_SIZE + 30);
    expect(store.loadingMore).toBe(false);
  });

  it('colours a list row that the map does not draw by itself in slate, the colour of "N smaller items"', async () => {
    const mock = await scanMock();
    const store = open(mock, { focus: mock.rootId }, wideApi(mock.api, 5));

    await settled(store);

    const biggest = store.topLevel[0] as NodeDto;

    expect(store.colorOf(biggest.id)).toBe(PALETTE[0]);
    expect(store.colorOf(COLLAPSED_NODE_ID)).toBe(SLATE);
    expect(store.items.map((row) => store.colorOf(row.id))).toEqual(store.items.map(() => SLATE));
  });
});

describe('resolveStart', () => {
  it('opens a folder on itself and the root when nothing is asked for', async () => {
    const mock = await scanMock();
    const folder = (await mock.api.getChildren(mock.scanId, mock.rootId, 0, 5, 'sizeDesc'))
      .items[0] as NodeDto;

    expect(await resolveStart(mock.api, mock.scanId, mock.rootId, undefined)).toEqual({ focus: mock.rootId });
    expect(await resolveStart(mock.api, mock.scanId, mock.rootId, folder.id)).toEqual({ focus: folder.id });
  });

  it('opens a file on its folder with the file selected', async () => {
    const mock = await scanMock();
    const lod = await mock.api.getLod(mock.scanId, mock.rootId, 400, 6);
    const find = (item: LodNode, parent: NodeId): [NodeDto, NodeId] | undefined => {
      if (item.node.kind === 'file') {
        return [item.node, parent];
      }

      for (const child of item.children) {
        const found = find(child, item.node.id);

        if (found) {
          return found;
        }
      }

      return undefined;
    };
    const [file, parent] = find(lod, mock.rootId) as [NodeDto, NodeId];

    expect(await resolveStart(mock.api, mock.scanId, mock.rootId, file.id)).toEqual({
      focus: parent,
      select: file.id,
    });
  });

  it('falls back to the root for a node that no longer exists', async () => {
    const mock = await scanMock();

    expect(await resolveStart(mock.api, mock.scanId, mock.rootId, 99_999_999)).toEqual({
      focus: mock.rootId,
    });
  });

  it('selects the file once its folder is shown', async () => {
    const mock = await scanMock();
    const folder = (await mock.api.getChildren(mock.scanId, mock.rootId, 0, 5, 'sizeDesc'))
      .items[0] as NodeDto;
    const child = (await mock.api.getChildren(mock.scanId, folder.id, 0, 5, 'sizeDesc')).items[0] as NodeDto;
    const store = open(mock, { focus: folder.id, select: child.id });

    await settled(store, folder.id);

    expect(store.selectedNode?.id).toBe(child.id);
  });
});

/** What the list is told when one child holds most of the folder, and what the "smaller items" tile selects. */
describe('ExplorerStore on a skewed disk', () => {
  it('points at the child that holds more than 85% of the folder, and opens it on request', async () => {
    const mock = await scanMock('/skew');
    const store = open(mock);

    await settled(store);

    expect(store.dominant?.node.name).toBe('home');
    expect(store.dominant?.percent).toBe(97);

    store.open(store.dominant?.node.id as NodeId);
    await settled(store, store.focus);

    expect(store.node?.name).toBe('home');
    // Inside home no child holds that much, so the hint is gone.
    expect(store.dominant).toBeUndefined();
  });

  it('has no hint on a balanced folder', async () => {
    const store = open(await scanMock());

    await settled(store);

    expect(store.dominant).toBeUndefined();
  });

  /** A store over a folder whose children have the given sizes, for the 85% line. */
  async function folderOf(sizes: number[], kind: 'dir' | 'file' = 'dir'): Promise<ExplorerStore> {
    const mock = await scanMock();
    const base = mock.api;
    const children = sizes.map((allocated, index) => ({
      node: {
        id: 100 + index,
        name: `c${index}`,
        kind,
        allocated,
        apparent: allocated,
        files: 1,
        hasChildren: kind === 'dir',
        category: 'other' as const,
        mtime: 1,
      },
      children: [],
    }));
    const api: Api = {
      ...base,
      getLod: async (scan, node, maxNodes, maxDepth) => ({
        ...(await base.getLod(scan, node, maxNodes, maxDepth)),
        children,
      }),
    };
    const store = open(mock, { focus: mock.rootId }, api);

    await settled(store);

    return store;
  }

  it('says nothing at 85% or under, and speaks above it', async () => {
    expect((await folderOf([85, 10, 5])).dominant).toBeUndefined();
    expect((await folderOf([86, 10, 4])).dominant?.percent).toBe(86);
  });

  it('does not point at a file, which cannot be opened', async () => {
    expect((await folderOf([99, 1], 'file')).dominant).toBeUndefined();
  });

  it('selects the "smaller items" tile with the rows it stands for highlighted, and never opens it', async () => {
    const mock = await scanMock('/skew');
    const store = open(mock);

    await settled(store);

    const small = store.items.filter((item) => item.allocated > 0).slice(-6);
    const tile: NodeDto = {
      ...(small[0] as NodeDto),
      id: COLLAPSED_NODE_ID,
      name: '6 smaller items',
      kind: 'collapsed',
      hasChildren: false,
    };

    store.chooseGroup(
      tile,
      small.map((item) => item.id),
    );

    expect(store.selected).toBe(COLLAPSED_NODE_ID);
    expect(store.groupSelected).toBe(true);
    expect(store.selectedNode?.name).toBe('6 smaller items');
    expect([...store.highlighted].sort()).toEqual(small.map((item) => item.id).sort());
    expect(store.items.filter((item) => store.highlighted.has(item.id))).toHaveLength(6);

    // Choosing it again does not open anything.
    store.choose(COLLAPSED_NODE_ID);
    expect(store.focus).toBe(mock.rootId);
    expect(store.selected).toBe(COLLAPSED_NODE_ID);

    // A real child takes the selection, and the highlight goes.
    store.choose(store.topLevel[0]?.id as NodeId);
    expect(store.groupSelected).toBe(false);
    expect(store.highlighted.size).toBe(0);
  });

  it('forgets the selected tile when the selection is cleared or another folder is opened', async () => {
    const mock = await scanMock('/skew');
    const store = open(mock);

    await settled(store);

    const tile = { ...(store.items[0] as NodeDto), id: COLLAPSED_NODE_ID, kind: 'collapsed' as const };

    store.chooseGroup(tile, [store.items[1]?.id as NodeId]);
    store.select(null);
    expect(store.selected).toBeNull();
    expect(store.highlighted.size).toBe(0);

    store.chooseGroup(tile, [store.items[1]?.id as NodeId]);
    store.open(store.topLevel[0]?.id as NodeId);
    expect(store.groupSelected).toBe(false);
    expect(store.highlighted.size).toBe(0);
  });

  it('highlights also the rows the backend left out of the map', async () => {
    const mock = await scanMock('/skew');
    const base = mock.api;
    // The map knows only the first three children and one "smaller items" node for the rest.
    const api: Api = {
      ...base,
      getLod: async (scan, node, maxNodes, maxDepth) => {
        const full = await base.getLod(scan, node, maxNodes, maxDepth);

        return { ...full, children: full.children.slice(0, 3) };
      },
    };
    const store = open(mock, { focus: mock.rootId }, api);

    await settled(store);

    const tile = { ...(store.items[0] as NodeDto), id: COLLAPSED_NODE_ID, kind: 'collapsed' as const };

    store.chooseGroup(tile, []);

    expect([...store.highlighted].sort()).toEqual(
      store.items
        .slice(3)
        .map((item) => item.id)
        .sort(),
    );
  });
});

/** Items without a size are not in the list; they are counted in one footer row. */
describe('ExplorerStore and items that are not counted', () => {
  function child(id: number, name: string, allocated: number): NodeDto {
    return {
      id,
      name,
      kind: allocated > 0 ? 'file' : 'dir',
      allocated,
      apparent: allocated,
      files: 0,
      hasChildren: false,
      category: 'other',
      mtime: 1,
    };
  }

  /** `sizes.length` children served in pages like the backend does, sorted as asked. */
  async function pagedStore(sizes: number[]): Promise<ExplorerStore> {
    const mock = await scanMock();
    const base = mock.api;
    const all = sizes.map((size, index) =>
      child(1000 + index, `item-${String(index).padStart(3, '0')}`, size),
    );
    const api: Api = {
      ...base,
      getChildren: async (_scan, node, offset, limit, sort) => {
        const sorted = [...all].sort((a, b) =>
          sort === 'nameAsc'
            ? a.name < b.name
              ? -1
              : 1
            : b.allocated - a.allocated || (a.name < b.name ? -1 : 1),
        );

        return { parentId: node, total: sorted.length, items: sorted.slice(offset, offset + limit) };
      },
    };
    const store = open(mock, { focus: mock.rootId }, api);

    await settled(store);

    return store;
  }

  const loadAll = async (store: ExplorerStore): Promise<void> => {
    while (store.hasMore) {
      await store.loadMore();
    }
  };

  it('keeps them out of the rows, counts the others in the header, and lists them by name', async () => {
    const store = await pagedStore([50, 0, 30, 0, 0, 10]);

    expect(store.sizedItems.map((item) => item.allocated)).toEqual([50, 30, 10]);
    expect(store.sizedTotal).toBe(3);
    expect(store.uncountedTotal).toBe(3);
    expect(store.uncountedItems.map((item) => item.name)).toEqual(['item-001', 'item-003', 'item-004']);
  });

  it('knows the count at once when sorted by size and the first page reaches the zero rows', async () => {
    // 150 with a size, 120 without: the first page of 200 holds the 50 first of the tail.
    const store = await pagedStore([...Array(150).fill(5), ...Array(120).fill(0)]);

    expect(store.hasMore).toBe(true);
    expect(store.uncountedTotal).toBe(120);
    expect(store.sizedTotal).toBe(150);
  });

  it('does not know the count while pages are missing, when sorted by size and no zero row has come yet', async () => {
    const store = await pagedStore([...Array(300).fill(5), ...Array(10).fill(0)]);

    expect(store.uncountedTotal).toBeUndefined();
    // The most it can be: everything not yet seen as empty.
    expect(store.sizedTotal).toBe(310);

    await loadAll(store);

    expect(store.uncountedTotal).toBe(10);
    expect(store.sizedTotal).toBe(300);
  });

  it('collects them from every page when sorted by name, and counts them once the last page is in', async () => {
    // Every fourth item has no size, so they are spread over all pages of 200.
    const sizes = Array.from({ length: 450 }, (_, index) => (index % 4 === 0 ? 0 : 7));
    const store = await pagedStore(sizes);

    store.setSort('nameAsc');
    await settled(store);

    expect(store.hasMore).toBe(true);
    expect(store.uncountedTotal).toBeUndefined();
    expect(store.uncountedItems.length).toBe(50);

    await loadAll(store);

    expect(store.uncountedTotal).toBe(113);
    expect(store.sizedTotal).toBe(337);
    expect(store.sizedItems.every((item) => item.allocated > 0)).toBe(true);

    const names = store.uncountedItems.map((item) => item.name);

    expect(names).toHaveLength(113);
    expect(names).toEqual([...names].sort());
  });

  it('has nothing to count in a folder where every item has a size', async () => {
    const store = await pagedStore([3, 2, 1]);

    expect(store.uncountedTotal).toBe(0);
    expect(store.uncountedItems).toEqual([]);
  });
});
