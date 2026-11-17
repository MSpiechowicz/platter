import { describe, expect, it, vi } from 'vitest';
import type { Api } from './client';
import { formatSize, type SizeUnits } from '../format';
import { isApiError } from './client';
import { createMockApi } from './mock';
import { setMockAdvisorState, type MockAdvisorOptions } from './mock-advisor';
import { FakeTimers } from './mock-advisor.testkit';
import type {
  AdvisorLabelsDto,
  AdvisorStatusDto,
  CleanupEvent,
  NodeId,
  ScanEvent,
  ScanId,
  SuggestionDto,
} from './types';

const NOW = 1_750_000_000;
const MIB = 1024 * 1024;
const LOAD_MS = 400;
const LABEL_MS = 300;
const ASK_MS = 800;

function scanOf(api: Api, root: string): Promise<ScanId> {
  return new Promise<ScanId>((resolve, reject) => {
    api
      .startScan({ root, oneFileSystem: true }, (event: ScanEvent) => {
        if (event.type === 'finished') {
          resolve(event.scanId);
        }
      })
      .catch(reject);
  });
}

function setup(
  advisor: MockAdvisorOptions = { state: 'disk' },
  root = '/',
): {
  api: Api;
  timers: FakeTimers;
  events: AdvisorStatusDto[];
  scan: Promise<ScanId>;
} {
  const timers = new FakeTimers();
  const api = createMockApi({ durationMs: 0, now: () => NOW, advisor: { timers, ...advisor } });
  const events: AdvisorStatusDto[] = [];
  const scan = scanOf(api, root);

  void api.watchAdvisor((status) => events.push(status));
  events.length = 0;

  return { api, timers, events, scan };
}

async function rejection(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }

  throw new Error('expected a rejection');
}

async function expectCode(promise: Promise<unknown>, code: string): Promise<void> {
  const error = await rejection(promise);

  expect(isApiError(error) && error.code).toBe(code);
}

/** The node id of a folder of the scan, found by its exact path. */
async function folder(api: Api, scan: ScanId, path: string): Promise<NodeId> {
  const [match] = await api.findFolders(scan, path, 1);

  expect(match?.path).toBe(path);

  return (match as { node: NodeId }).node;
}

/** Starts a labelling and plays it out to its end. */
async function labelAll(api: Api, timers: FakeTimers, scan: ScanId): Promise<AdvisorLabelsDto> {
  const { candidates } = await api.getAdvisorLabels(scan);

  await api.labelFolders(scan);
  timers.advance(LOAD_MS + LABEL_MS * (candidates + 1));

  return api.getAdvisorLabels(scan);
}

const sorted = (values: number[]): number[] => [...values].sort((a, b) => a - b);

describe('labelFolders: validation', () => {
  it('rejects an unknown scan as notFound', async () => {
    const { api } = setup();

    await expectCode(api.labelFolders(4242), 'notFound');
  });

  it('rejects a scan that is still running', async () => {
    const timers = new FakeTimers();
    const api = createMockApi({
      durationMs: 1000,
      holdAt: 0.5,
      now: () => NOW,
      advisor: { timers, state: 'disk' },
    });
    const scan = await api.startScan({ root: '/', oneFileSystem: true }, () => {});

    await expectCode(api.labelFolders(scan), 'invalid');

    await api.dropScan(scan);
  });

  it('rejects an outdated scan (a cleanup ran on it)', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const labels = await labelAll(api, timers, id);

    expect(labels.labelled).toBeGreaterThan(0);

    const plan = await api.planCleanup(id, ['trash']);
    const events: CleanupEvent[] = [];

    await api.startCleanup(plan.planId, (event) => events.push(event));
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));

    await expectCode(api.labelFolders(id), 'invalid');
  });

  it('rejects when the model is not downloaded, downloading or switched off', async () => {
    const none = setup({ state: 'none' });

    await expectCode(none.api.labelFolders(await none.scan), 'invalid');

    const downloading = setup({ state: 'downloading' });

    await expectCode(downloading.api.labelFolders(await downloading.scan), 'invalid');

    const off = setup({
      state: 'disk',
      options: { enabled: false, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true },
    });

    await expectCode(off.api.labelFolders(await off.scan), 'invalid');
    expect((await off.api.advisorStatus()).labelling).toBeUndefined();
  });

  it('rejects a second job while one runs, and an unload', async () => {
    const { api, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await api.labelFolders(id);

    await expectCode(api.labelFolders(id), 'invalid');
    await expectCode(api.unloadModel(), 'invalid');
    expect((await api.advisorStatus()).state).toBe('loaded');
  });
});

describe('labelFolders: a run', () => {
  it('loads a model on disk first, then reports progress folder by folder and ends quietly', async () => {
    const { api, timers, events, scan } = setup();
    const id = await scan;
    const { candidates } = await api.getAdvisorLabels(id);

    expect(candidates).toBeGreaterThan(3);

    await api.labelFolders(id);

    expect(events.map((event) => event.state)).toEqual(['loading']);
    expect(events[0]?.labelling).toEqual({ scan: id, done: 0, total: candidates });

    timers.advance(LOAD_MS);

    expect(events.at(-1)).toMatchObject({
      state: 'loaded',
      labelling: { scan: id, done: 0, total: candidates },
    });

    timers.advance(LABEL_MS * 3);

    expect(events.at(-1)?.labelling).toEqual({ scan: id, done: 3, total: candidates });
    // As in the backend, the labels of a run are stored when it ends.
    expect((await api.getAdvisorLabels(id)).labelled).toBe(0);

    timers.advance(LABEL_MS * candidates);

    const done = events.at(-1);

    expect(done?.state).toBe('loaded');
    expect(done?.labelling).toBeUndefined();
    expect(
      events.every((event) => event.labelling === undefined || event.labelling.total === candidates),
    ).toBe(true);

    const progress = events.flatMap((event) => (event.labelling ? [event.labelling.done] : []));

    expect(progress).toEqual(sorted(progress));
    expect((await api.getAdvisorLabels(id)).labelled).toBe(candidates);
  });

  it('labels large folders no rule covers, never a protected or rule-covered one, and never says safe', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const result = await labelAll(api, timers, id);
    const paths = result.labels.map((label) => label.path);

    expect(result.labelled).toBe(result.candidates);
    expect(paths).toContain('/var/tmp');
    expect(paths).toContain('/var/lib/flatpak');
    // Rules matched inside it (disk images), so it is entered, not labelled as a whole.
    expect(paths).not.toContain('/home/demo/.local/share/containers');
    expect(paths).not.toContain('/home/demo/.local');
    expect(paths).not.toContain('/var/lib');
    expect(paths).not.toContain('/home');
    expect(paths).not.toContain('/usr');
    expect(paths).not.toContain('/home/demo/.var');
    expect(paths).not.toContain('/home/demo/.cache');
    expect(paths).not.toContain('/var/lib/docker');
    expect(paths.some((path) => path.startsWith('/usr/') || path.startsWith('/home/demo/.cache/'))).toBe(
      false,
    );

    expect(result.labels.length).toBeLessThanOrEqual(50);
    expect(result.labels.every((label) => label.allocated >= 256 * MIB)).toBe(true);
    expect(result.labels.every((label) => label.verdict === 'review' || label.verdict === 'keep')).toBe(true);
    expect(result.labels.every((label) => label.reason.length > 0 && label.reason.length <= 200)).toBe(true);
    expect(result.labels.every((label) => !label.reason.includes('\n'))).toBe(true);
    expect(result.labels.map((label) => label.allocated)).toEqual(
      result.labels.map((label) => label.allocated).sort((a, b) => b - a),
    );
  });

  it('says review for names that look regenerable and keep for the rest', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const result = await labelAll(api, timers, await scan);
    const verdictOf = (path: string) => result.labels.find((label) => label.path === path)?.verdict;

    expect(verdictOf('/var/tmp')).toBe('review');
    expect(verdictOf('/var/lib/flatpak')).toBe('keep');
    expect(verdictOf('/home/demo/Projekty/platter/src')).toBe('keep');

    const home = setup({ state: 'loaded' }, '/home/demo');
    const inHome = await labelAll(home.api, home.timers, await home.scan);

    expect(inHome.labels.find((label) => label.path === '/home/demo/.local/share/containers')?.verdict).toBe(
      'review',
    );
  });

  it('takes at most 50 folders on a disk with more candidates', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' }, '/real');
    const id = await scan;
    const result = await labelAll(api, timers, id);

    expect(result.candidates).toBe(50);
    expect(result.labelled).toBe(50);
  });

  it('finds no candidates without a known home, and finishes with no labels', async () => {
    const { api, timers, events, scan } = setup({ state: 'loaded' }, '/mnt/backup');
    const id = await scan;

    expect((await api.getAdvisorLabels(id)).candidates).toBe(0);

    await api.labelFolders(id);
    timers.advance(LABEL_MS);

    expect(events.at(-1)?.labelling).toBeUndefined();
    expect((await api.getAdvisorLabels(id)).labelled).toBe(0);
  });

  it('answers a scan with no candidate with an empty run at once, without loading the model', async () => {
    const { api, events, scan } = setup({ state: 'disk' }, '/mnt/backup');
    const id = await scan;

    await api.labelFolders(id);

    expect(events).toEqual([]);
    expect(await api.advisorStatus()).toMatchObject({ state: 'onDisk' });
    expect(await api.getAdvisorLabels(id)).toMatchObject({ labelled: 0, candidates: 0 });
    expect((await api.getAdvisorLabels(id)).stopped).toBeUndefined();
  });

  it('shows tokens per second once something was generated, only while loaded', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;

    expect((await api.advisorStatus()).tokensPerSecond).toBeUndefined();

    await api.labelFolders(id);
    timers.advance(LABEL_MS);

    expect((await api.advisorStatus()).tokensPerSecond).toBeGreaterThan(0);

    timers.advance(LABEL_MS * 100);
    await api.unloadModel();

    expect((await api.advisorStatus()).tokensPerSecond).toBeUndefined();
  });

  it('idles the model only after the run, then unloads it as usual', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await api.configureAdvisor({ enabled: true, unloadAfter: 'oneMinute', logQuestions: true, useGpu: true });
    await labelAll(api, timers, id);

    expect((await api.advisorStatus()).state).toBe('loaded');

    timers.advance(59_000);

    expect((await api.advisorStatus()).state).toBe('loaded');

    timers.advance(1000);

    expect((await api.advisorStatus()).state).toBe('onDisk');
  });

  it('answers once the job is taken: a model switched off while it loads is a status problem, not an error', async () => {
    const { api, events, scan } = setup();
    const id = await scan;

    await expect(api.labelFolders(id)).resolves.toBeUndefined();
    expect(events.at(-1)).toMatchObject({ state: 'loading', labelling: { scan: id } });

    await api.configureAdvisor({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });

    const status = await api.advisorStatus();

    expect(status.state).toBe('onDisk');
    expect(status.labelling).toBeUndefined();
    expect(status.problem).toBeTruthy();
    expect(events.at(-1)).toEqual(status);
    expect((await api.getAdvisorLabels(id)).labelled).toBe(0);
  });

  it('replaces the labels of the scan when it runs again', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const first = await labelAll(api, timers, id);
    const second = await labelAll(api, timers, id);

    expect(second.labelled).toBe(first.labelled);
    expect(second.labels).toEqual(first.labels);
  });
});

describe('cancelLabelling and ending early', () => {
  it('keeps the labels made so far, says it was cancelled and goes quiet', async () => {
    const { api, timers, events, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const { candidates } = await api.getAdvisorLabels(id);

    await api.labelFolders(id);
    timers.advance(LABEL_MS * 2);
    await api.cancelLabelling();

    const result = await api.getAdvisorLabels(id);

    expect(result.labelled).toBe(2);
    expect(result.stopped).toBe('cancelled');
    expect(result.candidates).toBe(candidates);
    expect(events.at(-1)?.labelling).toBeUndefined();
    expect(timers.count).toBeGreaterThan(0);

    const pushed = events.length;

    timers.advance(LABEL_MS * 10);

    expect(events.length).toBe(pushed);
    expect((await api.getAdvisorLabels(id)).labelled).toBe(2);

    // The model is free again.
    await api.labelFolders(id);
    timers.advance(LABEL_MS * (candidates + 1));

    const again = await api.getAdvisorLabels(id);

    expect(again.labelled).toBe(candidates);
    expect(again.stopped).toBeUndefined();
  });

  it('keeps the earlier labels when a later run stops before it made any', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const first = await labelAll(api, timers, id);

    expect(first.labelled).toBeGreaterThan(0);

    await api.labelFolders(id);
    await api.cancelLabelling();
    timers.advance(LABEL_MS * 100);

    expect(await api.getAdvisorLabels(id)).toEqual(first);
  });

  it('keeps the earlier labels when the model is switched off during the load of a later run', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const first = await labelAll(api, timers, id);

    await api.unloadModel();
    await api.labelFolders(id);
    await api.configureAdvisor({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });
    timers.advance(LOAD_MS + LABEL_MS * 100);

    expect(await api.getAdvisorLabels(id)).toEqual(first);
  });

  it('replaces the earlier labels with what a later run stopped with, when that is something', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const first = await labelAll(api, timers, id);

    expect(first.labelled).toBeGreaterThan(2);

    await api.labelFolders(id);
    timers.advance(LABEL_MS * 2);
    await api.cancelLabelling();

    expect(await api.getAdvisorLabels(id)).toMatchObject({ labelled: 2, stopped: 'cancelled' });
  });

  it('replaces the earlier labels when a later run ends normally', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await labelAll(api, timers, id);
    await api.labelFolders(id);
    await api.cancelLabelling();

    const stopped = await api.getAdvisorLabels(id);

    expect(stopped.labelled).toBeGreaterThan(0);

    const second = await labelAll(api, timers, id);

    expect(second).toEqual(stopped);
    expect(second.stopped).toBeUndefined();
  });

  it('is not an error with nothing running', async () => {
    const { api, events } = setup({ state: 'loaded' });

    await api.cancelLabelling();

    expect(events).toEqual([]);
  });

  it('cancels while the model is still loading, ending with no labels once it has loaded', async () => {
    const { api, timers, events, scan } = setup();
    const id = await scan;

    await api.labelFolders(id);
    await api.cancelLabelling();
    timers.advance(LOAD_MS);

    const result = await api.getAdvisorLabels(id);

    // A run that stopped with no label stores nothing, so there is no earlier run to show either.
    expect(result).toMatchObject({ labelled: 0 });
    expect(result.stopped).toBeUndefined();
    expect(events.at(-1)).toMatchObject({ state: 'loaded' });
    expect(events.at(-1)?.labelling).toBeUndefined();
  });

  it('ends when the model is switched off, keeping what was labelled', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await api.labelFolders(id);
    timers.advance(LABEL_MS * 2);

    const status = await api.configureAdvisor({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });

    expect(status.state).toBe('onDisk');
    expect(status.labelling).toBeUndefined();

    timers.advance(LABEL_MS * 100);

    expect(await api.getAdvisorLabels(id)).toMatchObject({ labelled: 2, stopped: 'cancelled' });
  });

  it('stops when the scan is dropped and forgets its labels', async () => {
    const { api, timers, events, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await api.labelFolders(id);
    timers.advance(LABEL_MS);
    await api.dropScan(id);
    timers.advance(LABEL_MS);

    expect(events.at(-1)?.labelling).toBeUndefined();
    await expectCode(api.getAdvisorLabels(id), 'notFound');
  });

  it('is ended by a URL-hook state change', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await api.labelFolders(id);
    setMockAdvisorState('disk');

    expect((await api.advisorStatus()).labelling).toBeUndefined();

    timers.advance(LABEL_MS * 100);

    expect((await api.advisorStatus()).state).toBe('onDisk');
  });
});

describe('getAdvisorLabels: the three steps', () => {
  it('counts rules, model and guard before any run', async () => {
    const { api, scan } = setup();
    const id = await scan;
    const result = await api.getAdvisorLabels(id);

    expect(result).toMatchObject({ scan: id, labelled: 0, labels: [], modelMs: 0 });
    expect(result.stopped).toBeUndefined();
    expect(result.candidates).toBeGreaterThan(0);
    expect(result.model).toEqual({ folders: 0, allocated: 0 });

    const { suggestions } = await api.getSuggestions(id);

    expect(result.rules.folders).toBe(suggestions.reduce((sum, entry) => sum + entry.items, 0));
    expect(result.rules.allocated).toBe(suggestions.reduce((sum, entry) => sum + entry.allocated, 0));
    expect(result.rules.example).toBeDefined();
    expect(result.guard.folders).toBeGreaterThan(0);
    expect(result.guard.example?.verdict).toBe('keep');
  });

  it('adds up the labelled folders with an example, and the run time', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const result = await labelAll(api, timers, id);

    expect(result.model.folders).toBe(result.labelled);
    expect(result.model.allocated).toBe(result.labels.reduce((sum, label) => sum + label.allocated, 0));
    expect(result.model.example).toEqual({
      path: result.labels[0]?.path,
      verdict: result.labels[0]?.verdict,
    });
    expect(result.modelMs).toBe(LABEL_MS * result.labelled);
  });

  it('gives each label the folder path and size of the scan', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const result = await labelAll(api, timers, id);

    for (const label of result.labels.slice(0, 3)) {
      expect(label.path).toBe((await api.getPath(id, label.node)).path);
      expect(label.allocated).toBe((await api.getNode(id, label.node)).allocated);
    }
  });

  it('rejects an unknown scan', async () => {
    const { api } = setup();

    await expectCode(api.getAdvisorLabels(4242), 'notFound');
  });
});

describe('suggestions and cleanup of the labels', () => {
  const advisorOf = (suggestions: SuggestionDto[]) => suggestions.find((entry) => entry.id === 'advisor');

  it('names the rules as the source of every suggestion, and has no advisor group without labels', async () => {
    const { api, scan } = setup();
    const { suggestions } = await api.getSuggestions(await scan);

    expect(suggestions.length).toBeGreaterThan(5);
    expect(suggestions.every((entry) => entry.source === 'rule')).toBe(true);
    expect(advisorOf(suggestions)).toBeUndefined();
  });

  it('lists the review labels as one review group made by the model, counted in the totals', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const before = await api.getSuggestions(id);
    const labels = await labelAll(api, timers, id);
    const after = await api.getSuggestions(id);
    const review = labels.labels.filter((label) => label.verdict === 'review');
    const group = advisorOf(after.suggestions);

    expect(review.length).toBeGreaterThan(0);
    expect(review.length).toBeLessThan(labels.labels.length);
    expect(group).toMatchObject({
      id: 'advisor',
      title: 'Labelled by the local model',
      verdict: 'review',
      source: 'model',
      items: review.length,
      allocated: review.reduce((sum, label) => sum + label.allocated, 0),
    });
    expect(sorted(group?.nodes ?? [])).toEqual(sorted(review.map((label) => label.node)));
    expect(group?.nodes[0]).toBe(review[0]?.node);
    expect(after.totals.review).toBe(before.totals.review + (group?.allocated ?? 0));
    expect(after.totals.safe).toBe(before.totals.safe);
    expect(after.totals.keep).toBe(before.totals.keep);
    expect(after.suggestions.filter((entry) => entry.id !== 'advisor')).toEqual(before.suggestions);
  });

  it('never lists the model as safe and ranks the group among the review ones by size', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await labelAll(api, timers, id);

    const { suggestions } = await api.getSuggestions(id);
    const review = suggestions.filter((entry) => entry.verdict === 'review').map((entry) => entry.allocated);

    expect(
      suggestions.filter((entry) => entry.source === 'model').every((entry) => entry.verdict === 'review'),
    ).toBe(true);
    expect(review).toEqual([...review].sort((a, b) => b - a));
  });

  it('forgets the group with the scan: a new scan has none', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await labelAll(api, timers, id);
    await api.dropScan(id);

    const next = await scanOf(api, '/');

    expect(advisorOf((await api.getSuggestions(next)).suggestions)).toBeUndefined();
    expect((await api.getAdvisorLabels(next)).labelled).toBe(0);
  });

  it('refuses to plan the advisor group without labels, and plans it as review, trash, last', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await expectCode(api.planCleanup(id, ['advisor']), 'invalid');
    await expectCode(api.planCleanup(id, ['trash', 'advisor']), 'invalid');

    const labels = await labelAll(api, timers, id);
    const review = labels.labels.filter((label) => label.verdict === 'review');
    const plan = await api.planCleanup(id, ['advisor', 'trash']);

    expect(plan.groups.map((group) => group.ruleId)).toEqual(['trash', 'advisor']);
    expect(plan.groups[1]).toMatchObject({
      ruleId: 'advisor',
      title: 'Labelled by the local model',
      verdict: 'review',
      action: 'trash',
      needsElevation: false,
      items: review.length,
      allocated: review.reduce((sum, label) => sum + label.allocated, 0),
    });
    expect(plan.groups[1]).not.toHaveProperty('command');
    expect(plan.groups[1]?.samplePaths.length).toBeLessThanOrEqual(20);
    expect(plan.groups[1]?.samplePaths.every((path) => review.some((label) => label.path === path))).toBe(
      true,
    );
    expect(plan.needsConfirmation).toBe(true);
    expect(plan.notFound).toEqual([]);
  });

  it('refuses the advisor group after a run that found nothing to label', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' }, '/mnt/backup');
    const id = await scan;

    expect((await labelAll(api, timers, id)).labelled).toBe(0);
    await expectCode(api.planCleanup(id, ['advisor']), 'invalid');
  });

  it('drops the labels of a scan when a cleanup outdates it, and with them the advisor group', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const other = await scanOf(api, '/');

    expect((await labelAll(api, timers, id)).labelled).toBeGreaterThan(0);
    expect((await labelAll(api, timers, other)).labelled).toBeGreaterThan(0);
    expect(advisorOf((await api.getSuggestions(id)).suggestions)).toBeDefined();

    const plan = await api.planCleanup(id, ['trash']);
    const events: CleanupEvent[] = [];

    await api.startCleanup(plan.planId, (event) => events.push(event));
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));

    const result = await api.getAdvisorLabels(id);

    expect(result).toMatchObject({ labelled: 0, labels: [] });
    expect(result.stopped).toBeUndefined();
    expect(result.modelMs).toBe(0);
    expect(advisorOf((await api.getSuggestions(id)).suggestions)).toBeUndefined();
    await expectCode(api.planCleanup(id, ['advisor']), 'invalid');
    expect((await api.getAdvisorLabels(other)).labelled).toBeGreaterThan(0);
  });

  it('stores nothing when a run ends after a cleanup outdated its scan', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const plan = await api.planCleanup(id, ['trash']);
    const events: CleanupEvent[] = [];

    await api.labelFolders(id);
    await api.startCleanup(plan.planId, (event) => events.push(event));
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));
    timers.advance(LABEL_MS * 100);

    expect((await api.advisorStatus()).labelling).toBeUndefined();
    expect(await api.getAdvisorLabels(id)).toMatchObject({ labelled: 0, labels: [] });
  });

  it('lists and plans the advisor group only while the advisor is switched on', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const labels = await labelAll(api, timers, id);
    const on = await api.getSuggestions(id);

    expect(advisorOf(on.suggestions)).toBeDefined();

    await api.configureAdvisor({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });

    const off = await api.getSuggestions(id);

    expect(advisorOf(off.suggestions)).toBeUndefined();
    expect(off.suggestions).toEqual(on.suggestions.filter((entry) => entry.id !== 'advisor'));
    expect(off.totals.review).toBe(on.totals.review - (advisorOf(on.suggestions)?.allocated ?? 0));
    await expectCode(api.planCleanup(id, ['advisor']), 'invalid');
    await expectCode(api.planCleanup(id, ['trash', 'advisor']), 'invalid');

    // Other groups can still be planned, and the labels are still the scan's.
    expect((await api.planCleanup(id, ['trash'])).groups.map((group) => group.ruleId)).toEqual(['trash']);
    expect(await api.getAdvisorLabels(id)).toMatchObject({ labelled: labels.labelled });

    await api.configureAdvisor({
      enabled: true,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });

    expect(advisorOf((await api.getSuggestions(id)).suggestions)).toBeDefined();
    expect((await api.planCleanup(id, ['advisor'])).groups[0]?.ruleId).toBe('advisor');
  });

  it('has no advisor group in a scan made while the advisor is switched off', async () => {
    const { api, scan } = setup({
      state: 'disk',
      options: { enabled: false, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true },
    });
    const id = await scan;

    expect(advisorOf((await api.getSuggestions(id)).suggestions)).toBeUndefined();
    await expectCode(api.planCleanup(id, ['advisor']), 'invalid');
  });

  it('trashes the labelled folders in a run that can be undone, and outdates the scan', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const labels = await labelAll(api, timers, id);
    const review = labels.labels.filter((label) => label.verdict === 'review');
    const plan = await api.planCleanup(id, ['advisor']);
    const events: CleanupEvent[] = [];
    const cleanup = await api.startCleanup(plan.planId, (event) => events.push(event));

    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));

    const done = events.filter((event) => event.type === 'targetDone');

    expect(done).toHaveLength(review.length);
    expect(done.every((event) => event.outcome.kind === 'trashed')).toBe(true);
    expect(events.at(-1)).toMatchObject({
      cleaned: review.length,
      freed: 0,
      canUndo: true,
      outdatedScan: id,
    });

    const undone = await api.undoCleanup(cleanup);

    expect(undone).toHaveLength(review.length);
    await expectCode(api.planCleanup(id, ['advisor']), 'invalid');
  });
});

function advisorGroup(result: { suggestions: SuggestionDto[] }): SuggestionDto | undefined {
  return result.suggestions.find((entry) => entry.id === 'advisor');
}

describe('askAdvisor: validation', () => {
  it('rejects an unknown scan or node as notFound and a file as invalid', async () => {
    const { api, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const project = await folder(api, id, '/home/demo/Projekty/platter');
    const page = await api.getChildren(id, project, 0, 50, 'sizeDesc');
    const file = page.items.find((item) => item.kind === 'file');

    await expectCode(api.askAdvisor(4242, project), 'notFound');
    await expectCode(api.askAdvisor(id, 99_999_999), 'notFound');
    expect(file).toBeDefined();
    await expectCode(api.askAdvisor(id, (file as { id: NodeId }).id), 'invalid');
  });

  it('rejects a scan that is still running as notFound', async () => {
    const timers = new FakeTimers();
    const api = createMockApi({
      durationMs: 1000,
      holdAt: 0.5,
      now: () => NOW,
      advisor: { timers, state: 'loaded' },
    });
    const running = await api.startScan({ root: '/', oneFileSystem: true }, () => {});

    await expectCode(api.askAdvisor(running, 0), 'notFound');

    await api.dropScan(running);
  });
});

describe('askAdvisor: the answer comes from the rules alone', () => {
  /** The size as the answer writes it, the folder's own, in the units asked for. */
  async function sizeOf(api: Api, scan: ScanId, path: string, units: SizeUnits): Promise<string> {
    const [match] = await api.findFolders(scan, path, 1);

    return formatSize(match?.allocated ?? 0, units);
  }

  it('answers for the scan root, a protected place and an unreadable folder from the guard', async () => {
    const { api, scan } = setup({ state: 'none' });
    const id = await scan;

    const cases: [string, string, string][] = [
      ['/', 'This is the folder that was scanned, so Platter never suggests cleaning it.', 'size'],
      ['/usr', 'This is a protected place, or inside one (/usr). Platter does not clean it.', 'size'],
      [
        '/home/demo/.var',
        'This is a protected place, or inside one (/home/demo/.var). Platter does not clean it.',
        'size',
      ],
      ['/root', 'Platter could not read this folder, so it cannot tell what is in it.', 'unknown'],
    ];

    for (const [path, why, size] of cases) {
      const answer = await api.askAdvisor(id, await folder(api, id, path));

      expect(answer, path).toMatchObject({
        path,
        verdict: 'keep',
        source: 'guard',
        reason: `Left alone by Platter. ${why}`,
      });

      if (size === 'unknown') {
        expect(answer.detail).toBe('Platter could not read it, so its size is unknown.');
      } else {
        expect(answer.detail).toMatch(/^[\d.]+ [KMGT]?B\./);
      }
    }
  });

  it('answers for a folder a rule covers, or lies in, with the rule words, its title, the size and the age', async () => {
    const { api, scan } = setup({ state: 'none' });
    const id = await scan;
    const { suggestions } = await api.getSuggestions(id);
    const rule = (ruleId: string) => suggestions.find((entry) => entry.id === ruleId) as SuggestionDto;

    const cache = await api.askAdvisor(id, await folder(api, id, '/home/demo/.cache'));

    expect(cache).toMatchObject({ verdict: 'review', source: 'rule', reason: rule('app-cache').reason });
    expect(cache.detail).toContain(
      `Rule: ${rule('app-cache').title}. ${await sizeOf(api, id, '/home/demo/.cache', 'GB')}. `,
    );
    expect(cache.detail).toMatch(
      /^Rule: .+\. [\d.]+ [KMGT]?B\. (?:None|Less than 1%|\d+%) .*older than a year\.$/,
    );

    // Inside the covered folder: the nearest rule above answers.
    const inside = await api.askAdvisor(id, await folder(api, id, '/home/demo/.cache/google-chrome'));

    expect(inside).toMatchObject({ verdict: 'review', source: 'rule', reason: rule('app-cache').reason });

    const kept = await api.askAdvisor(
      id,
      await folder(api, id, '/home/demo/.local/share/Steam/steamapps/common'),
    );

    expect(kept).toMatchObject({ verdict: 'keep', source: 'rule' });
    expect((await api.advisorStatus()).state).toBe('notDownloaded');
  });

  it('writes the size in the units asked for, and in GB when none is given', async () => {
    const { api, scan } = setup({ state: 'none' });
    const id = await scan;
    const node = await folder(api, id, '/home/demo/.cache');
    const decimal = await sizeOf(api, id, '/home/demo/.cache', 'GB');
    const binary = await sizeOf(api, id, '/home/demo/.cache', 'GiB');

    expect(binary).not.toBe(decimal);
    expect((await api.askAdvisor(id, node)).detail).toContain(` ${decimal}. `);
    expect((await api.askAdvisor(id, node, 'GB')).detail).toContain(` ${decimal}. `);
    expect((await api.askAdvisor(id, node, 'GiB')).detail).toContain(` ${binary}. `);
    expect((await api.askAdvisor(id, node, 'GiB')).detail).not.toContain(` ${decimal}. `);
  });

  it('says there is no rule for a folder nothing covers, and leaves the verdict to the user as review', async () => {
    const { api, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const answer = await api.askAdvisor(id, await folder(api, id, '/var/tmp'));

    expect(answer).toMatchObject({
      path: '/var/tmp',
      verdict: 'review',
      source: 'unruled',
      reason: 'Platter has no rule for it. Deleting it is your call.',
    });
    expect(answer.detail).toMatch(/^[\d.]+ [KMGT]?B\./);
    expect(answer.detail).not.toMatch(/model/i);
  });

  it('answers from a label made earlier in the scan with the label as the verdict and no word of the model', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const labels = await labelAll(api, timers, id);

    expect(new Set(labels.labels.map((entry) => entry.verdict))).toEqual(new Set(['review', 'keep']));

    for (const label of labels.labels) {
      const answer = await api.askAdvisor(id, label.node);
      const marked = label.verdict === 'keep' ? 'to keep' : 'for review';

      expect(answer, label.path).toMatchObject({
        path: label.path,
        verdict: label.verdict,
        source: 'model',
        reason: `Platter has no rule for it. Deleting it is your call. The local model marked it ${marked}.`,
      });
      expect(`${answer.reason} ${answer.detail}`).not.toContain(label.reason);
      expect(answer.detail).not.toMatch(/model|label/i);
    }
  });

  it('answers at once, setting no timer and starting no model job, whatever state the model is in', async () => {
    for (const advisor of [
      { state: 'none' },
      { state: 'downloading' },
      { state: 'disk' },
      { state: 'loaded' },
      {
        state: 'disk',
        options: { enabled: false, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true },
      },
    ] as const) {
      const { api, timers, events, scan } = setup(advisor);
      const id = await scan;
      const before = (await api.advisorStatus()).state;
      const timersBefore = timers.count;
      const node = await folder(api, id, '/var/tmp');

      expect(await api.askAdvisor(id, node)).toMatchObject({ source: 'unruled' });
      expect(timers.count).toBe(timersBefore);
      expect(events).toEqual([]);
      expect((await api.advisorStatus()).state).toBe(before);
    }
  });

  it('answers while folders are being labelled, and neither holds up nor is held up by the labelling', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const { candidates } = await api.getAdvisorLabels(id);

    await api.labelFolders(id);

    expect(await api.askAdvisor(id, await folder(api, id, '/var/tmp'))).toMatchObject({ source: 'unruled' });
    expect((await api.advisorStatus()).labelling).toBeDefined();

    timers.advance(LOAD_MS + LABEL_MS * (candidates + 1));

    expect((await api.getAdvisorLabels(id)).labelled).toBe(candidates);
  });

  it('answers while a free question is on its way, and does not stop it', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;
    const asking = api.askScan(id, 'what is in the sketchbook folder?', 'GB');

    expect(await api.askAdvisor(id, await folder(api, id, '/var/tmp'))).toMatchObject({ source: 'unruled' });
    await expectCode(api.askScan(id, 'what is in the other folder?', 'GB'), 'invalid');

    timers.advance(ASK_MS);

    await expect(asking).resolves.toMatchObject({ scan: id });
  });

  it('does not store its answer as a label', async () => {
    const { api, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await api.askAdvisor(id, await folder(api, id, '/var/tmp'));

    expect((await api.getAdvisorLabels(id)).labelled).toBe(0);
    expect(advisorGroup(await api.getSuggestions(id))).toBeUndefined();
  });

  it('does not keep the model from idling away', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' });
    const id = await scan;

    await api.configureAdvisor({ enabled: true, unloadAfter: 'oneMinute', logQuestions: true, useGpu: true });
    await api.askAdvisor(id, await folder(api, id, '/var/tmp'));

    timers.advance(60_000);

    expect((await api.advisorStatus()).state).toBe('onDisk');
  });
});

describe('findFolders', () => {
  it('puts an exact path first, then names starting with the text, then names containing it', async () => {
    const { api, scan } = setup();
    const id = await scan;
    const found = await api.findFolders(id, 'cache', 20);
    const names = found.map((match) => match.path.slice(match.path.lastIndexOf('/') + 1));
    const firstSubstring = names.findIndex((name) => !name.toLowerCase().startsWith('cache'));

    expect(found.length).toBeGreaterThan(2);
    expect(names.slice(0, firstSubstring).every((name) => name.toLowerCase().startsWith('cache'))).toBe(true);
    expect(names.slice(firstSubstring).every((name) => name.toLowerCase().includes('cache'))).toBe(true);
    expect(names).toContain('.cache');

    // Largest first within a group.
    const prefix = found.slice(0, firstSubstring).map((match) => match.allocated);
    const rest = found.slice(firstSubstring).map((match) => match.allocated);

    expect(prefix).toEqual([...prefix].sort((a, b) => b - a));
    expect(rest).toEqual([...rest].sort((a, b) => b - a));

    const exact = await api.findFolders(id, '/home/demo/.cache', 5);

    expect(exact[0]?.path).toBe('/home/demo/.cache');
    expect(exact.slice(1).every((match) => match.path !== '/home/demo/.cache')).toBe(true);
  });

  it('reads ~/ as the home folder and / as the scan root', async () => {
    const { api, scan } = setup();
    const id = await scan;

    expect((await api.findFolders(id, '~/Projekty', 3))[0]?.path).toBe('/home/demo/Projekty');
    expect((await api.findFolders(id, '~', 3))[0]?.path).toBe('/home/demo');
    expect((await api.findFolders(id, '/', 3))[0]).toMatchObject({ node: 0, path: '/' });
    expect((await api.findFolders(id, '/home/demo/Projekty/', 3))[0]?.path).toBe('/home/demo/Projekty');
    expect(await api.findFolders(id, '/home/demo/Projekty/nothing-here', 3)).toEqual([]);
    expect(await api.findFolders(id, '/home/demo/../demo', 3)).toEqual([]);
  });

  it('looks up a path under a scan root that is a subfolder', async () => {
    const { api, scan } = setup({ state: 'disk' }, '/home/demo');
    const id = await scan;

    expect((await api.findFolders(id, '~/Projekty', 3))[0]?.path).toBe('/home/demo/Projekty');
    expect((await api.findFolders(id, '/home/demo/Projekty', 3))[0]?.path).toBe('/home/demo/Projekty');
    expect(await api.findFolders(id, '/var/cache', 3)).toEqual([]);
  });

  it('lists folders only, ignoring case, with the size and node of each', async () => {
    const { api, scan } = setup();
    const id = await scan;
    const found = await api.findFolders(id, 'PROJEKTY', 20);

    expect(found[0]?.path).toBe('/home/demo/Projekty');
    expect(found[0]?.allocated).toBe((await api.getNode(id, found[0]?.node as NodeId)).allocated);
    expect(await api.findFolders(id, 'IMG_', 20)).toEqual([]);

    for (const match of found) {
      expect((await api.getNode(id, match.node)).kind).toBe('dir');
    }
  });

  it('clamps the limit to 1..20 and refuses a text of more than 1024 bytes', async () => {
    const { api, scan } = setup();
    const id = await scan;

    expect(await api.findFolders(id, 'a', 0)).toHaveLength(1);
    expect(await api.findFolders(id, 'a', -5)).toHaveLength(1);
    expect(await api.findFolders(id, 'a', 3)).toHaveLength(3);
    expect((await api.findFolders(id, 'a', 1000)).length).toBe(20);

    await expectCode(api.findFolders(id, 'a'.repeat(1025), 5), 'invalid');
    await expectCode(api.findFolders(id, 'ł'.repeat(513), 5), 'invalid');
    expect(await api.findFolders(id, 'a'.repeat(1024), 5)).toEqual([]);
  });

  it('matches the scan root only by its exact path', async () => {
    const { api, scan } = setup({ state: 'disk' }, '/home/demo');
    const id = await scan;

    expect((await api.findFolders(id, '/home/demo', 5))[0]).toMatchObject({ node: 0, path: '/home/demo' });
    expect((await api.findFolders(id, '~', 5))[0]).toMatchObject({ path: '/home/demo' });

    for (const text of ['demo', 'dem', 'm', 'o']) {
      const found = await api.findFolders(id, text, 20);

      expect(
        found.map((match) => match.node),
        text,
      ).not.toContain(0);
    }
  });

  it('matches the text as typed: spaces around it are not trimmed, only a blank text finds nothing', async () => {
    const { api, scan } = setup();
    const id = await scan;

    expect((await api.findFolders(id, 'cache', 5)).length).toBeGreaterThan(0);
    expect(await api.findFolders(id, ' cache', 5)).toEqual([]);
    expect(await api.findFolders(id, 'cache ', 5)).toEqual([]);
    // A path with spaces around it is not an exact path either.
    expect(await api.findFolders(id, ' /home/demo/.cache', 5)).toEqual([]);
    expect(await api.findFolders(id, '/home/demo/.cache ', 5)).toEqual([]);
  });

  it('finds nothing for a blank text, and rejects an unknown scan', async () => {
    const { api, scan } = setup();
    const id = await scan;

    expect(await api.findFolders(id, '', 5)).toEqual([]);
    expect(await api.findFolders(id, '   ', 5)).toEqual([]);
    await expectCode(api.findFolders(4242, 'a', 5), 'notFound');
  });
});

describe('findEntries', () => {
  const nameOf = (path: string): string => path.slice(path.lastIndexOf('/') + 1);

  it('lists files as well as folders, saying which each is', async () => {
    const { api, scan } = setup();
    const id = await scan;
    const found = await api.findEntries(id, 'IMG_11', 20);

    expect(found.length).toBeGreaterThan(2);
    expect(found.every((match) => match.kind === 'file')).toBe(true);

    for (const match of found) {
      const node = await api.getNode(id, match.node);

      expect(node.kind).toBe('file');
      expect(match.allocated).toBe(node.allocated);
    }

    const mixed = await api.findEntries(id, 'Pictures', 20);

    expect(mixed[0]).toMatchObject({ path: '/home/demo/Pictures', kind: 'dir' });
  });

  it('looks in the whole tree for a name, the exact name first, then names that start with it, then names that hold it', async () => {
    const { api, scan } = setup();
    const id = await scan;
    const found = await api.findEntries(id, 'Cache', 20);
    const names = found.map((match) => nameOf(match.path).toLowerCase());
    const tier = (name: string): number => (name === 'cache' ? 0 : name.startsWith('cache') ? 1 : 2);

    expect(found.length).toBeGreaterThan(2);
    expect(names.map(tier)).toEqual([...names.map(tier)].sort((a, b) => a - b));

    // Largest first within a tier.
    for (const wanted of [0, 1, 2]) {
      const sizes = found
        .filter((_, at) => tier(names[at] as string) === wanted)
        .map((match) => match.allocated);

      expect(sizes).toEqual([...sizes].sort((a, b) => b - a));
    }
  });

  it('lists the children of the folder before the last slash, whose name fits the rest', async () => {
    const { api, scan } = setup();
    const id = await scan;
    const all = await api.findEntries(id, '~/Projekty/', 20);
    const some = await api.findEntries(id, '~/Projekty/pl', 20);

    expect(all.length).toBeGreaterThan(2);
    expect(all.every((match) => match.path.startsWith('/home/demo/Projekty/'))).toBe(true);
    expect(all.every((match) => !match.path.slice('/home/demo/Projekty/'.length).includes('/'))).toBe(true);
    // An empty rest lists every child, largest first.
    expect(all.map((match) => match.allocated)).toEqual(
      [...all.map((match) => match.allocated)].sort((a, b) => b - a),
    );
    // Names that start with the rest come before names that only hold it.
    expect(some.map((match) => match.path).slice(0, 2)).toEqual([
      '/home/demo/Projekty/platter',
      '/home/demo/Projekty/rust-playground',
    ]);
    expect(some.every((match) => nameOf(match.path).toLowerCase().includes('pl'))).toBe(true);

    // A bare `~` is `~/`, and a path may be written in full.
    expect((await api.findEntries(id, '~', 3))[0]?.path).toBe('/home/demo/Projekty');
    expect((await api.findEntries(id, '/home/demo/Projekty/plat', 3)).map((match) => match.path)).toEqual([
      '/home/demo/Projekty/platter',
    ]);
  });

  it('lists the files inside a folder one level at a time', async () => {
    const { api, scan } = setup();
    const id = await scan;
    const [first] = await api.findEntries(id, '~/Pictures/2021/IMG_11', 1);

    expect(first?.kind).toBe('file');
    expect(first?.path.startsWith('/home/demo/Pictures/2021/IMG_11')).toBe(true);

    // The text before the last slash must be a folder: a file has no children.
    expect(await api.findEntries(id, `${first?.path}/x`, 5)).toEqual([]);
    expect(await api.findEntries(id, '~/Nothing/', 5)).toEqual([]);
    expect(await api.findEntries(id, '~/Projekty/../', 5)).toEqual([]);
  });

  it('reads a query that only starts with ~ as a name, not a path', async () => {
    const { api, scan } = setup();
    const id = await scan;

    expect(await api.findEntries(id, '~foo', 5)).toEqual([]);
    expect(await api.findEntries(id, 'Projekty', 5)).not.toEqual([]);
  });

  it('never lists a link, a mount point or the scan root', async () => {
    const { api, scan } = setup({ state: 'disk' }, '/home/demo');
    const id = await scan;

    for (const text of ['demo', 'o', '.', 'a', '~', '~/']) {
      for (const match of await api.findEntries(id, text, 20)) {
        const node = await api.getNode(id, match.node);

        expect(['dir', 'file'], `${text} ${match.path}`).toContain(node.kind);
        expect(match.node, text).not.toBe(0);
      }
    }

    const whole = setup();
    const full = await whole.scan;

    expect(await whole.api.findEntries(full, '/proc', 5)).toEqual([]);
    expect((await whole.api.findEntries(full, '/', 50)).map((match) => nameOf(match.path))).not.toContain(
      'proc',
    );
  });

  it('clamps the limit to 1..20 and refuses a text of more than 1024 bytes', async () => {
    const { api, scan } = setup();
    const id = await scan;

    expect(await api.findEntries(id, 'a', 0)).toHaveLength(1);
    expect(await api.findEntries(id, 'a', -5)).toHaveLength(1);
    expect(await api.findEntries(id, 'a', 3)).toHaveLength(3);
    expect((await api.findEntries(id, 'a', 1000)).length).toBe(20);

    await expectCode(api.findEntries(id, 'a'.repeat(1025), 5), 'invalid');
    await expectCode(api.findEntries(id, 'ł'.repeat(513), 5), 'invalid');
    expect(await api.findEntries(id, 'a'.repeat(1024), 5)).toEqual([]);
  });

  it('finds nothing for a blank text, and rejects an unknown scan', async () => {
    const { api, scan } = setup();
    const id = await scan;

    expect(await api.findEntries(id, '', 5)).toEqual([]);
    expect(await api.findEntries(id, '   ', 5)).toEqual([]);
    await expectCode(api.findEntries(4242, 'a', 5), 'notFound');
  });
});
