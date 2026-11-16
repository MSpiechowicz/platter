import { flushSync, tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { COLLAPSED_NODE_ID, type Api } from '../../../api/client';
import { createMockApi } from '../../../api/mock';
import type { ChildrenPage, NodeDto, NodeId } from '../../../api/types';
import { PALETTE } from '../colors';
import { SEARCH_LIMIT } from '../search';
import { ExplorerStore, EXPANSION_DEPTH, PAGE_SIZE, type ExplorerStart } from './explorer.svelte';
import { MAX_CHAIN_EXPANSIONS } from '../layout/chains';

interface Mock {
  api: Api;
  scanId: number;
  rootId: number;
}

async function scanMock(root: string): Promise<Mock> {
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

function open(mock: Mock, api: Api = mock.api, start: ExplorerStart = { focus: mock.rootId }): ExplorerStore {
  let store!: ExplorerStore;

  cleanups.push(
    $effect.root(() => {
      store = new ExplorerStore(mock.scanId, mock.rootId, start, api);
    }),
  );
  flushSync();

  return store;
}

async function settled(store: ExplorerStore, focus: NodeId = store.focus): Promise<void> {
  await tick();
  await vi.waitFor(() => {
    expect(store.frame.loading).toBe(false);
    expect(store.page.loading).toBe(false);
    expect(store.shown?.focus).toBe(focus);
  });
}

/** The chains on the map have been read as deep as they are going to be. */
async function expanded(store: ExplorerStore): Promise<void> {
  await settled(store);
  await new Promise((resolve) => setTimeout(resolve, 60));
  await tick();
}

const tile = (store: ExplorerStore, name: string) =>
  store.model?.children.find((child) => child.node.name === name);

/** Counts the `getLod` calls and can hold the ones for folders other than the shown one. */
function spy(base: Api) {
  const calls: { node: NodeId; maxDepth: number }[] = [];
  const held: (() => void)[] = [];
  let hold = false;

  const api: Api = {
    ...base,
    getLod: async (scan, node, maxNodes, maxDepth) => {
      calls.push({ node, maxDepth });

      if (hold && maxDepth === EXPANSION_DEPTH) {
        await new Promise<void>((release) => held.push(release));
      }

      return base.getLod(scan, node, maxNodes, maxDepth);
    },
  };

  return { api, calls, held, holdExtra: () => (hold = true) };
}

describe("chains on the map of the user's disk", () => {
  it('draws home / sam as one tile, and reads the folders inside it deeper, up to three times', async () => {
    const mock = await scanMock('/real');
    const { api, calls } = spy(mock.api);
    const store = open(mock, api);

    await expanded(store);

    const home = tile(store, 'home');

    expect(home?.chain?.map((link) => link.name)).toEqual(['home', 'sam']);
    // The first answer was the frame; the rest are chain expansions, never more than the cap.
    expect(calls.length).toBeGreaterThan(1);
    expect(calls.length - 1).toBeLessThanOrEqual(MAX_CHAIN_EXPANSIONS);
    expect(calls[1]?.maxDepth).toBe(EXPANSION_DEPTH);
    expect(calls.slice(1).every((call) => call.node !== mock.rootId)).toBe(true);

    // What is inside the last folder of the chain is there, with its own chains.
    const local = home?.children.find((child) => child.node.name === '.local');

    expect(local?.chain?.map((link) => link.name)).toEqual([
      '.local',
      'share',
      'Steam',
      'steamapps',
      'common',
    ]);
    expect(local?.children.length).toBeGreaterThan(5);

    const docker = home?.children.find((child) => child.node.name === '.docker');

    expect(docker?.chain?.map((link) => link.name)).toEqual(['.docker', 'desktop', 'vms', '0', 'data']);
  });

  it('reads nothing more for a folder that has no chain', async () => {
    const mock = await scanMock('/');
    const { api, calls } = spy(mock.api);
    const store = open(mock, api);

    await expanded(store);

    // `home / demo` is a chain, so there is a read for it; `var` and the others are not.
    expect(calls.length - 1).toBeLessThanOrEqual(MAX_CHAIN_EXPANSIONS);
    expect(tile(store, 'var')?.chain).toBeUndefined();
  });

  it('drops an answer that comes after the user has moved on', async () => {
    const mock = await scanMock('/real');
    const { api, held, holdExtra } = spy(mock.api);

    holdExtra();

    const store = open(mock, api);

    await settled(store);
    await vi.waitFor(() => expect(held).toHaveLength(1));

    // The read of the chains is in flight; move on to another folder, and let it answer afterwards.
    const variable = store.topLevel.find((node) => node.name === 'var') as NodeDto;

    store.open(variable.id);
    await settled(store, variable.id);
    held.splice(0).forEach((release) => release());
    await new Promise((resolve) => setTimeout(resolve, 60));
    await tick();

    // `var` is shown with its own tiles; nothing of `home` was spliced in.
    expect(store.model?.children.map((child) => child.node.name)).toEqual(['lib', 'cache', 'log']);
    expect(held).toHaveLength(0);
  });

  it('selects the chain by its first folder, and opens its last, with the whole path in the crumbs', async () => {
    const mock = await scanMock('/real');
    const store = open(mock);

    await expanded(store);

    const home = store.topLevel.find((node) => node.name === 'home') as NodeDto;

    store.choose(home.id);
    expect(store.selected).toBe(home.id);
    expect(store.focus).toBe(mock.rootId);
    expect(store.openTarget(home.id)?.name).toBe('sam');

    store.openSelected();

    const target = store.openTarget(home.id) as NodeDto;

    expect(store.focus).toBe(target.id);
    expect(store.selected).toBeNull();
    await settled(store, target.id);
    expect(store.crumbs.map((crumb) => crumb.name)).toEqual(['real', 'home', 'sam']);
    expect(store.crumbs.map((crumb) => crumb.path).at(-1)).toBe('/real/home/sam');
  });

  it("names the chain's last folder in the hint that more than 85% is in one place", async () => {
    const mock = await scanMock('/real');
    const store = open(mock);

    await expanded(store);

    expect(store.dominant).toMatchObject({ name: 'home/sam', percent: 97 });

    store.open(store.dominant?.node.id as NodeId);
    await settled(store);
    expect(store.crumbs.map((crumb) => crumb.name)).toEqual(['real', 'home', 'sam']);
  });

  it('goes back through the folders of the chain one at a time', async () => {
    const mock = await scanMock('/real');
    const store = open(mock);

    await expanded(store);
    store.open(store.topLevel.find((node) => node.name === 'home')?.id as NodeId);
    await settled(store);

    const sam = store.focus;

    store.up();
    await settled(store);
    expect(store.crumbs.map((crumb) => crumb.name)).toEqual(['real', 'home']);
    expect(store.focus).not.toBe(sam);
  });
});

describe('the colours of folders', () => {
  it("colour the scan root's children from the palette, and a chain tile with its first folder's colour", async () => {
    const mock = await scanMock('/real');
    const store = open(mock);

    await expanded(store);

    const [home, usr] = [tile(store, 'home'), tile(store, 'usr')];

    expect(store.focusColor).toBeNull();
    expect(store.colors.get(home?.node.id as NodeId)).toBe(PALETTE[0]);
    // `var` and `usr` are about the same size: they take the next two colours, whichever is first.
    expect([PALETTE[1], PALETTE[2]]).toContain(store.colors.get(usr?.node.id as NodeId));
    // The list's dot is the map's colour.
    expect(store.colorOf(home?.node.id as NodeId)).toBe(PALETTE[0]);
    expect(store.colorOf(COLLAPSED_NODE_ID)).not.toBe(PALETTE[0]);
  });

  it('shows the folders inside a dominant tile in the colours they have after the tile is opened', async () => {
    const mock = await scanMock('/real');
    const store = open(mock);

    await expanded(store);

    const home = tile(store, 'home');
    const inside = (home?.children ?? []).filter((child) => child.node.id !== COLLAPSED_NODE_ID);
    const before = new Map(inside.map((child) => [child.node.name, store.colors.get(child.node.id)]));

    expect(store.dominantTile).toBe(home?.node.id);
    // The biggest folder inside keeps the tile's colour; the next ones take the palette without it.
    expect(before.get('.local')).toBe(PALETTE[0]);
    expect(before.get('.docker')).toBe(PALETTE[1]);
    expect(before.get('Faugus')).toBe(PALETTE[2]);

    store.open(home?.node.id as NodeId);
    await expanded(store);

    expect(store.focusColor).toBe(PALETTE[0]);

    for (const name of ['.local', '.docker', 'Faugus', 'Projekty', '.cache']) {
      const node = store.topLevel.find((candidate) => candidate.name === name) as NodeDto;

      expect(store.colors.get(node.id), name).toBe(before.get(name));
    }
  });

  it("keeps a folder's colour when it is opened, for its largest child, and restores them on the way back", async () => {
    const mock = await scanMock('/real');
    const store = open(mock);

    await expanded(store);

    const rootColors = new Map(store.colors);
    const usr = store.topLevel.find((node) => node.name === 'usr') as NodeDto;

    store.open(usr.id);
    await settled(store);

    expect(store.focusColor).toBe(rootColors.get(usr.id));
    expect(store.colors.get(store.topLevel[0]?.id as NodeId)).toBe(rootColors.get(usr.id));
    expect(store.colors.get(store.topLevel[1]?.id as NodeId)).toBe(PALETTE[0]);

    store.up();
    await settled(store, mock.rootId);

    expect(store.focusColor).toBeNull();
    expect(store.colors.get(usr.id)).toBe(rootColors.get(usr.id));
  });

  it('finds the colour of a folder reached by a link, from the tree', async () => {
    const mock = await scanMock('/real');
    const viaRoot = open(mock);

    await expanded(viaRoot);

    const usr = viaRoot.topLevel.find((node) => node.name === 'usr') as NodeDto;
    const link = open(mock, mock.api, { focus: usr.id });

    await settled(link, usr.id);
    await vi.waitFor(() => expect(link.focusColor).toBe(viaRoot.colors.get(usr.id)));
  });

  it('takes the colour the caller knows, without asking', async () => {
    const mock = await scanMock('/real');
    const store = open(mock, mock.api, { focus: mock.rootId, color: null });

    await settled(store);
    expect(store.focusColor).toBeNull();
  });
});

/** A folder of `total` items on top of the demo scan, served in pages like the backend does. */
function bigFolder(mock: Mock, folder: NodeId, total: number) {
  const calls: { offset: number; sort: string }[] = [];
  const gate: { hold?: Promise<void> } = {};

  const item = (index: number): NodeDto => ({
    id: 1_000_000 + index,
    name: `file-${index}`,
    kind: 'file',
    allocated: 10_000 - index,
    apparent: 10_000 - index,
    files: 1,
    hasChildren: false,
    category: 'other',
    mtime: 1_700_000_000,
  });

  const api: Api = {
    ...mock.api,
    getChildren: async (scan, node, offset, limit, sort): Promise<ChildrenPage> => {
      if (node !== folder) {
        return mock.api.getChildren(scan, node, offset, limit, sort);
      }

      calls.push({ offset, sort });
      await gate.hold;

      const items = Array.from({ length: Math.max(0, Math.min(limit, total - offset)) }, (_, index) =>
        item(offset + index),
      );

      return { parentId: node, total, items };
    },
  };

  return { api, calls, gate };
}

describe('searching a folder', () => {
  async function inFolder(total: number) {
    const mock = await scanMock('/real');
    const probe = open(mock);

    await settled(probe);

    const folder = probe.topLevel.find((node) => node.name === 'opt') as NodeDto;
    const big = bigFolder(mock, folder.id, total);
    const store = open(mock, big.api, { focus: folder.id });

    await settled(store, folder.id);

    return { store, ...big };
  }

  it('loads the rest of the pages in the background while a query is on, and then stops', async () => {
    const { store, calls } = await inFolder(1000);

    expect(store.items).toHaveLength(PAGE_SIZE);
    expect(calls).toHaveLength(1);

    store.setQuery('file-9');
    await vi.waitFor(() => expect(store.items).toHaveLength(1000));

    expect(calls.map((call) => call.offset)).toEqual([0, 200, 400, 600, 800]);
    expect(store.searching).toBe(false);
    expect(store.searchTruncated).toBe(false);
  });

  it('loads nothing without a query', async () => {
    const { store, calls } = await inFolder(1000);

    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(calls).toHaveLength(1);
    expect(store.searching).toBe(false);
  });

  it('stops at the limit and says the folder has more', async () => {
    const { store, calls } = await inFolder(12_000);

    store.setQuery('x');
    await vi.waitFor(() => expect(store.items.length).toBe(SEARCH_LIMIT));
    await new Promise((resolve) => setTimeout(resolve, 40));

    expect(store.items).toHaveLength(SEARCH_LIMIT);
    expect(calls).toHaveLength(SEARCH_LIMIT / PAGE_SIZE);
    expect(store.searchTruncated).toBe(true);
    expect(store.searching).toBe(false);
  });

  it('asks for no more pages once the query is cleared', async () => {
    const { store, calls, gate } = await inFolder(5000);
    let release!: () => void;

    gate.hold = new Promise<void>((resolve) => (release = resolve));
    store.setQuery('a');
    await vi.waitFor(() => expect(calls).toHaveLength(2));

    store.setQuery('');
    release();
    await new Promise((resolve) => setTimeout(resolve, 50));

    // The page that was on its way arrived; nothing was asked for after it.
    expect(calls).toHaveLength(2);
    expect(store.searching).toBe(false);
  });

  it('ignores pages that arrive after the sort changed, and starts again for the new sort', async () => {
    const { store, calls, gate } = await inFolder(1000);
    let release!: () => void;

    gate.hold = new Promise<void>((resolve) => (release = resolve));
    store.setQuery('a');
    await vi.waitFor(() => expect(calls).toHaveLength(2));

    store.setSort('nameAsc');
    gate.hold = undefined;
    release();
    await settled(store);
    await vi.waitFor(() => expect(store.items).toHaveLength(1000));

    expect(calls.filter((call) => call.sort === 'nameAsc').length).toBeGreaterThan(1);
    expect(new Set(store.items.map((item) => item.id)).size).toBe(1000);
  });

  it('forgets the query when another folder is opened', async () => {
    const { store } = await inFolder(600);

    store.setQuery('file');
    store.open(store.rootId);
    expect(store.query).toBe('');
    await settled(store, store.rootId);
  });
});
