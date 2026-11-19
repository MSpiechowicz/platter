import { flushSync, tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { COLLAPSED_NODE_ID, isApiError, type Api } from '../../../api/client';
import { createMockApi } from '../../../api/mock';
import type { ChildrenPage, NodeDto, NodeId } from '../../../api/types';
import { PALETTE } from '../colors';
import { SEARCH_LIMIT } from '../search';
import {
  ExplorerStore,
  EXPANSION_DEPTH,
  PAGE_SIZE,
  QUERY_DELAY_MS,
  type ExplorerStart,
} from './explorer.svelte';
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
    // The biggest folder inside keeps the tile's colour. The next ones take the palette without it, and without the
    // colours of the tiles beside it, which are on the same map.
    const besides = store.topLevel
      .filter((node) => node.id !== home?.node.id)
      .map((node) => store.colors.get(node.id));
    expect(before.get('.local')).toBe(PALETTE[0]);
    for (const name of ['.docker', 'Faugus']) {
      expect(before.get(name), name).not.toBe(PALETTE[0]);
      expect(besides, name).not.toContain(before.get(name));
    }

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
    const home = store.topLevel.find((node) => node.name === 'home') as NodeDto;

    store.open(usr.id);
    await settled(store);

    expect(store.focusColor).toBe(rootColors.get(usr.id));
    expect(store.colors.get(store.topLevel[0]?.id as NodeId)).toBe(rootColors.get(usr.id));
    // The second in /usr skips the colours used above it: not cyan, the colour of /home beside /usr.
    expect(rootColors.get(home.id)).toBe(PALETTE[0]);
    expect(store.colors.get(store.topLevel[1]?.id as NodeId)).not.toBe(PALETTE[0]);

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

/**
 * A folder of `total` items on top of the demo scan, served in pages like the backend does: the items are named
 * `file-0` to `file-<total - 1>`, and a query keeps those whose name contains it, ignoring case.
 */
function bigFolder(mock: Mock, folder: NodeId, total: number) {
  const calls: { offset: number; sort: string; query?: string }[] = [];
  const gate: { hold?: Promise<void>; holdQuery?: { query: string; until: Promise<void> } } = {};

  const item = (index: number): NodeDto => ({
    id: 1_000_000 + index,
    name: `file-${index}`,
    kind: 'file',
    allocated: 100_000 - index,
    apparent: 100_000 - index,
    files: 1,
    hasChildren: false,
    category: 'other',
    mtime: 1_700_000_000,
  });

  const api: Api = {
    ...mock.api,
    getChildren: async (scan, node, offset, limit, sort, query): Promise<ChildrenPage> => {
      if (node !== folder) {
        return mock.api.getChildren(scan, node, offset, limit, sort, query);
      }

      calls.push({ offset, sort, query });
      await gate.hold;

      if (gate.holdQuery && gate.holdQuery.query === query) {
        await gate.holdQuery.until;
      }

      if (query?.includes('boom')) {
        throw new Error('the folder cannot be read');
      }

      const needle = query?.toLowerCase();
      const indices = Array.from({ length: total }, (_, index) => index).filter(
        (index) => !needle || item(index).name.includes(needle),
      );

      return {
        parentId: node,
        total: indices.length,
        items: indices.slice(offset, offset + limit).map(item),
      };
    },
  };

  return { api, calls, gate };
}

/** The store of `opt` of the `/real` scan with `total` items in it; `wrap` can change the API it is served by. */
async function inBigFolder(total: number, wrap: (api: Api) => Api = (api) => api) {
  const mock = await scanMock('/real');
  const probe = open(mock);

  await settled(probe);

  const folder = probe.topLevel.find((node) => node.name === 'opt') as NodeDto;
  const big = bigFolder(mock, folder.id, total);
  const store = open(mock, wrap(big.api), { focus: folder.id });

  await settled(store, folder.id);

  return { store, ...big };
}

describe('searching a folder', () => {
  const inFolder = (total: number) => inBigFolder(total);
  const asked = (calls: { query?: string }[]) => calls.filter((call) => call.query !== undefined);
  const names = (store: ExplorerStore) => store.items.map((item) => item.name);

  /** Types a query and lets the pause run out, as the list does after the last key. */
  async function search(store: ExplorerStore, query: string): Promise<void> {
    store.setQuery(query);
    await vi.advanceTimersByTimeAsync(QUERY_DELAY_MS);
    await tick();
  }

  async function answered(store: ExplorerStore): Promise<void> {
    await vi.waitFor(() => expect(store.page.loading).toBe(false));
  }

  afterEach(() => {
    vi.useRealTimers();
  });

  it('finds a match far beyond the old limit in one request for the first page', async () => {
    const { store, calls } = await inFolder(20_000);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'FILE-19999');
    await answered(store);

    expect(names(store)).toEqual(['file-19999']);
    expect(store.total).toBe(1);
    expect(store.hasMore).toBe(false);
    expect(calls.map((call) => [call.offset, call.query])).toEqual([
      [0, undefined],
      [0, 'FILE-19999'],
    ]);
    expect(store.matchedTotal).toBe(1);
    expect(store.allSizedTotal).toBe(20_000);
  });

  it('asks the backend once for what was typed in a burst, after the pause', async () => {
    const { store, calls } = await inFolder(1000);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    store.setQuery('f');
    await vi.advanceTimersByTimeAsync(QUERY_DELAY_MS - 50);
    store.setQuery('fi');
    await vi.advanceTimersByTimeAsync(QUERY_DELAY_MS - 50);
    store.setQuery('file-99');

    expect(asked(calls)).toEqual([]);
    expect(store.query).toBe('');

    await vi.advanceTimersByTimeAsync(QUERY_DELAY_MS);
    await tick();
    await answered(store);

    expect(store.query).toBe('file-99');
    expect(asked(calls).map((call) => call.query)).toEqual(['file-99']);
    expect(names(store)).toEqual(['file-99', ...Array.from({ length: 10 }, (_, index) => `file-99${index}`)]);
  });

  it('drops an answer that arrives after the query changed', async () => {
    const { store, gate } = await inFolder(20_000);
    let release!: () => void;

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    gate.holdQuery = { query: 'file-1', until: new Promise<void>((resolve) => (release = resolve)) };

    // The first query is slow to answer, the second is quick.
    await search(store, 'file-1');
    expect(store.page.loading).toBe(true);

    await search(store, 'file-12345');
    await answered(store);
    expect(names(store)).toEqual(['file-12345']);

    release();
    await vi.advanceTimersByTimeAsync(10);
    await tick();

    expect(store.query).toBe('file-12345');
    expect(names(store)).toEqual(['file-12345']);
    expect(store.total).toBe(1);
    expect(store.page.loading).toBe(false);
  });

  it('shows the loading state, with no rows, until the matches have arrived', async () => {
    const { store, gate } = await inFolder(1000);
    let release!: () => void;

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    gate.holdQuery = { query: 'file-5', until: new Promise<void>((resolve) => (release = resolve)) };
    await search(store, 'file-5');

    expect(store.page.loading).toBe(true);
    expect(store.page.data).toBeUndefined();
    expect(store.page.error).toBeUndefined();
    expect(store.items).toEqual([]);
    expect(store.pageCurrent).toBe(false);
    expect(store.matchedTotal).toBeUndefined();

    release();
    await answered(store);
    expect(store.page.loading).toBe(false);
    expect(store.items.length).toBeGreaterThan(0);
  });

  it('pages further on demand, asking for the same query', async () => {
    const { store, calls } = await inFolder(20_000);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'file-1');
    await answered(store);

    // 'file-1', 'file-10'..'file-19', 'file-100'.., 'file-1000'.., 'file-10000'..: 11,111 names.
    expect(store.total).toBe(11_111);
    expect(store.items).toHaveLength(PAGE_SIZE);
    expect(store.hasMore).toBe(true);
    expect(store.matchedTotal).toBe(11_111);

    await store.loadMore();

    expect(store.items).toHaveLength(2 * PAGE_SIZE);
    expect(new Set(store.items.map((item) => item.id)).size).toBe(2 * PAGE_SIZE);
    expect(calls.at(-1)).toMatchObject({ offset: PAGE_SIZE, query: 'file-1' });
    expect(asked(calls)).toHaveLength(2);
  });

  it('shows the whole folder again when the query is cleared, without asking for it again', async () => {
    const { store, calls } = await inFolder(20_000);

    await store.loadMore();
    expect(store.items).toHaveLength(2 * PAGE_SIZE);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'file-777');
    await answered(store);
    expect(store.total).toBeLessThan(200);

    const before = calls.length;

    store.setQuery('');

    // At once: no pause, no request, and the rows that were loaded are back.
    expect(store.query).toBe('');
    expect(store.page.loading).toBe(false);
    expect(store.items).toHaveLength(2 * PAGE_SIZE);
    expect(store.total).toBe(20_000);
    expect(store.matchedTotal).toBeUndefined();
    await vi.advanceTimersByTimeAsync(QUERY_DELAY_MS * 2);
    expect(calls).toHaveLength(before);
  });

  it('cancels a pause that has not run out when the query is cleared', async () => {
    const { store, calls } = await inFolder(1000);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    store.setQuery('file-5');
    store.setQuery('');
    await vi.advanceTimersByTimeAsync(QUERY_DELAY_MS * 2);

    expect(store.query).toBe('');
    expect(asked(calls)).toEqual([]);
  });

  it('takes a query of only spaces for no query', async () => {
    const { store, calls } = await inFolder(1000);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, '   ');

    expect(store.query).toBe('');
    expect(asked(calls)).toEqual([]);
  });

  it('shows the error when the matches cannot be read, never "no matches"', async () => {
    const { store, calls } = await inFolder(1000);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'boom');
    await answered(store);

    expect(store.page.error).toEqual(new Error('the folder cannot be read'));
    expect(store.page.loading).toBe(false);
    expect(store.items).toEqual([]);

    // Another query is a new attempt: the error goes while it is on its way, and the list is back after it.
    await search(store, 'file-5');
    await answered(store);
    expect(store.page.error).toBeUndefined();
    expect(store.items.length).toBeGreaterThan(0);

    // "Try again" asks for the matches again.
    await search(store, 'boom2');
    await answered(store);
    expect(store.page.error).toBeDefined();

    const before = asked(calls).length;

    store.retry();
    await tick();
    expect(store.page.error).toBeUndefined();
    expect(store.page.loading).toBe(true);
    await answered(store);
    expect(asked(calls)).toHaveLength(before + 1);
    expect(store.page.error).toBeDefined();
  });

  it('refuses a query the backend refuses, with its own error', async () => {
    const mock = await scanMock('/real');
    const store = open(mock);

    await settled(store);
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'a'.repeat(1025));
    await answered(store);

    expect(isApiError(store.page.error)).toBe(true);
    expect(store.page.error).toMatchObject({ code: 'invalid' });
    expect(store.items).toEqual([]);
  });

  it('keeps the chosen row selected while a search hides it, and after it is cleared', async () => {
    const { store } = await inFolder(20_000);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'file-19999');
    await answered(store);

    const row = store.items[0] as NodeDto;

    store.choose(row.id);
    expect(store.selectedNode?.name).toBe('file-19999');

    await search(store, 'file-5');
    await answered(store);
    expect(store.items.some((item) => item.id === row.id)).toBe(false);
    expect(store.selectedNode?.name).toBe('file-19999');

    store.setQuery('');
    expect(store.selectedNode?.name).toBe('file-19999');
  });

  it('counts the sized items of the folder and of the matches, and does not count the rest as matches', async () => {
    const mock = await scanMock('/real');
    const store = open(mock);

    await settled(store);

    const all = store.sizedTotal;

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'zzz-no-such-name');
    await answered(store);

    expect(store.allSizedTotal).toBe(all);
    expect(store.matchedTotal).toBe(0);
    expect(store.total).toBe(0);
    expect(store.items).toEqual([]);
    expect(store.page.error).toBeUndefined();
  });

  it('asks again for the new sort while a query is on', async () => {
    const { store, calls } = await inFolder(1000);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'file-9');
    await answered(store);
    store.setSort('nameAsc');
    await tick();
    await answered(store);

    expect(calls.filter((call) => call.query === 'file-9').map((call) => call.sort)).toEqual([
      'sizeDesc',
      'nameAsc',
    ]);
    expect(store.items.length).toBeGreaterThan(0);
  });

  it('does not load the rest of a big folder by itself', async () => {
    const { store, calls } = await inFolder(12_000);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'file');
    await answered(store);
    await vi.advanceTimersByTimeAsync(200);

    expect(store.total).toBe(12_000);
    expect(store.items).toHaveLength(PAGE_SIZE);
    expect(calls).toHaveLength(2);
  });

  it('ignores a page of more rows that arrives after the query changed', async () => {
    const { store, calls, gate } = await inFolder(5000);
    let release!: () => void;

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'file-1');
    await answered(store);

    gate.hold = new Promise<void>((resolve) => (release = resolve));

    const more = store.loadMore();

    await vi.advanceTimersByTimeAsync(1);
    expect(store.loadingMore).toBe(true);

    gate.hold = undefined;
    await search(store, 'file-4');
    await answered(store);
    release();
    await more;

    expect(store.loadingMore).toBe(false);
    expect(store.items.every((item) => item.name.includes('file-4'))).toBe(true);
    expect(calls.filter((call) => call.offset > 0)).toHaveLength(1);
  });

  it('forgets the query, and its pause, when another folder is opened', async () => {
    const { store, calls } = await inFolder(600);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    store.setQuery('file');
    store.open(store.rootId);
    expect(store.query).toBe('');
    await vi.advanceTimersByTimeAsync(QUERY_DELAY_MS * 2);
    expect(store.query).toBe('');
    expect(asked(calls)).toEqual([]);

    vi.useRealTimers();
    await settled(store, store.rootId);
  });

  it('loads nothing more without a query, and a selected tile does not load matches in the background', async () => {
    const { store, calls } = await inFolder(5000);

    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(calls).toHaveLength(1);

    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    await search(store, 'file');
    await answered(store);

    store.chooseGroup(
      {
        ...(store.items[0] as NodeDto),
        id: COLLAPSED_NODE_ID,
        name: '4,000 smaller items',
        kind: 'collapsed',
      },
      [],
    );
    expect(store.groupNeedsRows).toBe(false);
    expect(store.groupTruncated).toBe(false);
  });
});

describe('the rows of a selected "smaller items" tile', () => {
  /** The "N smaller items" tile of the folder, as the map would hand it over. */
  const smaller = (store: ExplorerStore, count: number): NodeDto => ({
    ...(store.items[0] as NodeDto),
    id: COLLAPSED_NODE_ID,
    name: `${count} smaller items`,
    kind: 'collapsed',
    hasChildren: false,
  });

  /** What the list does while the tile asks for rows: one page after another, for as long as it is told to. */
  const loadWhileWanted = async (store: ExplorerStore): Promise<void> => {
    // A list that is never told to stop would loop for ever, so the test has a limit of its own.
    for (let round = 0; round < 100 && store.groupNeedsRows && !store.loadingMore; round++) {
      await store.loadMore();
    }
  };

  it('are loaded in the background only up to the limit, and the list says it shows the first ones', async () => {
    const { store, calls } = await inBigFolder(12_000);

    store.chooseGroup(smaller(store, 11_000), []);
    expect(store.groupNeedsRows).toBe(true);
    expect(store.groupTruncated).toBe(false);

    await loadWhileWanted(store);

    expect(store.items).toHaveLength(SEARCH_LIMIT);
    expect(calls).toHaveLength(SEARCH_LIMIT / PAGE_SIZE);
    expect(store.groupNeedsRows).toBe(false);
    expect(store.groupTruncated).toBe(true);

    // The user can still ask for more by hand.
    await store.loadMore();
    expect(store.items).toHaveLength(SEARCH_LIMIT + PAGE_SIZE);
  });

  it('are loaded to the end when the tile is small enough, and nothing is said', async () => {
    const { store, calls } = await inBigFolder(1000);

    store.chooseGroup(smaller(store, 900), []);
    await loadWhileWanted(store);

    expect(calls.length).toBeGreaterThan(1);
    expect(store.groupNeedsRows).toBe(false);
    expect(store.groupTruncated).toBe(false);
    expect(store.highlighted.size).toBeGreaterThanOrEqual(900);
  });

  it('stop being asked for after a request failed', async () => {
    let failed = 0;

    const { store } = await inBigFolder(5000, (api) => ({
      ...api,
      getChildren: (scan, node, offset, limit, sort) => {
        if (offset === 0) {
          return api.getChildren(scan, node, offset, limit, sort);
        }

        failed++;

        return Promise.reject(new Error('disk gone'));
      },
    }));

    store.chooseGroup(smaller(store, 4000), []);
    await loadWhileWanted(store);

    expect(store.moreError).not.toBeNull();
    expect(store.groupNeedsRows).toBe(false);
    expect(failed).toBe(1);
  });
});

describe('the details of a chain tile', () => {
  it('describe its last folder: the same one "Go into folder" opens and "Show in file manager" shows', async () => {
    const mock = await scanMock('/real');
    const openInFileManager = vi.fn().mockResolvedValue(undefined);
    const store = open(mock, { ...mock.api, openInFileManager });

    await expanded(store);

    const home = tile(store, 'home') as NonNullable<ReturnType<typeof tile>>;
    const sam = home.chain?.at(-1) as NodeDto;

    expect(sam.name).toBe('sam');

    // Whether the tile is chosen in the map or its row in the list, it is the same selection.
    store.choose(home.node.id);

    expect(store.selectedNode?.name).toBe('home');
    expect(store.selectedTarget).toBe(sam);
    expect(store.selectedTarget).toBe(store.openTarget(home.node.id));

    await store.reveal();
    expect(openInFileManager).toHaveBeenCalledWith(mock.scanId, sam.id);

    store.openSelected();
    expect(store.focus).toBe(sam.id);
  });

  it('are the folder itself for a tile that is not a chain, and for the open folder when nothing is selected', async () => {
    const mock = await scanMock('/real');
    const openInFileManager = vi.fn().mockResolvedValue(undefined);
    const store = open(mock, { ...mock.api, openInFileManager });

    await expanded(store);
    expect(store.selectedTarget).toBeUndefined();

    await store.reveal();
    expect(openInFileManager).toHaveBeenLastCalledWith(mock.scanId, mock.rootId);

    const variable = store.topLevel.find((node) => node.name === 'var') as NodeDto;

    store.choose(variable.id);
    expect(store.selectedTarget).toBe(store.selectedNode);

    await store.reveal();
    expect(openInFileManager).toHaveBeenLastCalledWith(mock.scanId, variable.id);
  });
});
