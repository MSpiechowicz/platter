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
  });

  it('does not merge when everything fits, and expands the largest directories first', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const all = await allChildren(api, id, rootId);

    const lod = await api.getLod(id, rootId, all.length + 1, 1);
    expect(lod.children.map((child) => child.node.id)).toEqual(all.map((n) => n.id));

    // With room for the root's children and a handful more, the biggest directory is the one that opens.
    const deeper = await api.getLod(id, rootId, all.length + 6, 2);
    const opened = deeper.children.filter((child) => child.children.length > 0);
    expect(opened.length).toBeGreaterThan(0);
    expect(opened[0]?.node.id).toBe(all[0]?.id);
  });

  it('rejects impossible limits', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);

    await expect(api.getLod(id, rootId, 0, 2)).rejects.toMatchObject({ code: 'invalid' });
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

  it('lists the 8 largest directories among children and grandchildren, without a directory its own child dominates', async () => {
    const api = createMockApi(FAST);
    const { id, rootId } = await scan(api);
    const summary = await api.getSummary(id, rootId);

    const dirs = async (node: number): Promise<NodeDto[]> =>
      (await allChildren(api, id, node)).filter((kid) => kid.kind === 'dir');
    const children = await dirs(rootId);
    const grandchildren = (await Promise.all(children.map((child) => dirs(child.id)))).flat();
    const dominated = new Set<number>();

    for (const child of children) {
      const inside = await dirs(child.id);

      if (inside.some((kid) => kid.allocated * 100 >= child.allocated * 80)) {
        dominated.add(child.id);
      }
    }

    const expected = [...children.filter((child) => !dominated.has(child.id)), ...grandchildren]
      .sort((a, b) => b.allocated - a.allocated || a.id - b.id)
      .slice(0, 8)
      .map((node) => node.id);

    expect(dominated.size).toBeGreaterThan(0);
    expect(summary.largestDirs).toHaveLength(8);
    expect(summary.largestDirs.map((dir) => dir.id)).toEqual(expected);
    expect(summary.largestDirs.every((dir) => dir.kind === 'dir')).toBe(true);
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

    expect(open.mock.calls).toEqual([
      ['https://github.com/MSpiechowicz/platter', '_blank', 'noopener'],
      ['https://github.com/MSpiechowicz/platter/issues', '_blank', 'noopener'],
      ['https://github.com/MSpiechowicz/platter/blob/main/LICENSE', '_blank', 'noopener'],
    ]);

    open.mockRestore();
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

  it('finds the demo disk folders at the paths it says', async () => {
    const api = createMockApi(FAST);
    const { id } = await scan(api, '/');
    const { suggestions } = await api.getSuggestions(id);
    const byId = new Map(suggestions.map((entry) => [entry.id, entry]));

    const pacman = byId.get('pacman-cache');
    const targets = byId.get('cargo-target');

    expect(pacman?.path).toBe('/var/cache/pacman/pkg');
    expect((await api.getPath(id, pacman?.nodes[0] ?? 0)).path).toBe('/var/cache/pacman/pkg');
    expect(targets?.path).toBe('~/Projekty/**/target');
    expect(targets?.items).toBe(2);

    for (const node of targets?.nodes ?? []) {
      expect((await api.getPath(id, node)).path).toMatch(/^\/home\/demo\/Projekty\/[^/]+\/target$/);
    }

    expect(byId.get('snapshots')?.verdict).toBe('review');
    expect(byId.get('steam-library')?.verdict).toBe('keep');
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

    expect(suggestions.map((entry) => entry.id)).toEqual([
      'cargo-target',
      'node-modules',
      'user-cache',
      'trash',
      'downloads',
      'steam-library',
    ]);
    expect(await api.getSuggestions(second.id)).toEqual(await api.getSuggestions(first.id));
  });

  it('finds suggestions in the lopsided demo disk too', async () => {
    const api = createMockApi({ ...FAST, dominant: true });
    const { id } = await scan(api, '/');
    const { suggestions } = await api.getSuggestions(id);

    expect(suggestions.length).toBeGreaterThanOrEqual(5);
    expect(suggestions.find((entry) => entry.id === 'user-cache')?.path).toBe('~/.cache');
  });

  it('rejects a scan that does not exist', async () => {
    await expect(createMockApi(FAST).getSuggestions(4242)).rejects.toMatchObject({ code: 'notFound' });
  });
});
