import { describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type Api } from '../api/client';
import { createMockApi } from '../api/mock';
import type { NodeId, ScanId } from '../api/types';
import { CleanupStore, MAX_MARKS, NOT_CLEANED, type Mark } from './cleanup.svelte';
import { ScanStore } from './scan.svelte';

const ROOT = { root: '/', oneFileSystem: true };

interface Setup {
  api: Api;
  scan: ScanStore;
  store: CleanupStore;
  scanId: ScanId;
}

/** A finished mock scan; the store has its suggestions, as the Explorer makes sure of. */
async function setup(wrap: (api: Api) => Api = (api) => api, load = true): Promise<Setup> {
  const api = wrap(createMockApi({ durationMs: 0, now: () => 1_750_000_000 }));
  const scan = new ScanStore(api);

  await scan.start(ROOT);
  await vi.waitFor(() => expect(scan.state.status).toBe('ready'));

  if (scan.state.status !== 'ready') {
    throw new Error('expected a finished scan');
  }

  const scanId = scan.state.scanId;
  const store = new CleanupStore(api, scan);

  store.adopt(scanId);

  if (load) {
    await store.ensureSuggestions(scanId);
  }

  return { api, scan, store, scanId };
}

/** The node at a path of the mock disk, found by name from the scan root. */
async function nodeAt(api: Api, scan: ScanId, path: string): Promise<NodeId> {
  let at: NodeId = 0;

  for (const name of path.split('/').filter(Boolean)) {
    const page = await api.getChildren(scan, at, 0, 50, 'nameAsc', name);
    const child = page.items.find((item) => item.name === name);

    if (!child) {
      throw new Error(`no ${name} in ${path}`);
    }

    at = child.id;
  }

  return at;
}

/** What the Explorer hands over when the user marks a node. */
async function markFor(api: Api, scan: ScanId, id: NodeId): Promise<Mark> {
  const [node, path] = await Promise.all([api.getNode(scan, id), api.getPath(scan, id)]);

  return {
    id,
    allocated: node.allocated,
    ancestors: path.segments.slice(0, -1).map((segment) => segment.id),
  };
}

const done = (store: CleanupStore) => (): void => {
  expect(store.phase.kind).toBe('done');
};

describe('marking', () => {
  it('marks and unmarks, and the total is what the marked items hold', async () => {
    const { api, store, scanId } = await setup();
    const downloads = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads'));
    const cache = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/.cache'));

    store.mark(downloads);
    store.mark(cache);

    expect(store.isMarked(downloads.id)).toBe(true);
    expect(store.outermostMarks.map((mark) => mark.id).sort()).toEqual([downloads.id, cache.id].sort());
    expect(store.markedBytes).toBe(downloads.allocated + cache.allocated);
    expect(store.canCleanMarked).toBe(true);

    store.unmark(downloads.id);

    expect(store.isMarked(downloads.id)).toBe(false);
    expect(store.markedBytes).toBe(cache.allocated);
  });

  it('counts only the outermost marks: an item inside another marked item is not counted twice', async () => {
    const { api, store, scanId } = await setup();
    const demo = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo'));
    const cache = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/.cache'));
    const inside = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads'));

    // The inner one first, then the outer one, then another inner one: the order does not matter.
    store.mark(cache);
    store.mark(demo);
    store.mark(inside);

    expect(store.marks.size).toBe(3);
    expect(store.outermostMarks.map((mark) => mark.id)).toEqual([demo.id]);
    expect(store.markedBytes).toBe(demo.allocated);

    store.unmark(demo.id);

    expect(store.outermostMarks.map((mark) => mark.id).sort()).toEqual([cache.id, inside.id].sort());
    expect(store.markedBytes).toBe(cache.allocated + inside.allocated);
  });

  it('sends the node ids of the outermost marks, and nothing else, to be planned', async () => {
    const { api, store, scanId } = await setup();
    const planMarkedCleanup = vi.spyOn(api, 'planMarkedCleanup');
    const demo = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo'));
    const inside = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads'));

    store.mark(inside);
    store.mark(demo);
    await store.cleanMarked();

    expect(planMarkedCleanup).toHaveBeenCalledExactlyOnceWith(scanId, [demo.id], {
      keepPackageVersions: 2,
      askForAdmin: true,
    });
  });

  it('refuses the scan root, a kept item, an item inside a kept folder, and the 1001st mark', async () => {
    const { api, store, scanId } = await setup();
    const videos = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Videos'));

    expect(store.keptNodes.has(videos.id)).toBe(true);

    store.mark({ id: 0, allocated: 10, ancestors: [] });
    store.mark(videos);
    store.mark({ id: 4_000_000, allocated: 10, ancestors: [...videos.ancestors, videos.id] });

    expect(store.marks.size).toBe(0);
    expect(store.isKept(4_000_000, [...videos.ancestors, videos.id])).toBe(true);

    for (let id = 0; id < MAX_MARKS; id++) {
      store.mark({ id: 10_000 + id, allocated: 1, ancestors: [0] });
    }

    expect(store.marks.size).toBe(MAX_MARKS);
    expect(store.markFull).toBe(true);

    store.mark({ id: 99_999, allocated: 1, ancestors: [0] });
    expect(store.marks.size).toBe(MAX_MARKS);

    // An item that is marked already can still be unmarked when the list is full.
    store.unmark(10_000);
    expect(store.markFull).toBe(false);
  });

  it('reads the kept and the safe nodes from the suggestions', async () => {
    const { api, store, scanId } = await setup();
    const { suggestions } = await api.getSuggestions(scanId);
    const nodesOf = (verdict: string): number[] =>
      suggestions.filter((item) => item.verdict === verdict).flatMap((item) => item.nodes);

    expect([...store.keptNodes].sort()).toEqual([...new Set(nodesOf('keep'))].sort());
    expect([...store.safeNodes].sort()).toEqual([...new Set(nodesOf('safe'))].sort());
    expect(store.keptNodes.size).toBeGreaterThan(0);
    expect(store.safeNodes.size).toBeGreaterThan(0);
    // Review nodes are neither.
    expect(nodesOf('review').every((id) => !store.keptNodes.has(id) && !store.safeNodes.has(id))).toBe(true);
  });

  it('keeps the marks while the same scan is shown again, and forgets them for another scan', async () => {
    const { api, store, scanId } = await setup();
    const mark = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads'));

    store.mark(mark);
    store.adopt(scanId);
    store.load(scanId, await api.getSuggestions(scanId));

    expect(store.isMarked(mark.id)).toBe(true);

    store.adopt(scanId + 1);

    expect(store.marks.size).toBe(0);
    expect(store.outermostMarks).toEqual([]);
    expect(store.canCleanMarked).toBe(false);
  });

  it('drops an old notice when the Explorer shows the scan, but never what a running cleanup says', async () => {
    const { api, store, scanId } = await setup();

    store.toggle('rust-target-recent');
    await store.cleanSelected();
    store.dismissReview();
    expect(store.notice).toBe(NOT_CLEANED);

    store.adopt(scanId);
    expect(store.notice).toBeUndefined();

    store.mark(await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads')));
    await store.cleanMarked();
    expect(store.phase.kind).toBe('review');

    store.adopt(scanId);
    expect(store.phase.kind).toBe('review');
    expect(store.plan).toBeDefined();
  });

  it('forgets the marks when the suggestions of another scan arrive', async () => {
    const { api, store, scanId } = await setup();

    store.mark(await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads')));
    store.load(scanId + 1, await api.getSuggestions(scanId));

    expect(store.marks.size).toBe(0);
  });

  it('forgets what was said about the last plan when the marks change', async () => {
    const { api, store, scanId } = await setup();
    const downloads = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads'));

    store.mark(downloads);
    await store.cleanMarked();
    store.dismissReview();

    expect(store.notice).toBe(NOT_CLEANED);

    store.unmark(downloads.id);

    expect(store.notice).toBeUndefined();
    expect(store.plan).toBeUndefined();
  });
});

describe('suggestions for the Explorer', () => {
  it('fetches them once, and does not fetch what the store has', async () => {
    const { api, store, scanId } = await setup();
    const getSuggestions = vi.spyOn(api, 'getSuggestions');

    await store.ensureSuggestions(scanId);

    expect(getSuggestions).not.toHaveBeenCalled();
    expect(store.suggestions.length).toBeGreaterThan(0);
  });

  it('leaves nothing shown as kept or safe when they cannot be read, and does not retry', async () => {
    const { api, store, scanId } = await setup(
      (api) => ({ ...api, getSuggestions: vi.fn().mockRejectedValue(new ApiRequestError('io', 'no')) }),
      false,
    );

    await store.ensureSuggestions(scanId);

    expect(store.suggestions).toEqual([]);
    expect(store.keptNodes.size).toBe(0);
    expect(api.getSuggestions).toHaveBeenCalledOnce();
  });

  it('drops an answer that arrives after the scan was replaced', async () => {
    let release: () => void = () => {};
    const late = {
      suggestions: [
        {
          id: 'videos',
          title: 'Videos',
          path: '~/Videos',
          verdict: 'keep' as const,
          reason: 'x',
          allocated: 1,
          items: 1,
          nodes: [7],
        },
      ],
      totals: { safe: 0, review: 0, keep: 1 },
    };
    const { store, scanId } = await setup(
      (api) => ({
        ...api,
        getSuggestions: async () => {
          await new Promise<void>((resolve) => (release = resolve));

          return late;
        },
      }),
      false,
    );

    const loading = store.ensureSuggestions(scanId);

    store.adopt(scanId + 1);
    release();
    await loading;

    expect(store.scanId).toBe(scanId + 1);
    expect(store.suggestions).toEqual([]);
    expect(store.keptNodes.size).toBe(0);
  });
});

describe('the hand-marked group of a plan', () => {
  it('is there while the plan is reviewed and while it runs and after, and goes with the scan', async () => {
    const { api, store, scanId } = await setup();

    expect(store.markedGroup).toBeUndefined();

    store.mark(await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads')));
    await store.cleanMarked();

    expect(store.reviewing).toBeDefined();
    expect(store.markedGroup).toMatchObject({ ruleId: 'marked', title: 'Marked in the Explorer', items: 1 });

    await store.confirmReview();
    await vi.waitFor(done(store));

    expect(store.markedGroup?.allocated).toBe(store.phase.kind === 'done' ? store.plan?.allocated : -1);
    expect(store.groups.get('marked')).toBe('done');

    store.load(scanId + 1, undefined);

    expect(store.markedGroup).toBeUndefined();
  });

  it('a dismissed review leaves no marked group behind', async () => {
    const { api, store, scanId } = await setup();

    store.mark(await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads')));
    await store.cleanMarked();
    store.dismissReview();

    expect(store.markedGroup).toBeUndefined();
  });
});

describe('cleaning what is marked', () => {
  it('asks for review first, runs after the confirmation, moves to the trash and offers undo', async () => {
    const { api, scan, store, scanId } = await setup();
    const startCleanup = vi.spyOn(api, 'startCleanup');
    const a = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads'));
    const b = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/.cache'));

    store.mark(a);
    store.mark(b);
    await store.cleanMarked();

    // Every marked item is a review item: the dialog opens, nothing runs yet.
    const plan = store.reviewing;

    expect(plan?.groups).toHaveLength(1);
    expect(plan?.groups[0]).toMatchObject({
      ruleId: 'marked',
      title: 'Marked in the Explorer',
      verdict: 'review',
      action: 'trash',
      items: 2,
    });
    expect(plan?.allocated).toBe(a.allocated + b.allocated);
    expect(startCleanup).not.toHaveBeenCalled();
    expect(store.canMark).toBe(false);

    await store.confirmReview();
    await vi.waitFor(done(store));

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    expect(startCleanup).toHaveBeenCalledOnce();
    expect(store.phase.result.trashed).toBe(2);
    expect(store.phase.result.undoable).toBe(true);
    expect(store.actions).toEqual(['undo', 'rescan']);
    // The scan no longer describes the disk: nothing can be marked or cleaned on it.
    expect(scan.state.status === 'ready' && scan.state.outdated).toBe(true);
    expect(store.canMark).toBe(false);
    expect(store.canCleanMarked).toBe(false);

    store.mark({ id: 12345, allocated: 1, ancestors: [0] });
    expect(store.isMarked(12345)).toBe(false);

    await store.undo();

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    expect(store.phase.result.undo?.restored).toBe(2);
    expect(store.phase.result.undoable).toBe(false);
    expect(store.actions).toEqual(['rescan']);
  });

  it('keeps the marks when the review is cancelled, and starts nothing', async () => {
    const { api, store, scanId } = await setup();
    const startCleanup = vi.spyOn(api, 'startCleanup');
    const mark = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads'));

    store.mark(mark);
    await store.cleanMarked();
    store.dismissReview();

    expect(store.phase.kind).toBe('idle');
    expect(store.notice).toBe(NOT_CLEANED);
    expect(store.isMarked(mark.id)).toBe(true);
    expect(store.canCleanMarked).toBe(true);
    expect(startCleanup).not.toHaveBeenCalled();
  });

  it('shows what the backend refused, and plans nothing when nothing is left', async () => {
    const { api, store, scanId } = await setup();
    // `.local` holds the Steam library, which a keep rule protects: the backend refuses it, the checkbox was there.
    const local = await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/.local'));

    expect(store.isKept(local.id, local.ancestors)).toBe(false);

    store.mark(local);
    await store.cleanMarked();

    expect(store.phase.kind).toBe('idle');
    expect(store.notice).toBe('Nothing from this selection can be cleaned.');
    expect(store.plan?.refused).toEqual([
      { ruleId: 'marked', path: '/home/demo/.local', reason: 'kept on purpose' },
    ]);
    expect(store.isMarked(local.id)).toBe(true);
  });

  it('says why a plan could not be made, and does not try again by itself', async () => {
    const { api, store, scanId } = await setup((api) => ({
      ...api,
      planMarkedCleanup: vi.fn().mockRejectedValue(new ApiRequestError('notFound', 'node 5 does not exist')),
    }));

    store.mark({ id: 5, allocated: 1, ancestors: [0] });
    await store.cleanMarked();

    expect(store.phase.kind).toBe('idle');
    expect(store.notice).toBe('Could not plan the cleanup: node 5 does not exist');
    expect(api.planMarkedCleanup).toHaveBeenCalledOnce();
    expect(api.planMarkedCleanup).toHaveBeenCalledWith(scanId, [5], {
      keepPackageVersions: 2,
      askForAdmin: true,
    });
  });

  it('does nothing without marks, while another flow runs, or twice for two clicks', async () => {
    const { api, store, scanId } = await setup();
    const planMarkedCleanup = vi.spyOn(api, 'planMarkedCleanup');

    await store.cleanMarked();
    expect(planMarkedCleanup).not.toHaveBeenCalled();

    store.mark(await markFor(api, scanId, await nodeAt(api, scanId, '/home/demo/Downloads')));

    const first = store.cleanMarked();

    await store.cleanMarked();
    await first;

    expect(planMarkedCleanup).toHaveBeenCalledOnce();
  });
});
