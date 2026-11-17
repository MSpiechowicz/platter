import { describe, expect, it, vi } from 'vitest';
import { COLLAPSED_NODE_ID, isApiError, type Api } from './client';
import { createMockApi, MOCK_VOLUMES } from './mock';
import type { LodNode, NodeDto, ScanEvent, ScanId, SortKey, Totals } from './types';

const NOW = 1_750_000_000;
const FAST = { durationMs: 30, tickMs: 5, now: () => NOW };

interface FinishedScan {
  id: ScanId;
  rootId: number;
  totals: Totals;
  events: ScanEvent[];
}

/** Starts a scan and waits for its last event. */
function scan(api: Api, root = '/', oneFileSystem = true): Promise<FinishedScan> {
  return new Promise((resolve, reject) => {
    const events: ScanEvent[] = [];

    api
      .startScan({ root, oneFileSystem }, (event) => {
        events.push(event);

        if (event.type === 'finished') {
          resolve({ id: event.scanId, rootId: event.rootId, totals: event.totals, events });
        } else if (event.type === 'failed' || event.type === 'cancelled') {
          reject(new Error(`scan ended with ${event.type}`));
        }
      })
      .catch(reject);
  });
}

async function allChildren(
  api: Api,
  scanId: ScanId,
  node: number,
  sort: SortKey = 'sizeDesc',
): Promise<NodeDto[]> {
  const page = await api.getChildren(scanId, node, 0, 1000, sort);

  return page.items;
}

function count(lod: LodNode): number {
  return 1 + lod.children.reduce((sum, child) => sum + count(child), 0);
}

function depth(lod: LodNode): number {
  return lod.children.length === 0 ? 0 : 1 + Math.max(...lod.children.map(depth));
}

describe('volumes and folder picker', () => {
  it('lists the demo volumes and picks a sample folder', async () => {
    const api = createMockApi(FAST);

    expect(await api.listVolumes()).toEqual(MOCK_VOLUMES);
    expect(await api.pickFolder()).toBe('/home/demo');
  });

  it('can simulate a cancelled picker', async () => {
    expect(await createMockApi({ pickedFolder: null }).pickFolder()).toBeNull();
  });
});

describe('folders left out of scans', () => {
  /** Starts a scan with these folders left out and waits for its end. */
  function scanWithout(api: Api, excluded: string[]): Promise<FinishedScan> {
    return new Promise((resolve, reject) => {
      api
        .startScan({ root: '/', oneFileSystem: true, excluded }, (event) => {
          if (event.type === 'finished') {
            resolve({ id: event.scanId, rootId: event.rootId, totals: event.totals, events: [] });
          }
        })
        .catch(reject);
    });
  }

  it('picks a folder to leave out, or nothing when cancelled', async () => {
    expect(await createMockApi(FAST).pickExcludedFolder()).toBe('/home/demo/Videos');
    expect(await createMockApi({ pickedExcluded: null }).pickExcludedFolder()).toBeNull();
  });

  it('shows a left-out folder as skipped, counts nothing of it, and never leaves out the root', async () => {
    const api = createMockApi(FAST);
    const full = await scan(api);
    const home = (await allChildren(api, full.id, full.rootId)).find((node) => node.name === 'home');
    expect(home?.kind).toBe('dir');

    const without = await scanWithout(api, ['/home', '/']);
    const left = (await allChildren(api, without.id, without.rootId)).find((node) => node.name === 'home');

    expect(left).toMatchObject({ kind: 'mount', skipped: 'excluded', allocated: 0 });
    expect(without.totals.allocated).toBeLessThan(full.totals.allocated);
  });

  it('refuses what the backend refuses: a relative path, too many folders', async () => {
    const api = createMockApi(FAST);

    await expect(
      api.startScan({ root: '/', oneFileSystem: true, excluded: ['~/x'] }, () => {}),
    ).rejects.toMatchObject({
      code: 'invalid',
    });
    const many = Array.from({ length: 65 }, (_, index) => `/x${index}`);
    await expect(
      api.startScan({ root: '/', oneFileSystem: true, excluded: many }, () => {}),
    ).rejects.toMatchObject({
      code: 'invalid',
    });
  });
});

describe('the root node', () => {
  it('is named like the real backend: the last path component, or "/" for the filesystem root', async () => {
    const api = createMockApi(FAST);

    for (const [root, name] of [
      ['/', '/'],
      ['/home/demo', 'demo'],
      ['/mnt/backup', 'backup'],
    ] as const) {
      const { id, rootId } = await scan(api, root);
      const path = await api.getPath(id, rootId);

      expect((await api.getNode(id, rootId)).name).toBe(name);
      expect(path.path).toBe(root);
      expect(path.segments).toEqual([{ id: rootId, name }]);
    }
  });
});

describe('scanning', () => {
  it('reports growing progress and then finishes with totals that match the root node', async () => {
    const api = createMockApi(FAST);
    const { id, rootId, totals, events } = await scan(api);

    const progress = events.filter((event) => event.type === 'progress');
    expect(progress.length).toBeGreaterThan(1);

    const files = progress.map((event) => event.files);
    expect(files).toEqual([...files].sort((a, b) => a - b));
    expect(progress.at(-1)?.currentPath).toMatch(/^\//);

    const root = await api.getNode(id, rootId);
    expect(totals.allocated).toBe(root.allocated);
    expect(totals.files).toBe(root.files);
    expect(totals.dirs).toBeGreaterThan(50);
  });

  it('builds a tree of a few thousand nodes that adds up to the volume usage', async () => {
    const api = createMockApi(FAST);
    const { totals } = await scan(api);
    const [volume] = MOCK_VOLUMES;

    expect(totals.files + totals.dirs).toBeGreaterThan(2000);
    expect(totals.files + totals.dirs).toBeLessThan(10_000);

    const used = (volume?.total ?? 0) - (volume?.available ?? 0);
    expect(Math.abs(totals.allocated - used) / used).toBeLessThan(0.01);
  });

  it('is deterministic for a seed', async () => {
    const ages = async (seed: number): Promise<unknown> => {
      const api = createMockApi({ ...FAST, seed });
      const { id, rootId } = await scan(api);

      return (await api.getSummary(id, rootId)).byAge;
    };

    expect(await ages(7)).toEqual(await ages(7));
    expect(await ages(7)).not.toEqual(await ages(8));
  });

  it('finishes at once when the duration is zero', async () => {
    const { events } = await scan(createMockApi({ ...FAST, durationMs: 0 }));

    expect(events.map((event) => event.type)).toEqual(['finished']);
  });

  it('keeps a mount point empty only when the scan stays on one filesystem', async () => {
    const api = createMockApi(FAST);
    const stay = await scan(api, '/', true);
    const cross = await scan(api, '/', false);

    const mount = (await allChildren(api, stay.id, stay.rootId)).find((node) => node.name === 'mnt');
    const crossed = (await allChildren(api, cross.id, cross.rootId)).find((node) => node.name === 'mnt');

    expect(mount).toMatchObject({ kind: 'mount', allocated: 0, hasChildren: false });
    expect(crossed?.kind).toBe('dir');
    expect(crossed?.allocated).toBeGreaterThan(0);
  });

  it('says why a mount was not entered, only on mounts, and always whether a folder could be read', async () => {
    const api = createMockApi(FAST);
    const stay = await scan(api, '/real', true);
    const kids = await allChildren(api, stay.id, stay.rootId);
    const mounts = Object.fromEntries(
      kids.filter((kid) => kid.kind === 'mount').map((kid) => [kid.name, kid.skipped]),
    );

    // The backend never enters a virtual filesystem and enters another disk only when the scan leaves it.
    expect(mounts).toEqual({
      boot: 'otherDisk',
      dev: 'system',
      proc: 'system',
      run: 'system',
      sys: 'system',
      tmp: 'system',
      mnt: 'otherDisk',
    });
    expect(kids.filter((kid) => kid.kind !== 'mount').every((kid) => !('skipped' in kid))).toBe(true);
    expect(kids.every((kid) => typeof kid.unreadable === 'boolean')).toBe(true);
    expect(kids.find((kid) => kid.name === 'root')?.unreadable).toBe(true);
    expect(kids.find((kid) => kid.name === 'home')?.unreadable).toBe(false);

    const cross = await scan(api, '/real', false);
    const crossed = await allChildren(api, cross.id, cross.rootId);

    expect(crossed.filter((kid) => kid.kind === 'mount').map((kid) => kid.name)).toEqual([
      'dev',
      'proc',
      'run',
      'sys',
      'tmp',
    ]);
    expect(crossed.find((kid) => kid.name === 'mnt')?.kind).toBe('dir');
    expect(await api.getNode(cross.id, crossed.find((kid) => kid.name === 'proc')?.id ?? 0)).toMatchObject({
      kind: 'mount',
      skipped: 'system',
      unreadable: false,
    });
  });

  it('emits cancelled, nothing after it, and frees the scan', async () => {
    const api = createMockApi({ durationMs: 500, tickMs: 5, now: () => NOW });
    const events: ScanEvent[] = [];
    const id = await api.startScan({ root: '/', oneFileSystem: true }, (event) => events.push(event));

    await new Promise((resolve) => setTimeout(resolve, 30));
    await api.cancelScan(id);
    await new Promise((resolve) => setTimeout(resolve, 60));

    expect(events.at(-1)).toEqual({ type: 'cancelled' });
    expect(events.filter((event) => event.type === 'cancelled')).toHaveLength(1);
    expect(events.some((event) => event.type === 'finished')).toBe(false);
    await expect(api.getNode(id, 0)).rejects.toMatchObject({ code: 'notFound' });
  });

  it('can hold a scan half way for screenshots', async () => {
    const api = createMockApi({ ...FAST, durationMs: 100, holdAt: 0.5 });
    const events: ScanEvent[] = [];

    await api.startScan({ root: '/', oneFileSystem: true }, (event) => events.push(event));
    await new Promise((resolve) => setTimeout(resolve, 200));

    expect(events.every((event) => event.type === 'progress')).toBe(true);
    expect(events.length).toBeGreaterThan(0);
  });

  it('forgets a scan that was dropped', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api);

    await api.dropScan(id);

    await expect(api.getSummary(id, 0)).rejects.toSatisfy(isApiError);
  });

  it('cancels and drops an unknown, finished or already dropped scan without an error', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api);

    await expect(api.cancelScan(id)).resolves.toBeUndefined();
    await expect(api.dropScan(id)).resolves.toBeUndefined();
    await expect(api.dropScan(id)).resolves.toBeUndefined();
    await expect(api.cancelScan(id)).resolves.toBeUndefined();
    await expect(api.cancelScan(99_999)).resolves.toBeUndefined();
    await expect(api.dropScan(99_999)).resolves.toBeUndefined();
  });
});

describe('getChildren', () => {
  it('pages through all children without gaps or repeats', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const everything = await allChildren(api, id, rootId);

    const seen: number[] = [];
    let total = 0;

    for (let offset = 0; offset < everything.length; offset += 3) {
      const page = await api.getChildren(id, rootId, offset, 3, 'sizeDesc');

      total = page.total;
      expect(page.parentId).toBe(rootId);
      expect(page.items.length).toBeLessThanOrEqual(3);
      seen.push(...page.items.map((item) => item.id));
    }

    expect(total).toBe(everything.length);
    expect(seen).toEqual(everything.map((item) => item.id));
  });

  it('returns an empty page past the end and reports the total', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const page = await api.getChildren(id, rootId, 1000, 10, 'sizeDesc');

    expect(page.items).toEqual([]);
    expect(page.total).toBeGreaterThan(0);
  });

  it('sorts by every key', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const [home] = await allChildren(api, id, rootId);
    const target = home?.id ?? rootId;

    const sizeDesc = await allChildren(api, id, target, 'sizeDesc');
    expect(sizeDesc.map((n) => n.allocated)).toEqual(sizeDesc.map((n) => n.allocated).sort((a, b) => b - a));

    const sizeAsc = await allChildren(api, id, target, 'sizeAsc');
    expect(sizeAsc.map((n) => n.allocated)).toEqual(sizeAsc.map((n) => n.allocated).sort((a, b) => a - b));

    const byFiles = await allChildren(api, id, target, 'filesDesc');
    expect(byFiles.map((n) => n.files)).toEqual(byFiles.map((n) => n.files).sort((a, b) => b - a));

    const byTime = await allChildren(api, id, target, 'mtimeDesc');
    expect(byTime.map((n) => n.mtime)).toEqual(byTime.map((n) => n.mtime).sort((a, b) => b - a));

    expect(new Set(sizeAsc.map((n) => n.id))).toEqual(new Set(sizeDesc.map((n) => n.id)));
  });

  it('sorts names in ASCII-lowercased byte order, so "file10" comes before "file2"', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api, '/home/demo');
    const lower = (name: string): string => name.replace(/[A-Z]/g, (letter) => letter.toLowerCase());

    // Every folder: the order is plain byte order of the lowercased names, never a "natural" number order.
    const folders = [rootId];

    for (let folder = folders.pop(); folder !== undefined; folder = folders.pop()) {
      const kids = await allChildren(api, id, folder, 'nameAsc');
      const names = kids.map((kid) => lower(kid.name));

      expect(names).toEqual([...names].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)));
      folders.push(...kids.filter((kid) => kid.kind === 'dir').map((kid) => kid.id));
    }
  });

  describe('with a name filter', () => {
    it('lists only the children whose name contains it, ignoring case, and counts them as the total', async () => {
      const api = createMockApi(FAST);
      const { id, rootId } = await scan(api, '/home/demo');
      const everything = await allChildren(api, id, rootId);
      const word = (everything.find((child) => /[a-z]{3}/i.test(child.name))?.name ?? '').slice(0, 3);
      const expected = everything.filter((child) => child.name.toLowerCase().includes(word.toLowerCase()));

      expect(expected.length).toBeGreaterThan(0);

      for (const query of [word.toLowerCase(), word.toUpperCase()]) {
        const page = await api.getChildren(id, rootId, 0, 1000, 'sizeDesc', query);

        expect(page.total).toBe(expected.length);
        expect(page.items.map((item) => item.id)).toEqual(expected.map((item) => item.id));
      }
    });

    it('pages through the matches in the requested order, with the total of the matches on every page', async () => {
      const api = createMockApi(FAST);
      const { id, rootId } = await scan(api, '/home/demo');
      const everything = await allChildren(api, id, rootId, 'nameAsc');
      const query = 'e';
      const expected = everything.filter((child) => child.name.toLowerCase().includes(query));
      const seen: number[] = [];

      expect(expected.length).toBeGreaterThan(2);
      expect(expected.length).toBeLessThan(everything.length);

      for (let offset = 0; offset < expected.length; offset += 2) {
        const page = await api.getChildren(id, rootId, offset, 2, 'nameAsc', query);

        expect(page.total).toBe(expected.length);
        seen.push(...page.items.map((item) => item.id));
      }

      expect(seen).toEqual(expected.map((item) => item.id));
      expect((await api.getChildren(id, rootId, 1000, 2, 'nameAsc', query)).items).toEqual([]);
    });

    it('has no matches for a name that is not there', async () => {
      const api = createMockApi(FAST);
      const { id, rootId } = await scan(api, '/home/demo');

      expect(await api.getChildren(id, rootId, 0, 10, 'sizeDesc', 'no such name \u2603')).toMatchObject({
        total: 0,
        items: [],
      });
    });

    it('filters nothing for a blank query', async () => {
      const api = createMockApi(FAST);
      const { id, rootId } = await scan(api, '/home/demo');
      const all = await api.getChildren(id, rootId, 0, 1000, 'sizeDesc');

      for (const query of ['', '   ']) {
        expect(await api.getChildren(id, rootId, 0, 1000, 'sizeDesc', query)).toEqual(all);
      }
    });

    it('refuses a query longer than 1024 bytes, counting bytes and not characters', async () => {
      const api = createMockApi(FAST);
      const { id, rootId } = await scan(api, '/home/demo');

      // 512 two-byte letters are exactly 1024 bytes; one more is over.
      await expect(api.getChildren(id, rootId, 0, 10, 'sizeDesc', 'a'.repeat(1025))).rejects.toMatchObject({
        code: 'invalid',
      });
      await expect(
        api.getChildren(id, rootId, 0, 10, 'sizeDesc', '\u00e9'.repeat(513)),
      ).rejects.toMatchObject({ code: 'invalid' });
      expect((await api.getChildren(id, rootId, 0, 10, 'sizeDesc', '\u00e9'.repeat(512))).total).toBe(0);
      expect((await api.getChildren(id, rootId, 0, 10, 'sizeDesc', 'a'.repeat(1024))).total).toBe(0);
    });
  });

  it('rejects unknown nodes and negative paging', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);

    await expect(api.getNode(id, 99_999_999)).rejects.toMatchObject({ code: 'notFound' });
    await expect(api.getChildren(id, rootId, -1, 10, 'sizeDesc')).rejects.toMatchObject({ code: 'invalid' });
    await expect(api.getChildren(id + 1, rootId, 0, 10, 'sizeDesc')).rejects.toMatchObject({
      code: 'notFound',
    });
  });

  it('gives directories the category that holds most of their bytes, and the newest mtime inside', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const [top] = await allChildren(api, id, rootId);
    const kids = await allChildren(api, id, top?.id ?? rootId);
    const summary = await api.getSummary(id, top?.id ?? rootId);

    expect(summary.byCategory[0]?.category).toBe(top?.category);
    expect(Math.max(...kids.map((kid) => kid.mtime))).toBe(top?.mtime);
  });
});

describe('getLod', () => {
  it('never returns more than maxNodes nodes or maxDepth levels', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);

    for (const [maxNodes, maxDepth] of [
      [1, 3],
      [7, 3],
      [60, 2],
      [400, 4],
      [100_000, 2],
    ] as const) {
      const lod = await api.getLod(id, rootId, maxNodes, maxDepth);

      expect(count(lod)).toBeLessThanOrEqual(maxNodes);
      expect(depth(lod)).toBeLessThanOrEqual(maxDepth);
      expect(lod.node.id).toBe(rootId);
    }
  });

  it('merges children that do not fit into one "N smaller items" node', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const all = await allChildren(api, id, rootId);

    const lod = await api.getLod(id, rootId, 4, 1);
    const collapsed = lod.children.find((child) => child.node.kind === 'collapsed');
    const shown = lod.children.filter((child) => child.node.kind !== 'collapsed');

    expect(lod.children).toHaveLength(4 - 1);
    expect(collapsed?.node.id).toBe(COLLAPSED_NODE_ID);
    expect(collapsed?.node.name).toBe(`${all.length - shown.length} smaller items`);
    expect(collapsed?.children).toEqual([]);

    // The biggest children stay; the merged node carries the rest of the bytes.
    expect(shown.map((child) => child.node.id)).toEqual(all.slice(0, shown.length).map((n) => n.id));
    const rest = all.slice(shown.length).reduce((sum, node) => sum + node.allocated, 0);
    expect(collapsed?.node.allocated).toBe(rest);

    // It takes the category that holds most of the merged bytes, not a fixed one.
    const bytes = new Map<string, number>();

    for (const node of all.slice(shown.length)) {
      bytes.set(node.category, (bytes.get(node.category) ?? 0) + node.allocated);
    }

    const heaviest = [...bytes].sort((x, y) => y[1] - x[1])[0]?.[0];

    expect(collapsed?.node.category).toBe(heaviest);
    expect(collapsed?.node.unreadable).toBe(false);
  });

  it('does not merge when everything fits, and expands the largest directories first', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const all = await allChildren(api, id, rootId);

    // Children come largest first; equal sizes go by id (the children list goes by name), like the backend's LOD.
    const lod = await api.getLod(id, rootId, all.length + 2, 1);
    expect(lod.children.map((child) => child.node.id).sort((a, b) => a - b)).toEqual(
      all.map((n) => n.id).sort((a, b) => a - b),
    );
    expect(lod.children.map((child) => child.node.allocated)).toEqual(all.map((n) => n.allocated));

    // With room for the root's children and a handful more, the biggest directory is the one that opens.
    const deeper = await api.getLod(id, rootId, all.length + 8, 2);
    const opened = deeper.children.filter((child) => child.children.length > 0);
    expect(opened.length).toBeGreaterThan(0);
    expect(opened[0]?.node.id).toBe(all[0]?.id);
  });

  it('names one merged child "1 smaller item", also when it could have been shown', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const all = await allChildren(api, id, rootId);

    // Like the backend: the loop stops when the budget is used up, before the last sibling takes the placeholder's place.
    const lod = await api.getLod(id, rootId, all.length + 1, 1);
    const collapsed = lod.children.at(-1)?.node;

    expect(lod.children).toHaveLength(all.length);
    expect(collapsed).toMatchObject({ id: COLLAPSED_NODE_ID, kind: 'collapsed', name: '1 smaller item' });
    expect(collapsed?.allocated).toBe(all.at(-1)?.allocated);
  });

  it('spends the budget on the largest nodes anywhere, so a directory shows only some of its children', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const lod = await api.getLod(id, rootId, 40, 4);
    const partial: LodNode[] = [];
    const visit = (node: LodNode): void => {
      if (node !== lod && node.children.some((child) => child.node.kind === 'collapsed')) {
        partial.push(node);
      }

      node.children.forEach(visit);
    };

    visit(lod);

    expect(count(lod)).toBeLessThanOrEqual(40);
    expect(partial.length).toBeGreaterThan(1);

    for (const node of partial) {
      const shown = node.children.filter((child) => child.node.kind !== 'collapsed');
      const hidden = Number(/^(\d+) smaller items?$/.exec(node.children.at(-1)?.node.name ?? '')?.[1]);
      const total = (await api.getChildren(id, node.node.id, 0, 1, 'sizeDesc')).total;

      expect(shown.length).toBeGreaterThan(0);
      expect(shown.length + hidden).toBe(total);
    }
  });

  it('rejects impossible limits', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);

    await expect(api.getLod(id, rootId, -1, 2)).rejects.toMatchObject({ code: 'invalid' });
    await expect(api.getLod(id, rootId, 10, -1)).rejects.toMatchObject({ code: 'invalid' });

    // The backend takes unsigned numbers and still returns the root when it is given no room.
    expect((await api.getLod(id, rootId, 0, 2)).children).toEqual([]);
  });
});

describe('getSummary', () => {
  it('adds categories and age buckets up to the node total', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);

    for (const node of [rootId, (await allChildren(api, id, rootId))[1]?.id ?? rootId]) {
      const summary = await api.getSummary(id, node);
      const sum = (rows: { allocated: number }[]): number =>
        rows.reduce((total, row) => total + row.allocated, 0);

      expect(sum(summary.byCategory)).toBe(summary.node.allocated);
      expect(sum(summary.byAge)).toBe(summary.node.allocated);
      expect(summary.byCategory.reduce((total, row) => total + row.files, 0)).toBe(summary.node.files);
      expect(summary.byAge.reduce((total, row) => total + row.files, 0)).toBe(summary.node.files);
    }
  });

  it('always lists the six age buckets in order and sorts categories by size', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const summary = await api.getSummary(id, rootId);

    expect(summary.byAge.map((row) => row.bucket)).toEqual([
      'week',
      'month',
      'quarter',
      'year',
      'twoYears',
      'older',
    ]);

    const sizes = summary.byCategory.map((row) => row.allocated);
    expect(sizes).toEqual([...sizes].sort((a, b) => b - a));
  });

  it('lists the largest directories like the backend: a dominated folder is replaced by its children, ties go by id', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const { largestDirs } = await api.getSummary(id, rootId);

    // /home holds all of its bytes in /home/demo and .snapshots 86% in its old half, so those two drop out.
    expect(largestDirs.map((dir) => dir.name)).toEqual([
      'demo',
      'older than 30 days',
      'var',
      'recent',
      'usr',
      'swap',
      'srv',
      'opt',
    ]);
    expect(largestDirs.map((dir) => dir.allocated)).toEqual(
      [...largestDirs.map((dir) => dir.allocated)].sort((a, b) => b - a),
    );
    expect(largestDirs.every((dir) => dir.kind === 'dir' && dir.allocated > 0)).toBe(true);
  });

  it('on a disk shaped like a real one (97% in /home) lists the one big folder, then the small ones', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api, '/real');
    const { largestDirs, node } = await api.getSummary(id, rootId);

    // /home is dominated by its only child, so `sam` stands in for it; it holds 97% of the disk. Nothing is
    // expanded after that, because eight candidates remain: the rest of the list is `/var`, `/usr`, `/opt`, a
    // 17 MB `/etc` and the first three of fourteen items of 8 KB.
    expect(largestDirs.map((dir) => dir.name)).toEqual([
      'sam',
      'var',
      'usr',
      'opt',
      'etc',
      'stub-1',
      'stub-2',
      'stub-3',
    ]);
    expect((largestDirs[0]?.allocated ?? 0) / node.allocated).toBeGreaterThan(0.97);
  });

  it('keeps /home in the list when its biggest child holds less than 80% of it', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api, '/skew');
    const { largestDirs, node } = await api.getSummary(id, rootId);

    // vm-images is a folder of 18 files and holds 68% of /home, so /home stays; no mount, empty or unreadable folder is listed.
    expect(largestDirs.map((dir) => dir.name).slice(0, 5)).toEqual(['home', 'var', 'usr', 'opt', 'etc']);
    expect((largestDirs[0]?.allocated ?? 0) / node.allocated).toBeGreaterThan(0.97);
    expect(largestDirs.every((dir) => dir.kind === 'dir' && dir.allocated > 0)).toBe(true);
  });

  it('lists /home/demo but not /home, which is all demo', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const names = (await api.getSummary(id, rootId)).largestDirs.map((dir) => dir.name);

    expect(names).toContain('demo');
    expect(names).not.toContain('home');
  });

  it('gives every directory one block of its own, and a mount point none', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api, '/', false);
    const own = async (node: NodeDto): Promise<number> => {
      const kids = await allChildren(api, id, node.id);

      return node.allocated - kids.reduce((sum, kid) => sum + kid.allocated, 0);
    };
    // A folder the scan could not read, or that is empty, is shown with no size at all.
    const dirs = (await allChildren(api, id, rootId)).filter(
      (kid) => kid.kind === 'dir' && kid.allocated > 0,
    );

    expect(dirs.length).toBeGreaterThan(3);

    for (const dir of dirs) {
      expect(await own(dir)).toBe(4096);
    }

    expect(await own(await api.getNode(id, rootId))).toBe(4096);

    const stay = await scan(api, '/', true);
    const mount = (await allChildren(api, stay.id, stay.rootId)).find((kid) => kid.kind === 'mount');

    expect(mount?.allocated).toBe(0);
  });

  it('has a folder that could not be read and an empty one at the top, both without size', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const kids = await allChildren(api, id, rootId);
    const unreadable = kids.find((kid) => kid.unreadable);
    const empty = kids.find((kid) => kid.allocated === 0 && kid.kind === 'dir' && !kid.unreadable);

    expect(unreadable).toMatchObject({ kind: 'dir', allocated: 0, hasChildren: false });
    expect(empty).toMatchObject({ allocated: 0, hasChildren: false });
  });

  it('gives a file an empty summary of its own size', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const music = (await allChildren(api, id, rootId)).find((node) => node.name === 'swap');
    const [file] = await allChildren(api, id, music?.id ?? rootId);
    const summary = await api.getSummary(id, file?.id ?? rootId);

    expect(summary.largestDirs).toEqual([]);
    expect(summary.byCategory[0]?.allocated).toBe(file?.allocated);
  });
});

describe('getPath', () => {
  it('joins names below the scan root', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api, '/home/demo');
    const [child] = await allChildren(api, id, rootId);
    const [grandchild] = await allChildren(api, id, child?.id ?? rootId);

    expect((await api.getPath(id, rootId)).path).toBe('/home/demo');

    const path = await api.getPath(id, grandchild?.id ?? rootId);
    expect(path.path).toBe(`/home/demo/${child?.name}/${grandchild?.name}`);
    expect(path.segments.map((segment) => segment.id)).toEqual([rootId, child?.id, grandchild?.id]);
  });

  it('does not double the slash for the root filesystem', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api, '/');
    const [child] = await allChildren(api, id, rootId);

    expect((await api.getPath(id, child?.id ?? rootId)).path).toBe(`/${child?.name}`);
  });

  it('opening in the file manager checks the node exists', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);

    await expect(api.openInFileManager(id, rootId)).resolves.toBeUndefined();
    await expect(api.openInFileManager(id, 99_999_999)).rejects.toMatchObject({ code: 'notFound' });
  });
});

describe('openExternal', () => {
  it('opens the project pages in a new window that cannot reach back', async () => {
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    const api = createMockApi(FAST);

    await api.openExternal('source');
    await api.openExternal('issues');
    await api.openExternal('license');
    await api.openExternal('releases');

    expect(open.mock.calls).toEqual([
      ['https://github.com/MSpiechowicz/platter', '_blank', 'noopener'],
      ['https://github.com/MSpiechowicz/platter/issues', '_blank', 'noopener'],
      ['https://github.com/MSpiechowicz/platter/blob/main/LICENSE', '_blank', 'noopener'],
      ['https://github.com/MSpiechowicz/platter/releases/latest', '_blank', 'noopener'],
    ]);

    open.mockRestore();
  });
});

describe('the update commands', () => {
  it('finds no update unless the test hook offers one', async () => {
    expect(await createMockApi(FAST).checkUpdate()).toEqual({ current: '0.1.0', update: null });

    const found = await createMockApi({ ...FAST, update: { offer: { canInstall: false } } }).checkUpdate();

    expect(found.update).toMatchObject({ version: '0.2.0', canInstall: false });
  });
});

describe('getSuggestions', () => {
  it('lists safe, then review, then keep, largest first within each, and adds up to the totals', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api, '/');
    const { suggestions, totals } = await api.getSuggestions(id);

    const order = { safe: 0, review: 1, keep: 2 };
    const sorted = [...suggestions].sort(
      (a, b) => order[a.verdict] - order[b.verdict] || b.allocated - a.allocated,
    );

    expect(suggestions.map((entry) => entry.id)).toEqual(sorted.map((entry) => entry.id));
    expect(new Set(suggestions.map((entry) => entry.verdict))).toEqual(new Set(['safe', 'review', 'keep']));

    for (const verdict of ['safe', 'review', 'keep'] as const) {
      const sum = suggestions
        .filter((entry) => entry.verdict === verdict)
        .reduce((total, entry) => total + entry.allocated, 0);

      expect(totals[verdict]).toBe(sum);
      expect(sum).toBeGreaterThan(0);
    }
  });

  it('finds the demo disk folders at the paths it says, with the backend rule ids', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api, '/');
    const { suggestions } = await api.getSuggestions(id);
    const byId = new Map(suggestions.map((entry) => [entry.id, entry]));

    const pacman = byId.get('pacman-cache');
    const targets = byId.get('rust-target');

    expect(pacman).toMatchObject({
      path: '/var/cache/pacman/pkg',
      title: 'Pacman package cache',
      verdict: 'review',
    });
    expect((await api.getPath(id, pacman?.nodes[0] ?? 0)).path).toBe('/var/cache/pacman/pkg');
    expect(targets?.path).toBe('~/Projekty/**/target');
    expect(targets?.items).toBe(2);
    expect(targets?.reason).toBe(
      'Build output that cargo creates again on the next build. 2 projects with nothing modified in over 30 days.',
    );

    for (const node of targets?.nodes ?? []) {
      expect((await api.getPath(id, node)).path).toMatch(/^\/home\/demo\/Projekty\/[^/]+\/target$/);
    }

    expect(byId.get('btrfs-snapshots')?.verdict).toBe('review');
    expect(byId.get('steam-library')?.verdict).toBe('keep');
    expect(byId.get('trash')).toMatchObject({ path: '~/.local/share/Trash', verdict: 'safe' });
  });

  it('gives the verdicts of the backend table: pacman cache, journal, caches and Docker data are review', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api, '/');
    const { suggestions } = await api.getSuggestions(id);
    const verdict = (rule: string): string | undefined =>
      suggestions.find((entry) => entry.id === rule)?.verdict;

    expect(
      ['pacman-cache', 'journal', 'app-cache', 'docker-data', 'btrfs-snapshots', 'old-downloads'].map(
        verdict,
      ),
    ).toEqual(Array(6).fill('review'));
    expect(
      ['rust-target', 'node-modules', 'trash', 'npm-cache', 'cargo-registry', 'coredumps'].map(verdict),
    ).toEqual(Array(6).fill('safe'));
    expect(['steam-library', 'videos', 'pictures', 'music', 'documents'].map(verdict)).toEqual(
      Array(5).fill('keep'),
    );

    // Not in the table: the old mock's own ids.
    expect(suggestions.map((entry) => entry.id)).not.toContain('user-cache');
    expect(suggestions.map((entry) => entry.id)).not.toContain('system-journal');
  });

  it('splits Rust projects and node_modules by age: not touched for a while is safe, touched lately is review', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api, '/');
    const { suggestions } = await api.getSuggestions(id);
    const byId = new Map(suggestions.map((entry) => [entry.id, entry]));
    const nameOf = async (node: number): Promise<string> => (await api.getPath(id, node)).path;

    const idle = await Promise.all((byId.get('rust-target')?.nodes ?? []).map(nameOf));
    const recent = byId.get('rust-target-recent');

    expect(idle.sort()).toEqual([
      '/home/demo/Projekty/ocpp-bridge/target',
      '/home/demo/Projekty/rust-playground/target',
    ]);
    expect(recent).toMatchObject({ verdict: 'review', path: '~/Projekty/platter/target', items: 1 });
    expect(recent?.reason).toBe(
      '1 project touched in the last 30 days, so you are probably still working on them. Removing the build output means a full rebuild.',
    );
    expect(byId.get('node-modules-recent')).toMatchObject({
      verdict: 'review',
      path: '~/Projekty/platter/node_modules',
    });
    expect(byId.get('node-modules')?.items).toBe(2);
  });

  it('counts the files that are hard-linked from elsewhere, so the Advisor can warn that the space freed differs', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api, '/');
    const { suggestions } = await api.getSuggestions(id);

    // Every suggestion carries the count, like the backend's; node_modules (as pnpm makes it) has some, the rest none.
    expect(suggestions.every((entry) => Number.isInteger(entry.sharedFiles))).toBe(true);
    expect(suggestions.find((entry) => entry.id === 'node-modules')?.sharedFiles).toBeGreaterThan(0);
    expect(suggestions.find((entry) => entry.id === 'node-modules-recent')?.sharedFiles).toBeGreaterThan(0);
    expect(suggestions.find((entry) => entry.id === 'pacman-cache')?.sharedFiles).toBe(0);
  });

  it('keeps what the owner keeps untouched: nothing inside a kept folder is suggested, and a scan inside one gets nothing', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api, '/home/demo');
    const { suggestions } = await api.getSuggestions(id);
    const kept = suggestions.filter((entry) => entry.verdict === 'keep');

    expect(kept.map((entry) => entry.path).sort()).toEqual([
      '~/.local/share/Steam/steamapps/common',
      '~/Documents',
      '~/Music',
      '~/Pictures',
      '~/Videos',
    ]);

    // A scan of the kept folder itself is matched as a whole; one that starts below it gets nothing at all.
    const itself = await scan(api, '/home/demo/Videos');

    expect((await api.getSuggestions(itself.id)).suggestions.map((entry) => entry.id)).toEqual(['videos']);

    const inside = await scan(api, '/home/demo/Videos/recordings');

    expect(await api.getSuggestions(inside.id)).toEqual({
      suggestions: [],
      totals: { safe: 0, review: 0, keep: 0 },
    });
  });

  it('reads old files in Downloads and big disk images as separate file rules, with a glob for the path', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api, '/');
    const { suggestions } = await api.getSuggestions(id);
    const old = suggestions.find((entry) => entry.id === 'old-downloads');
    const images = suggestions.find((entry) => entry.id === 'disk-images');

    expect(old?.path).toBe('~/Downloads/**');
    expect(old?.reason).toMatch(/^\d+ files in Downloads not modified for 365 days or more\./);
    expect(images?.reason).toMatch(/^\d+ images of 1 GiB or more in your home folder\./);

    for (const node of old?.nodes ?? []) {
      const dto = await api.getNode(id, node);

      expect(dto.kind).toBe('file');
      expect(NOW - dto.mtime).toBeGreaterThanOrEqual(365 * 86_400);
    }
  });

  it('lists up to 20 matched nodes, largest first, and never counts a byte twice', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api, '/');
    const { suggestions, totals } = await api.getSuggestions(id);
    const paths: string[] = [];

    for (const suggestion of suggestions) {
      const nodes = await Promise.all(suggestion.nodes.map((node) => api.getNode(id, node)));

      expect(nodes.length).toBe(Math.min(suggestion.items, 20));
      expect(nodes.map((node) => node.allocated)).toEqual(
        [...nodes.map((node) => node.allocated)].sort((a, b) => b - a),
      );

      if (suggestion.items <= 20) {
        expect(nodes.reduce((sum, node) => sum + node.allocated, 0)).toBe(suggestion.allocated);
      }

      paths.push(
        ...(await Promise.all(suggestion.nodes.map(async (node) => (await api.getPath(id, node)).path))),
      );
    }

    // No matched folder is inside another matched folder.
    for (const path of paths) {
      expect(paths.filter((other) => other !== path && path.startsWith(`${other}/`))).toEqual([]);
    }

    const root = await api.getNode(id, 0);

    expect(totals.safe + totals.review + totals.keep).toBeLessThan(root.allocated);
  });

  it('finds fewer things in a folder scan, and the same things every time', async () => {
    const api = createMockApi(FAST);
    const first = await scan(api, '/home/demo');
    const second = await scan(api, '/home/demo');
    const { suggestions } = await api.getSuggestions(first.id);

    // Safe, then review, then keep; largest first within each.
    expect(suggestions.map((entry) => entry.id)).toEqual([
      'rust-target',
      'node-modules',
      'steam-shader-cache',
      'trash',
      'npm-cache',
      'cargo-registry',
      'rust-target-recent',
      'app-cache',
      'old-downloads',
      'node-modules-recent',
      'videos',
      'steam-library',
      'pictures',
      'music',
      'documents',
    ]);
    expect(await api.getSuggestions(second.id)).toEqual(await api.getSuggestions(first.id));
  });

  it('finds suggestions in the lopsided demo disk too', async () => {
    const api = createMockApi({ ...FAST, dominant: true });
    const { id } = await scan(api, '/');
    const { suggestions } = await api.getSuggestions(id);

    expect(suggestions.length).toBeGreaterThanOrEqual(5);
    expect(suggestions.find((entry) => entry.id === 'app-cache')?.path).toBe('~/.cache');
  });

  it('rejects a scan that does not exist', async () => {
    await expect(createMockApi(FAST).getSuggestions(4242)).rejects.toMatchObject({ code: 'notFound' });
  });
});

describe('the Windows demo disk', () => {
  const WINDOWS = { ...FAST, os: 'windows' as const };

  it('lists the drives C:\\ and D:\\ and offers folders with backslashes', async () => {
    const api = createMockApi(WINDOWS);

    expect((await api.listVolumes()).map((volume) => volume.mountPoint)).toEqual(['C:\\', 'D:\\']);
    expect(await api.pickFolder()).toBe('C:\\Users\\demo');
    expect(await api.pickExcludedFolder()).toBe('C:\\Users\\demo\\Videos');
  });

  it('takes a folder to leave out that starts at a drive, not a bare drive letter', async () => {
    const api = createMockApi(WINDOWS);
    const start = (excluded: string[]) =>
      api.startScan({ root: 'C:\\', oneFileSystem: true, excluded }, () => {});

    await expect(start(['C:\\Users\\demo\\Videos'])).resolves.toBeTypeOf('number');
    await expect(start(['C:'])).rejects.toMatchObject({ code: 'invalid' });
    await expect(start(['Users\\demo'])).rejects.toMatchObject({ code: 'invalid' });
  });

  it('scans the drive with backslash paths and the volume usage', async () => {
    const api = createMockApi(WINDOWS);
    const { id, rootId, totals } = await scan(api, 'C:\\');
    const [volume] = await api.listVolumes();
    const top = await allChildren(api, id, rootId);
    const users = top.find((child) => child.name === 'Users');
    const [profile] = await allChildren(api, id, users?.id ?? rootId);

    expect(totals.allocated).toBeGreaterThan(0);
    expect(volume?.total).toBeGreaterThan(totals.allocated);
    expect((await api.getPath(id, rootId)).path).toBe('C:\\');
    expect((await api.getPath(id, users?.id ?? rootId)).path).toBe('C:\\Users');
    expect((await api.getPath(id, profile?.id ?? rootId)).path).toBe('C:\\Users\\demo');
    expect(top.find((child) => child.name === '$Recycle.Bin')).toMatchObject({ kind: 'mount' });
  });

  it('suggests folders of the profile by their paths, with ~ for the profile folder', async () => {
    const api = createMockApi(WINDOWS);
    const { id } = await scan(api, 'C:\\');
    const { suggestions } = await api.getSuggestions(id);
    const targets = suggestions.find((entry) => entry.id === 'rust-target');

    expect(targets?.path).toBe('~\\Projekty\\rust-playground\\target');
    expect(suggestions.find((entry) => entry.id === 'node-modules')?.path).toBe(
      '~\\Projekty\\**\\node_modules',
    );

    const first = targets?.nodes[0] ?? 0;

    expect((await api.getPath(id, first)).path).toMatch(/^C:\\Users\\demo\\Projekty\\[^\\]+\\target$/);
  });
});
