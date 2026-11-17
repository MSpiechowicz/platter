import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Api } from './client';
import { createMockApi } from './mock';
import { RULE_IDS } from './mock-suggestions';
import type { CleanupEvent, CleanupPlanDto, ScanEvent, ScanId } from './types';

const NOW = 1_750_000_000;
const FAST = { durationMs: 0, now: () => NOW };

function scan(api: Api, root = '/'): Promise<ScanId> {
  return new Promise((resolve, reject) => {
    api
      .startScan({ root, oneFileSystem: true }, (event: ScanEvent) => {
        if (event.type === 'finished') {
          resolve(event.scanId);
        } else if (event.type === 'failed' || event.type === 'cancelled') {
          reject(new Error(`scan ended with ${event.type}`));
        }
      })
      .catch(reject);
  });
}

/** Starts a plan and collects its events until the last one. */
async function runPlan(
  api: Api,
  plan: CleanupPlanDto,
): Promise<{ id: number; events: CleanupEvent[]; finished: Extract<CleanupEvent, { type: 'finished' }> }> {
  const events: CleanupEvent[] = [];
  const id = await api.startCleanup(plan.planId, (event) => events.push(event));

  await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));

  return { id, events, finished: events.at(-1) as Extract<CleanupEvent, { type: 'finished' }> };
}

describe('planCleanup', () => {
  it('groups the chosen suggestions as the backend does: action per rule, tool command, elevation, paths', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const { suggestions } = await api.getSuggestions(id);
    const plan = await api.planCleanup(id, [
      'trash',
      'pacman-cache',
      'rust-target',
      'app-cache',
      'old-downloads',
    ]);
    const group = (rule: string) => plan.groups.find((entry) => entry.ruleId === rule);
    const suggestion = (rule: string) => suggestions.find((entry) => entry.id === rule);

    expect(plan.scanId).toBe(id);
    expect(plan.planId).toBeGreaterThan(0);
    expect(group('rust-target')).toMatchObject({ action: 'delete', verdict: 'safe', needsElevation: false });
    expect(group('app-cache')).toMatchObject({ action: 'emptyFolder' });
    expect(group('trash')).toMatchObject({ action: 'emptyTrash', title: suggestion('trash')?.title });
    expect(group('old-downloads')).toMatchObject({ action: 'trash', verdict: 'review' });
    expect(group('pacman-cache')).toMatchObject({
      action: 'runTool',
      command: 'paccache -rk2',
      needsElevation: true,
    });
    expect(group('rust-target')).not.toHaveProperty('command');

    expect(group('rust-target')?.items).toBe(2);
    expect(group('rust-target')?.samplePaths.sort()).toEqual([
      '/home/demo/Projekty/ocpp-bridge/target',
      '/home/demo/Projekty/rust-playground/target',
    ]);
    expect(plan.allocated).toBe(plan.groups.reduce((sum, entry) => sum + entry.allocated, 0));
    expect(plan.groups.every((entry) => entry.samplePaths.length <= 20)).toBe(true);
  });

  it('fixes the options into the plan: the kept pacman versions, and nothing needing the password when that is off', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);

    const standard = await api.planCleanup(id, ['pacman-cache', 'rust-target']);
    expect(standard.options).toEqual({ keepPackageVersions: 2, askForAdmin: true });

    const three = await api.planCleanup(id, ['pacman-cache'], { keepPackageVersions: 3, askForAdmin: true });
    expect(three.groups[0]).toMatchObject({ ruleId: 'pacman-cache', command: 'paccache -rk3' });

    const noAdmin = await api.planCleanup(id, ['pacman-cache', 'rust-target'], {
      keepPackageVersions: 2,
      askForAdmin: false,
    });
    expect(noAdmin.groups.map((group) => group.ruleId)).toEqual(['rust-target']);
    expect(noAdmin.refused[0]).toMatchObject({
      ruleId: 'pacman-cache',
      reason: 'Needs administrator rights, which are turned off in Settings.',
    });
  });

  it('refuses options the backend refuses', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);

    await expect(
      api.planCleanup(id, ['rust-target'], { keepPackageVersions: 4 as never, askForAdmin: true }),
    ).rejects.toMatchObject({ code: 'invalid' });
  });

  it('plans groups in the rule table order', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const plan = await api.planCleanup(id, ['trash', 'rust-target', 'pacman-cache']);
    const positions = plan.groups.map((entry) => RULE_IDS.indexOf(entry.ruleId));

    expect(positions).toEqual([...positions].sort((a, b) => a - b));
  });

  it('asks for confirmation whenever the plan does something, moving to the trash too', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);

    expect((await api.planCleanup(id, ['old-downloads'])).needsConfirmation).toBe(true);
    expect((await api.planCleanup(id, ['old-downloads', 'rust-target'])).needsConfirmation).toBe(true);
    expect((await api.planCleanup(id, ['pacman-cache'])).needsConfirmation).toBe(true);
  });

  it('refuses what is kept or not cleaned, with a reason, and makes no group of it', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const plan = await api.planCleanup(id, ['steam-library', 'btrfs-snapshots', 'rust-target']);

    expect(plan.groups.map((entry) => entry.ruleId)).toEqual(['rust-target']);
    expect(plan.refused.filter((entry) => entry.ruleId === 'steam-library')).not.toHaveLength(0);
    expect(plan.refused.find((entry) => entry.ruleId === 'steam-library')?.reason).toBe('kept on purpose');
    expect(plan.refused.find((entry) => entry.ruleId === 'btrfs-snapshots')?.reason).toBe(
      'Platter does not clean this',
    );
    expect(plan.needsConfirmation).toBe(true);

    const onlyKept = await api.planCleanup(id, ['documents']);

    expect(onlyKept.groups).toEqual([]);
    expect(onlyKept.allocated).toBe(0);
    expect(onlyKept.needsConfirmation).toBe(false);
    expect(onlyKept.refusedMore).toBe(0);
    await expect(api.startCleanup(onlyKept.planId, () => {})).rejects.toMatchObject({ code: 'invalid' });
  });

  it('lists a real rule that matched nothing in this scan as notFound', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const { suggestions } = await api.getSuggestions(id);
    const absent = RULE_IDS.find((rule) => !suggestions.some((entry) => entry.id === rule));

    expect(absent).toBeDefined();

    const plan = await api.planCleanup(id, [absent as string, 'rust-target']);

    expect(plan.notFound).toEqual([absent]);
    expect(plan.groups.map((entry) => entry.ruleId)).toEqual(['rust-target']);
  });

  it('is invalid with no ids, an unknown id, too many ids, and is notFound for an unknown scan', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);

    await expect(api.planCleanup(id, [])).rejects.toMatchObject({ code: 'invalid' });
    await expect(api.planCleanup(id, ['rust-target', 'no-such-rule'])).rejects.toMatchObject({
      code: 'invalid',
    });
    await expect(api.planCleanup(id, Array(65).fill('rust-target'))).rejects.toMatchObject({
      code: 'invalid',
    });
    await expect(api.planCleanup(4242, ['rust-target'])).rejects.toMatchObject({ code: 'notFound' });
  });

  it('keeps only the newest four plans', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const plans: CleanupPlanDto[] = [];

    for (let index = 0; index < 5; index++) {
      plans.push(await api.planCleanup(id, ['rust-target']));
    }

    const [oldest, next] = plans.map((entry) => entry.planId) as [number, number];

    await expect(api.startCleanup(oldest, () => {})).rejects.toMatchObject({ code: 'notFound' });
    await expect(api.startCleanup(next, () => {})).resolves.toBeGreaterThan(0);
  });
});

describe('startCleanup', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('plays each group as started, one done per target, finished, then closes with the totals', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const plan = await api.planCleanup(id, ['rust-target', 'pacman-cache', 'old-downloads']);
    const { id: cleanup, events, finished } = await runPlan(api, plan);

    const expected = plan.groups.flatMap((entry) => [
      `groupStarted:${entry.ruleId}`,
      ...Array(entry.items).fill(`targetDone:${entry.ruleId}`),
      `groupFinished:${entry.ruleId}`,
    ]);
    const seen = events
      .slice(0, -1)
      .map((event) => ('ruleId' in event ? `${event.type}:${event.ruleId}` : event.type));

    expect(seen).toEqual(expected);
    expect(events.filter((event) => event.type === 'finished')).toHaveLength(1);
    expect(events.find((event) => event.type === 'groupStarted')).toMatchObject({
      targets: plan.groups[0]?.items,
    });
    expect(finished).toMatchObject({
      cleanupId: cleanup,
      outdatedScan: id,
      cleaned: plan.groups.reduce((sum, entry) => sum + entry.items, 0),
      withProblems: 0,
      // What went to the trash frees nothing yet.
      freed: plan.groups
        .filter((entry) => entry.action !== 'trash')
        .reduce((sum, entry) => sum + entry.allocated, 0),
      cancelled: false,
      toolsNotRun: [],
      canUndo: true,
    });
  });

  it('reports the outcome per action: removed, trashed, or run by the tool with its command', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const plan = await api.planCleanup(id, ['rust-target', 'pacman-cache', 'old-downloads']);
    const { events } = await runPlan(api, plan);
    const outcome = (rule: string) =>
      events.flatMap((event) =>
        event.type === 'targetDone' && event.ruleId === rule ? [event.outcome] : [],
      );

    expect(new Set(outcome('rust-target').map((entry) => entry.kind))).toEqual(new Set(['removed']));
    expect(new Set(outcome('old-downloads').map((entry) => entry.kind))).toEqual(new Set(['trashed']));
    expect(outcome('pacman-cache')).toEqual([{ kind: 'byTool', detail: 'paccache -rk2', problems: 0 }]);
    expect(outcome('rust-target')[0]).not.toHaveProperty('detail');
  });

  it('cannot be undone when nothing was moved to the trash', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const { id: cleanup, finished } = await runPlan(api, await api.planCleanup(id, ['rust-target']));

    expect(finished.canUndo).toBe(false);
    await expect(api.undoCleanup(cleanup)).resolves.toEqual([]);
  });

  it('starts a plan once: the second time it does not exist', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const plan = await api.planCleanup(id, ['old-downloads']);

    await runPlan(api, plan);

    await expect(api.startCleanup(plan.planId, () => {})).rejects.toMatchObject({ code: 'notFound' });
    await expect(api.startCleanup(9999, () => {})).rejects.toMatchObject({ code: 'notFound' });
  });

  it('runs one cleanup at a time, and the refused plan is still there afterwards', async () => {
    const api = createMockApi({ ...FAST, durationMs: 200 });
    const id = await scan(api);
    const first = await api.planCleanup(id, ['rust-target']);
    const second = await api.planCleanup(id, ['old-downloads']);
    const events: CleanupEvent[] = [];

    await api.startCleanup(first.planId, (event) => events.push(event));
    await expect(api.startCleanup(second.planId, () => {})).rejects.toMatchObject({ code: 'invalid' });
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));

    // The refused plan was not used up, but the scan is outdated now.
    await expect(api.startCleanup(second.planId, () => {})).rejects.toMatchObject({ code: 'invalid' });
    await expect(api.startCleanup(second.planId, () => {})).rejects.toMatchObject({ code: 'notFound' });
  });

  it('resolves with the id before the first event arrives', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const plan = await api.planCleanup(id, ['rust-target']);
    const events: CleanupEvent[] = [];
    const cleanup = api.startCleanup(plan.planId, (event) => events.push(event));

    await cleanup;
    expect(events).toEqual([]);
    await vi.waitFor(() => expect(events.length).toBeGreaterThan(0));
  });

  it('spreads the events over time when scans take time, and plays at once when they do not', async () => {
    const slow = createMockApi({ ...FAST, durationMs: 300 });
    const slowScan = await scan(slow);
    const events: CleanupEvent[] = [];

    await slow.startCleanup((await slow.planCleanup(slowScan, ['rust-target'])).planId, (event) =>
      events.push(event),
    );
    await new Promise((resolve) => setTimeout(resolve, 100));

    expect(events.some((event) => event.type === 'targetDone')).toBe(true);
    expect(events.some((event) => event.type === 'finished')).toBe(false);
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));
  });

  it('marks the scan outdated: it gets no new plan, and an older plan of it cannot start', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const other = await scan(api);
    const older = await api.planCleanup(id, ['old-downloads']);
    const used = await api.planCleanup(id, ['rust-target']);
    const elsewhere = await api.planCleanup(other, ['rust-target']);

    await runPlan(api, used);

    await expect(api.planCleanup(id, ['rust-target'])).rejects.toMatchObject({ code: 'invalid' });
    await expect(api.startCleanup(older.planId, () => {})).rejects.toMatchObject({ code: 'invalid' });

    // Another scan is not affected, and the outdated one can still be read.
    await expect(api.planCleanup(other, ['rust-target'])).resolves.toMatchObject({ scanId: other });
    await expect(api.getSuggestions(id)).resolves.toBeDefined();
    await expect(runPlan(api, elsewhere)).resolves.toMatchObject({ finished: { outdatedScan: other } });
  });

  it('refuses a plan whose scan was dropped, and one that waited too long', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });

    const api = createMockApi(FAST);
    const id = await scan(api);
    const dropped = await api.planCleanup(id, ['rust-target']);
    const stale = await api.planCleanup(id, ['old-downloads']);

    await api.dropScan(id);
    await expect(api.startCleanup(dropped.planId, () => {})).rejects.toMatchObject({ code: 'invalid' });

    vi.setSystemTime(Date.now() + 10 * 60 * 1000 + 1);
    await expect(api.startCleanup(stale.planId, () => {})).rejects.toMatchObject({ code: 'invalid' });
    await expect(api.startCleanup(stale.planId, () => {})).rejects.toMatchObject({ code: 'notFound' });
  });

  it('proceeds without asking even when the plan cannot be undone (the browser has no native dialog)', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const plan = await api.planCleanup(id, ['rust-target', 'pacman-cache']);

    expect(plan.needsConfirmation).toBe(true);
    await expect(runPlan(api, plan)).resolves.toMatchObject({ finished: { cancelled: false } });
  });
});

describe('cancelCleanup', () => {
  it('stops after the target in progress, closes the group and reports cancelled', async () => {
    const api = createMockApi({ ...FAST, durationMs: 400 });
    const id = await scan(api);
    const plan = await api.planCleanup(id, ['rust-target', 'app-cache']);
    const events: CleanupEvent[] = [];
    const cleanup = await api.startCleanup(plan.planId, (event) => events.push(event));

    await vi.waitFor(() => expect(events.some((event) => event.type === 'targetDone')).toBe(true));
    await api.cancelCleanup(cleanup);
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));

    const done = events.filter((event) => event.type === 'targetDone').length;
    const planned = plan.groups.reduce((sum, entry) => sum + entry.items, 0);
    const finished = events.at(-1) as Extract<CleanupEvent, { type: 'finished' }>;

    expect(finished).toMatchObject({ cancelled: true, outdatedScan: id, cleaned: done });
    expect(done).toBeLessThan(planned);
    expect(finished.freed).toBe(
      events.reduce((sum, event) => sum + (event.type === 'targetDone' ? event.allocated : 0), 0),
    );
    expect(events.filter((event) => event.type === 'groupStarted')).toHaveLength(
      events.filter((event) => event.type === 'groupFinished').length,
    );

    // The scan is outdated all the same, and the slot is free again.
    await expect(api.planCleanup(id, ['rust-target'])).rejects.toMatchObject({ code: 'invalid' });
  });

  it('does not fail for an unknown or ended cleanup', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const { id: cleanup } = await runPlan(api, await api.planCleanup(id, ['rust-target']));

    await expect(api.cancelCleanup(9999)).resolves.toBeUndefined();
    await expect(api.cancelCleanup(cleanup)).resolves.toBeUndefined();
  });

  it('frees the single slot, so another cleanup can follow a cancelled one', async () => {
    const api = createMockApi({ ...FAST, durationMs: 100 });
    const id = await scan(api);
    const other = await scan(api);
    const events: CleanupEvent[] = [];
    const cleanup = await api.startCleanup((await api.planCleanup(id, ['rust-target'])).planId, (event) =>
      events.push(event),
    );

    await api.cancelCleanup(cleanup);
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));

    await expect(runPlan(api, await api.planCleanup(other, ['rust-target']))).resolves.toBeDefined();
  });
});

describe('undoCleanup', () => {
  it('restores what was moved to the trash, once, then it is notFound', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const plan = await api.planCleanup(id, ['old-downloads', 'rust-target']);
    const { id: cleanup, events } = await runPlan(api, plan);
    const trashed = events.flatMap((event) =>
      event.type === 'targetDone' && event.outcome.kind === 'trashed' ? [event.path] : [],
    );

    expect(trashed.length).toBeGreaterThan(0);

    const undone = await api.undoCleanup(cleanup);

    expect(undone).toEqual(trashed.map((path) => ({ path, restored: true })));
    await expect(api.undoCleanup(cleanup)).rejects.toMatchObject({ code: 'notFound' });
  });

  it('rejects with "cancelled" when the confirmation is declined, and can still be undone after', async () => {
    let approve = false;
    const api = createMockApi({ ...FAST, confirmUndo: () => approve });
    const id = await scan(api);
    const { id: cleanup } = await runPlan(api, await api.planCleanup(id, ['old-downloads', 'rust-target']));

    await expect(api.undoCleanup(cleanup)).rejects.toMatchObject({ code: 'cancelled' });

    approve = true;

    await expect(api.undoCleanup(cleanup)).resolves.not.toHaveLength(0);
  });

  it('is notFound for an unknown cleanup and while the cleanup is still running', async () => {
    const api = createMockApi({ ...FAST, durationMs: 200 });
    const id = await scan(api);
    const events: CleanupEvent[] = [];
    const cleanup = await api.startCleanup((await api.planCleanup(id, ['old-downloads'])).planId, (event) =>
      events.push(event),
    );

    await expect(api.undoCleanup(9999)).rejects.toMatchObject({ code: 'notFound' });
    await expect(api.undoCleanup(cleanup)).rejects.toMatchObject({ code: 'notFound' });
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));
    await expect(api.undoCleanup(cleanup)).resolves.not.toHaveLength(0);
  });
});

/** The node at a path of the mock disk, found by name from the scan root. */
async function nodeAt(api: Api, scan: ScanId, path: string): Promise<number> {
  let at = 0;

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

describe('planMarkedCleanup', () => {
  it('plans one review group that moves the marked items to the trash', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const downloads = await nodeAt(api, id, '/home/demo/Downloads');
    const cache = await nodeAt(api, id, '/home/demo/.cache');
    const plan = await api.planMarkedCleanup(id, [downloads, cache]);

    expect(plan.scanId).toBe(id);
    expect(plan.planId).toBeGreaterThan(0);
    expect(plan.groups).toHaveLength(1);
    expect(plan.groups[0]).toMatchObject({
      ruleId: 'marked',
      title: 'Marked in the Explorer',
      verdict: 'review',
      action: 'trash',
      items: 2,
      needsElevation: false,
    });
    expect(plan.groups[0]?.samplePaths.sort()).toEqual(['/home/demo/.cache', '/home/demo/Downloads']);
    expect(plan.groups[0]).not.toHaveProperty('command');
    expect(plan.allocated).toBe(plan.groups[0]?.allocated);
    expect(plan.refused).toEqual([]);
    expect(plan.needsConfirmation).toBe(true);
  });

  it('refuses what the backend refuses: the scan root, kept items, what holds or lies in them, and an item inside another', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const demo = await nodeAt(api, id, '/home/demo');
    const downloads = await nodeAt(api, id, '/home/demo/Downloads');
    const videos = await nodeAt(api, id, '/home/demo/Videos');
    const local = await nodeAt(api, id, '/home/demo/.local');
    const plan = await api.planMarkedCleanup(id, [0, videos, local, downloads, demo]);

    const reason = (path: string) => plan.refused.find((item) => item.path === path)?.reason;

    expect(reason('/')).toBe('it is the scanned folder itself');
    expect(reason('/home/demo/Videos')).toBe('kept on purpose');
    // Holds the Steam library, which a keep rule protects.
    expect(reason('/home/demo/.local')).toBe('kept on purpose');
    // The folder around everything that is left stays; the one inside it is left to it.
    expect(reason('/home/demo')).toBe('kept on purpose');
    expect(plan.refused.every((item) => item.ruleId === 'marked')).toBe(true);
    expect(plan.groups.flatMap((group) => group.samplePaths)).toEqual(['/home/demo/Downloads']);
  });

  it('leaves an item inside another marked one to the outer', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const projekty = await nodeAt(api, id, '/home/demo/Projekty');
    const inside = await nodeAt(api, id, '/home/demo/Projekty/web-shop');
    const plan = await api.planMarkedCleanup(id, [inside, projekty]);

    expect(plan.groups[0]?.samplePaths).toEqual(['/home/demo/Projekty']);
    expect(plan.groups[0]?.items).toBe(1);
    expect(plan.refused).toEqual([
      { ruleId: 'marked', path: '/home/demo/Projekty/web-shop', reason: 'inside another item of the plan' },
    ]);
  });

  it('counts an id named twice once', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const downloads = await nodeAt(api, id, '/home/demo/Downloads');
    const plan = await api.planMarkedCleanup(id, [downloads, downloads]);

    expect(plan.groups[0]?.items).toBe(1);
  });

  it('is invalid for no ids, for more than 1000, and for an outdated scan; unknown ids are not found', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const downloads = await nodeAt(api, id, '/home/demo/Downloads');

    await expect(api.planMarkedCleanup(id, [])).rejects.toMatchObject({ code: 'invalid' });
    await expect(
      api.planMarkedCleanup(
        id,
        Array.from({ length: 1001 }, () => downloads),
      ),
    ).rejects.toMatchObject({ code: 'invalid' });
    await expect(
      api.planMarkedCleanup(
        id,
        Array.from({ length: 1000 }, () => downloads),
      ),
    ).resolves.toMatchObject({ groups: [{ items: 1 }] });
    await expect(api.planMarkedCleanup(id, [downloads, 9_999_999])).rejects.toMatchObject({
      code: 'notFound',
    });
    await expect(api.planMarkedCleanup(999, [downloads])).rejects.toMatchObject({ code: 'notFound' });

    // A cleanup ran: the scan is outdated, and a plan for it is invalid.
    const plan = await api.planMarkedCleanup(id, [downloads]);
    await runPlan(api, plan);

    await expect(api.planMarkedCleanup(id, [downloads])).rejects.toMatchObject({ code: 'invalid' });
  });

  it('is started, confirmed and undone like any plan: it trashes, frees nothing, and can be undone once', async () => {
    const api = createMockApi(FAST);
    const id = await scan(api);
    const plan = await api.planMarkedCleanup(id, [await nodeAt(api, id, '/home/demo/Downloads')]);
    const { id: cleanup, events, finished } = await runPlan(api, plan);

    expect(events.find((event) => event.type === 'targetDone')).toMatchObject({
      ruleId: 'marked',
      path: '/home/demo/Downloads',
      outcome: { kind: 'trashed' },
    });
    expect(finished).toMatchObject({ cleaned: 1, freed: 0, canUndo: true, outdatedScan: id });
    await expect(api.startCleanup(plan.planId, () => {})).rejects.toMatchObject({ code: 'notFound' });

    await expect(api.undoCleanup(cleanup)).resolves.toEqual([
      { path: '/home/demo/Downloads', restored: true },
    ]);
    await expect(api.undoCleanup(cleanup)).rejects.toMatchObject({ code: 'notFound' });
  });
});
