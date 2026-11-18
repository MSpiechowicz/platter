import { describe, expect, it, vi } from 'vitest';
import { formatCount, formatSize } from '../format';
import type { Api } from './client';
import { isApiError } from './client';
import { createMockApi, type MockOptions } from './mock';
import type { MockAdvisorOptions } from './mock-advisor';
import { FakeTimers } from './mock-advisor.testkit';
import { protection } from './mock-advisor';
import { nodeAt, sketchTree } from './mock-sketch';
import {
  answerQuestion,
  BUILD_FAMILY,
  CACHE_FAMILY,
  ground,
  interpret,
  MAX_LIMIT,
  OFFTOPIC_TEXT,
  OUTDATED_NOTE,
  percent,
  type ScanQuery,
  type ScanQuestionContext,
} from './mock-scan-question';
import { folderPathOf, RULE_IDS } from './mock-suggestions';
import type { CleanupEvent, ScanAnswerDto, ScanEvent, ScanId, ScanRowDto, SizeUnits } from './types';

const NOW = 1_750_000_000;
const LOAD_MS = 400;
const LABEL_MS = 300;
const ASK_MS = 800;
const HOME = '/home/demo';
const SWITCHED_OFF = {
  enabled: false,
  unloadAfter: 'fiveMinutes',
  logQuestions: true,
  useGpu: true,
} as const;
const BUSY = 'the model is busy, so wait for it to finish';

function setup(
  advisor: MockAdvisorOptions = { state: 'loaded' },
  extra: MockOptions = {},
): { api: Api; timers: FakeTimers; scan: Promise<ScanId> } {
  const timers = new FakeTimers();
  const api = createMockApi({ durationMs: 0, now: () => NOW, advisor: { timers, ...advisor }, ...extra });
  const scan = new Promise<ScanId>((resolve, reject) => {
    api
      .startScan({ root: '/', oneFileSystem: true }, (event: ScanEvent) => {
        if (event.type === 'finished') {
          resolve(event.scanId);
        }
      })
      .catch(reject);
  });

  return { api, timers, scan };
}

async function rejection(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }

  throw new Error('expected a rejection');
}

async function expectCode(promise: Promise<unknown>, code: string): Promise<string> {
  const error = await rejection(promise);

  expect(isApiError(error) && error.code).toBe(code);

  return isApiError(error) ? error.message : '';
}

/** Asks and plays the model's time out. A model that is not loaded yet is loaded first. */
async function ask(
  api: Api,
  timers: FakeTimers,
  scan: ScanId,
  question: string,
  units: SizeUnits = 'GB',
): Promise<ScanAnswerDto> {
  const answer = api.askScan(scan, question, units);

  timers.advance(LOAD_MS + ASK_MS);

  return answer;
}

/** Labels every candidate of the scan with the fake model, which is loaded. */
async function labelAll(api: Api, timers: FakeTimers, scan: ScanId): Promise<void> {
  const { candidates } = await api.getAdvisorLabels(scan);

  await api.labelFolders(scan);
  timers.advance(LABEL_MS * (candidates + 1));
}

/** A display path as `getPath` writes it, with the home folder as `~`. */
const shown = (path: string): string => (path === HOME ? '~' : path.replace(`${HOME}/`, '~/'));

/** What every row of every answer must be: a folder of the scan, under the scope, within the limit, safe only by rule. */
async function expectSoundRows(api: Api, scan: ScanId, answer: ScanAnswerDto): Promise<void> {
  const safe = new Set(
    (await api.getSuggestions(scan)).suggestions
      .filter((suggestion) => suggestion.verdict === 'safe')
      .flatMap((suggestion) => suggestion.nodes),
  );
  const scope = answer.understood.scope;

  expect(answer.rows.length).toBeLessThanOrEqual(answer.understood.limit);
  expect(new Set(answer.rows.map((row) => row.node)).size).toBe(answer.rows.length);

  for (const row of answer.rows) {
    const node = await api.getNode(scan, row.node);
    const path = await api.getPath(scan, row.node);

    expect(node.kind).toBe('dir');
    expect(row.allocated).toBe(node.allocated);
    expect(row.path).toBe(shown(path.path));

    if (scope) {
      expect(path.segments.map((segment) => segment.id)).toContain(scope.node);
    }

    if (row.verdict === 'safe') {
      expect(safe.has(row.node), `${row.path} is called safe`).toBe(true);
    }

    if (row.source === 'model') {
      expect(row.verdict).not.toBe('safe');
    }
  }
}

describe('percent', () => {
  it('rounds a share to whole per cent', () => {
    expect(percent(1, 4)).toBe(25);
    expect(percent(2, 3)).toBe(67);
  });

  it('is 0 for nothing and for a share too small to round up', () => {
    expect(percent(0, 100)).toBe(0);
    expect(percent(1, 1000)).toBe(0);
    expect(percent(5, 0)).toBe(0);
  });

  it('never reads 100 for a share under the whole', () => {
    expect(percent(996, 1000)).toBe(99);
    expect(percent(999_999, 1_000_000)).toBe(99);
  });

  it('reads 100 for the whole', () => {
    expect(percent(1000, 1000)).toBe(100);
  });
});

describe('askScan: errors, in order', () => {
  it('rejects a blank question before it looks at the scan', async () => {
    const { api } = setup({ state: 'none' });

    expect(await expectCode(api.askScan(4242, '', 'GB'), 'invalid')).toBe('the question is empty');
    expect(await expectCode(api.askScan(4242, ' \n\t ', 'GB'), 'invalid')).toBe('the question is empty');
  });

  it('takes 500 characters and rejects 501, counted after trimming and as characters, not bytes', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;

    const tooLong = 'the question is longer than 500 characters';

    expect(await expectCode(api.askScan(4242, 'a'.repeat(501), 'GB'), 'invalid')).toBe(tooLong);
    expect(await expectCode(api.askScan(id, 'a'.repeat(501), 'GB'), 'invalid')).toBe(tooLong);
    expect(await expectCode(api.askScan(id, '😀'.repeat(501), 'GB'), 'invalid')).toBe(tooLong);

    for (const question of ['a'.repeat(500), `  ${'a'.repeat(500)}\n`, '😀'.repeat(500)]) {
      expect((await ask(api, timers, id, question)).understood.intent).toBe('offtopic');
    }
  });

  it('rejects an unknown or a running scan as notFound, before any model or rules check', async () => {
    const timers = new FakeTimers();
    const api = createMockApi({
      durationMs: 1000,
      holdAt: 0.5,
      now: () => NOW,
      advisor: {
        timers,
        state: 'none',
        options: { enabled: false, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true },
      },
      userRules: { broken: true },
    });

    await expectCode(api.askScan(4242, 'what takes space?', 'GB'), 'notFound');

    const running = await api.startScan({ root: '/', oneFileSystem: true }, () => {});

    await expectCode(api.askScan(running, 'what takes space?', 'GB'), 'notFound');

    await api.dropScan(running);
  });

  it('rejects a broken rules file before the model is looked at', async () => {
    const { api, scan } = setup({ state: 'none' }, { userRules: { broken: true } });
    const message = await expectCode(api.askScan(await scan, 'what takes space?', 'GB'), 'invalid');

    expect(message).toContain('rules file');
  });

  it('rejects while folders are being labelled and while another question runs', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;

    await api.labelFolders(id);
    expect(await expectCode(api.askScan(id, 'what takes space?', 'GB'), 'invalid')).toContain('labelled');

    await api.cancelLabelling();

    const first = api.askScan(id, 'what takes space?', 'GB');

    expect(await expectCode(api.askScan(id, 'overview', 'GB'), 'invalid')).toBe(BUSY);

    timers.advance(ASK_MS);
    expect((await first).understood.intent).toBe('largest');
  });

  it('rejects when the model is switched off, not on disk or downloading, and changes nothing', async () => {
    const off = setup({
      state: 'disk',
      options: { enabled: false, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true },
    });

    expect(await expectCode(off.api.askScan(await off.scan, 'what takes space?', 'GB'), 'invalid')).toBe(
      'the local model is switched off in the settings',
    );
    expect((await off.api.advisorStatus()).state).toBe('onDisk');

    const missing = setup({ state: 'none' });

    expect(
      await expectCode(missing.api.askScan(await missing.scan, 'what takes space?', 'GB'), 'invalid'),
    ).toBe('the model is not on disk yet');
    expect((await missing.api.advisorStatus()).state).toBe('notDownloaded');

    const downloading = setup({ state: 'downloading' });

    expect(
      await expectCode(downloading.api.askScan(await downloading.scan, 'what takes space?', 'GB'), 'invalid'),
    ).toBe(BUSY);
    expect((await downloading.api.advisorStatus()).state).toBe('downloading');
  });

  it('refuses a busy model before it looks at the options, and a model that is switched off before one not on disk', async () => {
    const busy = setup({ state: 'downloading', options: SWITCHED_OFF });
    const off = setup({ state: 'none', options: SWITCHED_OFF });
    const loading = setup({ state: 'disk' });
    const loadingId = await loading.scan;

    expect(await expectCode(busy.api.askScan(await busy.scan, 'what takes space?', 'GB'), 'invalid')).toBe(
      BUSY,
    );
    expect(await expectCode(off.api.askScan(await off.scan, 'what takes space?', 'GB'), 'invalid')).toBe(
      'the local model is switched off in the settings',
    );

    void loading.api.loadModel();
    expect((await loading.api.advisorStatus()).state).toBe('loading');
    expect(await expectCode(loading.api.askScan(loadingId, 'what takes space?', 'GB'), 'invalid')).toBe(BUSY);
  });

  it('refuses a second question while one runs, whichever scan it is about, and takes the next once it is answered', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const first = api.askScan(id, 'what takes space?', 'GB');

    expect(await expectCode(api.askScan(id, 'what takes space?', 'GB'), 'invalid')).toBe(BUSY);

    timers.advance(ASK_MS);
    await first;

    expect((await ask(api, timers, id, 'overview')).understood.intent).toBe('general');
  });

  it('refuses labelling while a question runs, but answers a folder question at once, even for a folder no rule covers', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const [pip] = await api.findFolders(id, '~/.cache/pip', 1);
    const node = pip?.node ?? -1;
    const asking = api.askScan(id, 'what takes space?', 'GB');

    expect(await expectCode(api.labelFolders(id), 'invalid')).toBe(BUSY);
    // A folder question uses no model: the rules answer while the model reads the other question.
    expect((await api.askAdvisor(id, node)).source).toBe('rule');
    expect((await api.askAdvisor(id, 0)).source).toBe('guard');

    timers.advance(ASK_MS);
    await asking;

    expect((await api.askAdvisor(id, node)).source).toBe('rule');
    await expect(api.labelFolders(id)).resolves.toBeUndefined();
  });

  it('is cancelled by switching the model off, and fails with notFound when the scan is dropped meanwhile', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const cancelled = api.askScan(id, 'what takes space?', 'GB');

    await api.configureAdvisor({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });
    expect(await expectCode(cancelled, 'invalid')).toBe('the model was stopped before it read the question');

    await api.configureAdvisor({
      enabled: true,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });

    const dropped = api.askScan(id, 'what takes space?', 'GB');

    await api.dropScan(id);
    timers.advance(LOAD_MS + ASK_MS);

    await expectCode(dropped, 'notFound');
  });
});

describe('askScan: the model', () => {
  it('loads a model that is on disk first and leaves it loaded', async () => {
    const { api, timers, scan } = setup({ state: 'disk' });
    const id = await scan;
    const answer = api.askScan(id, 'what takes the most space?', 'GB');

    expect((await api.advisorStatus()).state).toBe('loading');

    timers.advance(LOAD_MS + ASK_MS);

    expect((await answer).understood.intent).toBe('largest');
    expect((await api.advisorStatus()).state).toBe('loaded');
  });

  it('answers an off-topic question with the fixed text, no rows and no figures', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'write me a poem');

    expect(answer).toMatchObject({
      understood: { intent: 'offtopic', kind: 'any' },
      answer: OFFTOPIC_TEXT,
      rows: [],
      more: 0,
    });
    expect(answer.understood.scope).toBeUndefined();
    expect(answer.answer).toBe(
      'I can only answer questions about this scan: what takes space, what can be cleaned, why a folder is big, and what the model labelled.',
    );
  });

  it('never stores a question or its answer', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const before = await api.getAdvisorLabels(id);

    await ask(api, timers, id, 'what can I clean?');

    expect(await api.getAdvisorLabels(id)).toEqual(before);
  });
});

describe('askScan: one question per intent', () => {
  const cases: {
    question: string;
    intent: ScanAnswerDto['understood']['intent'];
    kind?: ScanAnswerDto['understood']['kind'];
    scope?: string;
    limit?: number;
  }[] = [
    { question: 'what takes the most space?', intent: 'largest' },
    { question: 'biggest folders in ~/Projekty', intent: 'largest', scope: '~/Projekty' },
    { question: 'what are my biggest caches?', intent: 'largest', kind: 'cache' },
    { question: 'which build output is the biggest?', intent: 'largest', kind: 'build' },
    { question: 'how much do my videos take?', intent: 'largest', kind: 'media' },
    { question: 'what can I clean in ~/Projekty?', intent: 'cleanable', scope: '~/Projekty' },
    { question: 'delete everything in /', intent: 'cleanable', scope: '/' },
    { question: 'what caches can I clean?', intent: 'cleanable', kind: 'cache' },
    { question: 'what can I clean in ~/.cache/pip', intent: 'cleanable', scope: '~/.cache/pip' },
    { question: 'what can I clean in ~/Videos?', intent: 'cleanable', scope: '~/Videos' },
    { question: 'why is ~/.local so big?', intent: 'explain', scope: '~/.local' },
    { question: 'why is Downloads so big', intent: 'explain', scope: '~/Downloads' },
    { question: 'where is node_modules?', intent: 'find' },
    { question: 'give me an overview', intent: 'general' },
    { question: 'what is kept in ~/Videos', intent: 'general' },
    { question: 'top 3 biggest folders in ~/Projekty', intent: 'largest', scope: '~/Projekty', limit: 3 },
  ];

  it.each(cases)('$question', async ({ question, intent, kind, scope, limit }) => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const answer = await ask(api, timers, id, question);

    expect(answer.scan).toBe(id);
    expect(answer.understood.intent).toBe(intent);
    expect(answer.understood.kind).toBe(kind ?? 'any');
    expect(answer.understood.limit).toBe(limit ?? 5);
    expect(answer.understood.scope?.path).toBe(scope);
    expect(answer.answer).not.toBe('');
    expect(answer.notes.length).toBeLessThanOrEqual(6);

    await expectSoundRows(api, id, answer);
  });

  it('gives rows under the scope for the questions that name a folder', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const [projects] = await api.findFolders(id, '~/Projekty', 1);

    for (const question of ['biggest folders in ~/Projekty', 'what can I clean in ~/Projekty?']) {
      const answer = await ask(api, timers, id, question);

      expect(answer.understood.scope).toEqual({ node: projects?.node, path: '~/Projekty' });
      expect(answer.rows.length).toBeGreaterThan(0);
      expect(answer.rows.every((row) => row.path.startsWith('~/Projekty/'))).toBe(true);
    }
  });

  it('names the biggest folders with their sizes in the text, written in the asked units', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const decimal = await ask(api, timers, id, 'biggest folders in ~/Projekty', 'GB');
    const binary = await ask(api, timers, id, 'biggest folders in ~/Projekty', 'GiB');
    const first = decimal.rows[0];

    expect(first).toBeDefined();
    expect(decimal.answer).toContain(
      `\u201C${first?.path}\u201D (${formatSize(first?.allocated ?? 0, 'GB')})`,
    );
    expect(binary.answer).toContain(
      `\u201C${first?.path}\u201D (${formatSize(first?.allocated ?? 0, 'GiB')})`,
    );
    expect(binary.answer).not.toBe(decimal.answer);
    expect(binary.rows).toEqual(decimal.rows);
  });

  it('keeps the rows within the limit and counts the rest in more', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const two = await ask(api, timers, id, 'top 2 biggest folders in ~/Projekty');
    const all = await ask(api, timers, id, 'top 10 biggest folders in ~/Projekty');
    const huge = await ask(api, timers, id, 'top 99 biggest folders');

    expect(two.rows).toHaveLength(2);
    expect(two.more).toBe(all.rows.length + all.more - 2);
    expect(two.rows).toEqual(all.rows.slice(0, 2));
    expect(huge.understood.limit).toBe(MAX_LIMIT);
    expect(huge.rows.length).toBeLessThanOrEqual(MAX_LIMIT);
  });

  it('lists caches and builds from their rule families, with the verdict of the rule', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const suggestions = await api.getSuggestions(id);
    const titles = (family: ReadonlySet<string>): string[] =>
      suggestions.suggestions.filter((entry) => family.has(entry.id)).map((entry) => entry.title);

    const caches = await ask(api, timers, id, 'what are my biggest caches?');
    const builds = await ask(api, timers, id, 'what are my biggest builds?');

    expect(caches.rows.length).toBeGreaterThan(0);
    expect(builds.rows.length).toBeGreaterThan(0);
    expect(caches.rows.every((row) => titles(CACHE_FAMILY).includes(row.note ?? ''))).toBe(true);
    expect(builds.rows.every((row) => titles(BUILD_FAMILY).includes(row.note ?? ''))).toBe(true);
    expect(caches.rows.map((row) => row.allocated)).toEqual(
      caches.rows.map((row) => row.allocated).sort((a, b) => b - a),
    );
    expect(caches.rows.every((row) => row.source === 'rule' && row.verdict !== undefined)).toBe(true);
  });

  it('lists the folders that can be cleaned, safe ones first, and nothing that is kept', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const answer = await ask(api, timers, id, 'what can I clean? top 10');
    const verdicts = answer.rows.map((row) => row.verdict);

    expect(verdicts).not.toContain('keep');
    expect(verdicts).toEqual([...verdicts].sort((a, b) => (a === 'safe' ? -1 : b === 'safe' ? 1 : 0)));
    expect(answer.answer).toMatch(/^Safe to clean: /);
  });

  it('says what keeps a scope that a keep rule covers, and shows a scope a rule covers as one row', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const kept = await ask(api, timers, id, 'what can I clean in ~/Videos?');
    const [videos] = await api.findFolders(id, '~/Videos', 1);

    // The kept folder is a row of its own, with the verdict the guard gives it, and the answer says no about it.
    expect(kept.rows).toEqual([
      expect.objectContaining({ node: videos?.node, path: '~/Videos', verdict: 'keep', source: 'guard' }),
    ]);
    expect(kept.notes).toEqual([]);
    expect(kept.answer).toMatch(/^No: \u201C~\/Videos\u201D \([\d.]+ [KMGT]?B\) is left alone by Platter\. /);

    const covered = await ask(api, timers, id, 'what can I clean in ~/.cache/pip');
    const [pip] = await api.findFolders(id, '~/.cache/pip', 1);

    expect(covered.rows).toHaveLength(1);
    expect(covered.rows[0]).toMatchObject({ node: pip?.node, path: '~/.cache/pip', verdict: 'review' });
  });

  it('answers a question for a folder the scan has no rows for with a plain text and no rows', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(
      api,
      timers,
      await scan,
      'where is the folder called nothing-like-this-anywhere?',
    );

    expect(answer.understood.intent).toBe('find');
    expect(answer.rows).toEqual([]);
    expect(answer.answer).toBe(
      'No file or folder called \u201Cnothing-like-this-anywhere\u201D is in this scan.',
    );
  });

  it('finds folders by name, largest first, each with the verdict of the rule that covers it', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const answer = await ask(api, timers, id, 'where is node_modules?');

    expect(answer.understood.scopeText).toBe('node_modules');
    expect(answer.rows.length).toBeGreaterThan(1);
    expect(answer.rows.every((row) => row.path.endsWith('/node_modules'))).toBe(true);
    expect(answer.rows.map((row) => row.allocated)).toEqual(
      answer.rows.map((row) => row.allocated).sort((a, b) => b - a),
    );
    expect(answer.rows.every((row) => row.verdict !== undefined && row.source === 'rule')).toBe(true);
  });

  it('gives a folder that no rule covers no verdict', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'biggest folders in ~/Projekty');
    const projects = answer.rows.filter((row) => row.path === '~/Projekty/platter');

    expect(projects).toHaveLength(1);
    expect(projects[0]?.verdict).toBeUndefined();
    expect(projects[0]?.source).toBeUndefined();
  });
});

describe("askScan: the model's labels", () => {
  it('points to "Label folders" when nothing is labelled', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'what did the model label in Downloads?');

    expect(answer.understood).toMatchObject({ intent: 'labelled', scope: { path: '~/Downloads' } });
    expect(answer.rows).toEqual([]);
    expect(answer.notes.join(' ')).toContain('Label folders');
  });

  it('lists the labels under the scope with the model as source and no note (its reason is never shown)', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;

    await labelAll(api, timers, id);

    const labels = await api.getAdvisorLabels(id);
    const answer = await ask(api, timers, id, 'what did the model label? top 10');

    expect(labels.labelled).toBeGreaterThan(0);
    expect(answer.rows.length).toBeGreaterThan(0);

    for (const row of answer.rows) {
      const label = labels.labels.find((entry) => entry.node === row.node);

      expect(row).toMatchObject({ verdict: label?.verdict, source: 'model' });
      expect(row).not.toHaveProperty('note');
      expect(row.verdict).not.toBe('safe');
    }

    await expectSoundRows(api, id, answer);

    const scoped = await ask(api, timers, id, 'what did the model label in ~/Downloads?');

    expect(
      scoped.rows.every((row) => row.path === '~/Downloads' || row.path.startsWith('~/Downloads/')),
    ).toBe(true);
  });

  it('writes the labels by verdict with the model totals, and says when a scope has none', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;

    await labelAll(api, timers, id);

    const answer = await ask(api, timers, id, 'what did the model label? top 10');
    const empty = await ask(api, timers, id, 'what did the model label in ~/.cache/pip');

    const total = '\\d+ to (?:review|keep) \\([\\d.]+ [KMGT]?B\\)';

    // How many and for what, then which way the model marked each folder, then how many are not listed.
    expect(answer.answer).toMatch(
      new RegExp(`^The model labelled \\d+ folders: ${total}(?: and ${total})?\\. `),
    );
    expect(answer.answer).toMatch(
      /The model marked \u201C[^\u201D]+\u201D \([\d.]+ [KMGT]?B\) (?:to keep|for review)\./,
    );
    // What the model said about a folder is not in the answer.
    for (const label of (await api.getAdvisorLabels(id)).labels) {
      expect(answer.answer).not.toContain(label.reason.replace(/[.!? ]+$/, ''));
    }
    expect(answer.answer).toMatch(/ \d+ more labelled folders are not listed\.$/);
    expect(empty.notes).toEqual([]);
    expect(empty.answer).toBe('The model has not labelled any folder in “~/.cache/pip”.');
  });

  it("lists the model's review labels among the folders that can be cleaned, never as safe", async () => {
    const { api, timers, scan } = setup();
    const id = await scan;

    await labelAll(api, timers, id);

    const advisor = (await api.getSuggestions(id)).suggestions.find((entry) => entry.id === 'advisor');
    const [folder] = advisor?.nodes ?? [];

    expect(folder).toBeDefined();

    const path = shown((await api.getPath(id, folder as number)).path);
    const answer = await ask(api, timers, id, `what can I clean in ${path}`);

    expect(answer.rows).toContainEqual(
      expect.objectContaining({
        node: folder,
        verdict: 'review',
        source: 'model',
        note: 'Labelled by the local model',
      }),
    );

    await expectSoundRows(api, id, answer);
  });
});

describe('askScan: scopes the scan does not hold', () => {
  it.each([
    ['biggest folders in Atlantis', 'Atlantis'],
    ['what can I clean in ~/no-such-folder?', '~/no-such-folder'],
    ['why is /nowhere/at/all so big', '/nowhere/at/all'],
    ['what did the model label in Atlantis?', 'Atlantis'],
  ])('%s', async (question, text) => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, question);

    expect(answer.rows).toEqual([]);
    expect(answer.more).toBe(0);
    expect(answer.understood.scope).toBeUndefined();
    expect(answer.understood.scopeText).toBe(text);
    expect(answer.notes).toContain(`No file or folder called “${text}” is in this scan`);
    // The user's own words are a context path: the answer sets them in quotes, as text, with no folder behind them,
    // and the answer is that note.
    // What the model has not labelled is said first, and the note that the folder is missing after it.
    const missing = `No file or folder called “${text}” is in this scan.`;

    expect(answer.answer).toBe(
      question.includes('label') ? `The model has not labelled any folder in this scan. ${missing}` : missing,
    );
    expect(answer.segments.some((segment) => segment.kind === 'path' && segment.path === text)).toBe(false);
  });

  it('resolves a bare name to a folder of the home folder first, then by search', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const downloads = await ask(api, timers, id, 'what takes space in downloads?');
    const nested = await ask(api, timers, id, 'what takes space in recordings?');

    expect(downloads.understood.scope?.path).toBe('~/Downloads');
    expect(nested.understood.scope?.path).toBe('~/Videos/recordings');
  });
});

describe('askScan: the direct first sentence', () => {
  const quote = (path: string): string => `“${path}”`;

  /** A row as the answer names it: its path in quotes, then its size. */
  const named = (row: ScanRowDto, units: SizeUnits = 'GB'): string =>
    `${quote(row.path)} (${formatSize(row.allocated, units)})`;

  /** The chips of an answer by path, with their verdict. */
  const chips = (answer: ScanAnswerDto): Record<string, string | undefined> =>
    Object.fromEntries(
      answer.segments.flatMap((segment) =>
        segment.kind === 'path' ? [[segment.path, segment.verdict]] : [],
      ),
    );

  it('names the biggest folders first for "which folder does take the biggest amount of data?"', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const answer = await ask(api, timers, id, 'which folder does take the biggest amount of data?');
    const [first, ...rest] = answer.rows.map((row) => named(row));
    const everything = await api.getNode(id, 0);

    // "amount of data" is no folder: the whole scan is asked about.
    expect(answer.understood).toMatchObject({ intent: 'largest', kind: 'any' });
    expect(answer.understood.scope).toBeUndefined();
    expect(answer.understood.scopeText).toBeUndefined();
    expect(answer.notes).toEqual([]);
    expect(rest).toHaveLength(4);
    expect(answer.answer).toBe(
      `The biggest folder in this scan is ${first}, then ${rest.slice(0, 3).join(', ')} and ${rest[3]}. ` +
        `This scan holds ${formatSize(everything.allocated, 'GB')} in ${formatCount(everything.files)} files.`,
    );
  });

  it.each([
    'can i safely delete the docker images',
    'is docker data safe to remove?',
    'can I clean the docker containers',
  ])(
    "says that the docker data needs a look, and that the chip is the docker-data rule's review: %s",
    async (question) => {
      const { api, timers, scan } = setup();
      const id = await scan;
      const answer = await ask(api, timers, id, question);
      const [docker] = await api.findFolders(id, '/var/lib/docker', 1);
      const rule = (await api.getSuggestions(id)).suggestions.find((entry) => entry.id === 'docker-data');

      expect(docker?.path).toBe('/var/lib/docker');
      expect(answer.understood).toMatchObject({
        intent: 'cleanable',
        kind: 'any',
        scope: { node: docker?.node, path: '/var/lib/docker' },
      });
      expect(answer.rows).toEqual([
        expect.objectContaining({
          node: docker?.node,
          verdict: 'review',
          source: 'rule',
          note: 'Docker data',
        }),
      ]);
      // The rule's title in quotes, then the rule's own words on the folder.
      expect(answer.answer).toBe(
        `No, not without a look: ${named(answer.rows[0] as ScanRowDto)} needs your review (“Docker data”). ${rule?.reason}`,
      );
      expect(answer.answer).toContain('Images, containers and volumes.');
      expect(chips(answer)).toEqual({ '/var/lib/docker': 'review' });
    },
  );

  it('says yes, with what the rule is, for a folder that is safe to clean', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'is it safe to clean the trash?');
    const [row] = answer.rows;

    expect(row).toMatchObject({ path: '~/.local/share/Trash', verdict: 'safe' });
    expect(answer.answer).toBe(`Yes: ${named(row as ScanRowDto)} is safe to clean (“Trash”).`);
    expect(chips(answer)).toEqual({ '~/.local/share/Trash': 'safe' });
  });

  it('reads "Steam shader cache" as one folder, not as the caches and not as everything of Steam', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'is it safe to remove the Steam shader cache');
    const [row] = answer.rows;

    expect(answer.understood).toMatchObject({ intent: 'cleanable', kind: 'any' });
    expect(answer.understood.scope?.path).toBe('~/.local/share/Steam/steamapps/shadercache');
    expect(answer.answer).toBe(`Yes: ${named(row as ScanRowDto)} is safe to clean (“Steam shader cache”).`);
    expect(chips(answer)).toEqual({ '~/.local/share/Steam/steamapps/shadercache': 'safe' });
  });

  it('says no for a folder that is kept, and gives it the verdict keep', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;

    // The guard leaves the personal folders alone, and says why in its own words.
    const documents = await ask(api, timers, id, 'can I clean ~/Documents');
    const [kept] = documents.rows;

    expect(kept).toMatchObject({ path: '~/Documents', verdict: 'keep', source: 'guard' });
    expect(documents.answer).toBe(
      `No: ${named(kept as ScanRowDto)} is left alone by Platter. This is a protected place, or inside one (/home/demo/Documents). Platter does not clean it.`,
    );
    expect(chips(documents)).toEqual({ '~/Documents': 'keep' });

    // A folder only a rule keeps: kept, with the rule's title and its own words.
    const games = await ask(api, timers, id, 'can I clean ~/.local/share/Steam/steamapps/common');
    const [library] = games.rows;
    const rule = (await api.getSuggestions(id)).suggestions.find((entry) => entry.id === 'steam-library');

    expect(library).toMatchObject({ verdict: 'keep', source: 'rule', note: 'Steam library' });
    expect(games.answer).toBe(
      `No: ${named(library as ScanRowDto)} is kept (“Steam library”). ${rule?.reason}`,
    );
    expect(chips(games)).toEqual({ '~/.local/share/Steam/steamapps/common': 'keep' });
  });

  it("says partly when a tool's folders are safe and kept, and puts the verdict of each on its chip", async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'can i delete the steam files');
    const safe = answer.rows.find((row) => row.verdict === 'safe') as ScanRowDto;
    const kept = answer.rows.find((row) => row.verdict === 'keep') as ScanRowDto;

    expect(answer.understood.scope).toBeUndefined();
    expect(answer.answer).toBe(
      `Partly: ${named(safe)} is safe to clean (“Steam shader cache”) and ${named(kept)} is kept (“Steam library”).`,
    );
    expect(Object.values(chips(answer)).sort()).toEqual(['keep', 'safe']);
  });

  it('says yes and not without a look about the folders of one rule, each with its own verdict', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'can I delete node_modules?');
    const verdicts = answer.rows.map((row) => row.verdict);
    const safe = answer.rows.filter((row) => row.verdict === 'safe');
    const review = answer.rows.filter((row) => row.verdict === 'review');

    expect(new Set(verdicts)).toEqual(new Set(['safe', 'review']));
    expect(answer.rows.every((row) => row.path.endsWith('/node_modules'))).toBe(true);
    // More than one folder is no one scope.
    expect(answer.understood.scope).toBeUndefined();
    expect(answer.understood.scopeText).toBeUndefined();
    expect(answer.answer).toBe(
      `Partly: ${safe.map((row) => named(row)).join(' and ')} are safe to clean and ${review.map((row) => named(row)).join(' and ')} needs your review (“node_modules (recent)”).`,
    );
  });

  it('says no folder for a tool the scan does not reach, and where the tool keeps its data', async () => {
    // A scan of a folder that holds no Docker data: the answer names the tool as the question did, and where it lives.
    const small = setup({ state: 'loaded' });
    const smallId = await new Promise<ScanId>((resolve) => {
      void small.api.startScan({ root: '~/Projekty', oneFileSystem: true }, (event) => {
        if (event.type === 'finished') {
          resolve(event.scanId);
        }
      });
    });
    const elsewhere = await ask(small.api, small.timers, smallId, 'can i delete the docker images');
    const hint =
      '“Docker data” lives in /var/lib/docker, so scan a folder that holds it. “Docker Desktop virtual machine” lives in ~/.docker/desktop/vms, so scan your home folder';

    expect(elsewhere.rows).toEqual([]);
    // The text of the scope is the question's; the answer calls it by the word that named the rule.
    expect(elsewhere.understood).toMatchObject({ intent: 'cleanable', scopeText: 'docker' });
    expect(elsewhere.notes).toEqual([`No file or folder called “docker” is in this scan (${hint})`]);
    expect(elsewhere.answer).toBe(`No file or folder called “docker” is in this scan (${hint}).`);
  });
});

describe('askScan: the text, as the backend template writes it', () => {
  const quote = (path: string): string => `“${path}”`;

  it('sets every path and the scope in quotes, at most five rows, and the sizes in the asked units', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const answer = await ask(api, timers, id, 'biggest folders in ~/Projekty', 'GiB');
    const [scopeNode] = await api.findFolders(id, '~/Projekty', 1);
    const scoped = await api.getNode(id, scopeNode?.node ?? -1);
    const [first, ...rest] = answer.rows.map(
      (row) => `${quote(row.path)} (${formatSize(row.allocated, 'GiB')})`,
    );

    expect(answer.rows).toHaveLength(5);
    expect(answer.more).toBeGreaterThan(0);
    // About one folder, what it holds comes first.
    expect(answer.answer).toBe(
      `${quote('~/Projekty')} holds ${formatSize(scoped.allocated, 'GiB')} in ${formatCount(scoped.files)} files. ` +
        `The biggest folder in ${quote('~/Projekty')} is ${first}, then ${rest.slice(0, 3).join(', ')} and ${rest[3]}.`,
    );
  });

  it('lifts a folder of the largest to the folder a rule matched above it', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'biggest folders in ~/Projekty');

    expect(answer.rows.map((row) => row.path)).toContain('~/Projekty/rust-playground/target');
    expect(answer.rows.some((row) => row.path.startsWith('~/Projekty/rust-playground/target/'))).toBe(false);
  });

  it('opens a largest answer with the biggest folder, then says what the scan holds, without quoting the whole scan', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'what takes the most space?');

    expect(answer.answer).toMatch(
      /^The biggest folder in this scan is “[^”]+” \([\d.]+ [KMGT]?B\), then “[^”]+” \([\d.]+ [KMGT]?B\)[^.]*\. This scan holds [\d.]+ [KMGT]?B in [\d,]+ files\./,
    );
  });

  it('starts an overview with the biggest folders, then what the scan holds', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'give me an overview');

    expect(answer.answer).toMatch(
      /^The biggest folders are “[^”]+” \([\d.]+ [KMGT]?B\), .*\. This scan holds [\d.]+ [KMGT]?B in [\d,]+ files\. /,
    );
  });

  it('writes the cleanable rows by verdict, with the totals as items', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const answer = await ask(api, timers, id, 'what can I clean? top 10');

    expect(answer.answer).toMatch(
      /^Safe to clean: [\d.]+ [KMGT]?B in \d+ items, (?:namely|the largest being) “[^”]+” \([\d.]+ [KMGT]?B\)/,
    );
    expect(answer.answer).toMatch(/ For you to review: [\d.]+ [KMGT]?B in \d+ items, the largest being “/);
    expect(answer.answer).not.toContain('in total');
    expect(answer.answer).not.toContain('Kept: ');
    // Matched files count in the totals and are not listed; the answer says so once.
    expect(answer.answer.match(/matching files are counted in the totals but not listed/g)).toHaveLength(1);
  });

  it('names a scope in the heading of the first list only', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'what can I clean in ~/Projekty?');

    expect(answer.answer).toMatch(
      /^Safe to clean in “~\/Projekty”: [\d.]+ [KMGT]?B in 4 items, the largest being /,
    );
    expect(answer.answer).toMatch(/ For you to review: [\d.]+ [KMGT]?B in 2 items, namely /);
  });

  it('writes the overview totals as the verdicts of the suggestions, without "in total"', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const { totals } = await api.getSuggestions(id);
    const answer = await ask(api, timers, id, 'give me an overview');

    expect(answer.answer).toContain(` Safe to clean: ${formatSize(totals.safe, 'GB')}.`);
    expect(answer.answer).toContain(` For you to review: ${formatSize(totals.review, 'GB')}.`);
    expect(answer.answer).toContain(` Kept: ${formatSize(totals.keep, 'GB')}.`);
    expect(answer.answer).not.toContain('in total');
  });

  it('writes the family total, with the item count, after the rows', async () => {
    const { api, timers, scan } = setup();
    const caches = await ask(api, timers, await scan, 'what are my biggest caches?');
    const builds = await ask(api, timers, await scan, 'what are my biggest builds?');

    expect(caches.answer).toMatch(/^The biggest cache in this scan is /);
    expect(caches.answer).toMatch(/ All caches: [\d.]+ [KMGT]?B in \d+ items\.$/);
    expect(builds.answer).toMatch(/^The biggest build output or dependency folder in this scan is /);
    expect(builds.answer).toMatch(
      / All build output and dependency folders: [\d.]+ [KMGT]?B in \d+ items\.$/,
    );
  });

  it('names the media categories as the backend does, with one total for each', async () => {
    const { api, timers, scan } = setup();
    const all = await ask(api, timers, await scan, 'what media takes the most space?');

    expect(all.answer).toMatch(
      /^The biggest media folder in this scan is .* Video: [\d.]+ [KMGT]?B in \d+ items?\. Images: [\d.]+ [KMGT]?B in \d+ items?\. Audio: [\d.]+ [KMGT]?B in \d+ items?\./,
    );
    // Categories follow the backend's table, and the home folder no longer counts as one video folder as a whole. So the
    // walk goes down to Videos, Downloads, Pictures, Music and the trash: three categories, with Music the audio one.
    expect(new Set(all.rows.map((row) => row.note))).toEqual(new Set(['Video', 'Images', 'Audio']));

    // One category named: its total comes first, and only its folders are listed.
    const videos = await ask(api, timers, await scan, 'how much do my videos take?');

    expect(videos.answer).toMatch(
      /^Video folders in this scan take [\d.]+ [KMGT]?B in total \(\d+ folders?\)\. The biggest video folder in this scan is “[^”]+” \([\d.]+ [KMGT]?B\)(?:, then “[^”]+” \([\d.]+ [KMGT]?B\))*\.$/,
    );
    // The home folder used to be the one video folder, so the answer named a single one. Now `~/Videos` and
    // `~/Downloads` are both video folders, and the answer lists them after the first.
    expect(videos.answer).toContain(' (2 folders)');
    expect(new Set(videos.rows.map((row) => row.note))).toEqual(new Set(['Video']));
  });

  it('explains with the shares of the folder, the age, what the rules matched and the biggest parts', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'why is ~/Projekty so big?');

    expect(answer.answer).toMatch(/^“~\/Projekty” holds [\d.]+ [KMGT]?B in [\d,]+ files\. /);
    expect(answer.answer).toMatch(/ The largest parts by type are [A-Z][A-Za-z ]+ \(\d+%, [\d.]+ [KMGT]?B\)/);
    expect(answer.answer).toMatch(/ \d+% of it \([\d.]+ [KMGT]?B\) is older than a year\. /);
    expect(answer.answer).toMatch(/ Platter's rules match “[^”]+” \([\d.]+ [KMGT]?B\)/);
    expect(answer.answer).toMatch(/ inside it\. The biggest parts are “~\/Projekty\//);

    const covered = await ask(api, timers, await scan, 'why is ~/.cache/pip so big?');

    // Which rule covers the folder is not said in the answer.
    expect(covered.notes.join(' ')).not.toMatch(/covered by the rule|labelled this folder/);
    expect(covered.answer).not.toMatch(/covered by the rule|labelled this folder/);
  });

  it('answers a find with the folders it found, and with a note when it found none', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const found = await ask(api, timers, id, 'where is node_modules?');
    const none = await ask(api, timers, id, 'where is the folder called nothing-xyz?');
    const [first, ...rest] = found.rows.map(
      (row) => `${quote(row.path)} (${formatSize(row.allocated, 'GB')})`,
    );

    expect(found.answer).toBe(
      `Matches for “node_modules”: ${first}, ${rest.slice(0, -1).join(', ')} and ${rest.at(-1)}.`,
    );
    expect(found.notes).toEqual([]);
    expect(none.answer).toBe('No file or folder called “nothing-xyz” is in this scan.');
    expect(none.notes).toEqual(['No file or folder called “nothing-xyz” is in this scan']);
    expect(none.understood.scopeText).toBe('nothing-xyz');
  });

  it('counts the matches of a find that are not rows', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'where is the target folder? top 2');

    expect(answer.rows).toHaveLength(2);
    expect(answer.more).toBeGreaterThan(0);
    expect(answer.answer).toMatch(
      new RegExp(
        `^Matches for “target”: “[^”]+” \\([\\d.]+ [KMGT]?B\\), “[^”]+” \\([\\d.]+ [KMGT]?B\\) and ${answer.more} more\\.$`,
      ),
    );
  });

  it('says that no name was given when the name searched is not backed by the question', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'find a folder');

    expect(answer.understood).toMatchObject({ intent: 'find' });
    expect(answer.understood.scopeText).toBeUndefined();
    expect(answer.rows).toEqual([]);
    expect(answer.notes).toEqual(['No folder name was given to look for.']);
    expect(answer.answer).toBe('No folder name was given to look for.');
  });

  it('says what it found nothing of, for the scan or for a scope', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;

    expect((await ask(api, timers, id, 'what did the model label?')).answer).toBe(
      'The model has not labelled any folder in this scan. Use “Label folders” in the Advisor view.',
    );
    // A folder with nothing to clean says what it holds and where Platter stands on it.
    expect((await ask(api, timers, id, 'what can I clean in /usr')).answer).toMatch(
      /^“\/usr” holds [\d.]+ [KMGT]?B in \d+ files\. Platter has no rule for it\. Deleting it is your call\.$/,
    );
  });
});

describe('askScan: what the backend leaves out of the scope', () => {
  it.each([
    ['what is kept in ~/Videos', 'general'],
    ['an overview of ~/Projekty', 'general'],
  ])('%s: the overview is of the whole scan, whatever folder it names', async (question, intent) => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, question);

    expect(answer.understood.intent).toBe(intent);
    expect(answer.understood.scope).toBeUndefined();
    expect(answer.understood.scopeText).toBeUndefined();
    expect(answer.answer).toMatch(/^The biggest folders are /);
    expect(answer.answer).toContain(' This scan holds ');
  });

  it('drops a scope the question does not hold as a word, and answers for the whole scan', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'biggest folders in x');

    expect(answer.understood.scope).toBeUndefined();
    expect(answer.understood.scopeText).toBeUndefined();
    expect(answer.notes).toEqual([]);
    expect(answer.answer).toMatch(/^The biggest folder in this scan is /);
    expect(answer.answer).toContain(' This scan holds ');
  });
});

describe('askScan: the guard comes first', () => {
  /** `/var` is a protected place, and a rule of the user cannot make it a target. */
  const varIsSafe = { rules: [{ verdict: 'safe', folder: '/var' }] } as const;

  it('lists no folder the guard leaves alone and counts none, even when a rule of the user says safe', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' }, { userRules: varIsSafe });
    const id = await scan;
    const answer = await ask(api, timers, id, 'what can I clean? top 10');
    const { suggestions, totals } = await api.getSuggestions(id);
    const [varRule] = suggestions.filter((suggestion) => suggestion.source === 'user');

    expect(varRule?.allocated).toBeGreaterThan(0);
    expect(answer.rows.every((row) => row.path !== '/var' && row.source !== 'guard')).toBe(true);
    expect(answer.answer).not.toContain('\u201C/var\u201D');
    // The suggestions count /var as safe; the answer does not.
    expect(answer.answer).not.toContain(`Safe to clean: ${formatSize(totals.safe, 'GB')} in`);
    expect(answer.answer).toContain('Safe to clean: ');
  });

  it('says why the scope is not cleaned when the guard leaves it alone', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' }, { userRules: varIsSafe });
    const answer = await ask(api, timers, await scan, 'what can I clean in /var');

    const [row] = answer.rows;

    // The folder the question named is its own row, kept by the guard; the guard's words are said once.
    expect(row).toMatchObject({ path: '/var', verdict: 'keep', source: 'guard' });
    expect(answer.notes).toEqual([
      'This is a protected place, or inside one (/var). Platter does not clean it.',
    ]);
    expect(answer.answer).toBe(
      `No: \u201C/var\u201D (${formatSize(row?.allocated ?? 0, 'GB')}) is left alone by Platter. This is a protected place, or inside one (\u201C/var\u201D). Platter does not clean it.`,
    );
  });

  it('gives a folder the guard leaves alone the verdict keep in any list, whatever a rule says', async () => {
    const { api, timers, scan } = setup({ state: 'loaded' }, { userRules: varIsSafe });
    const id = await scan;
    const answer = await ask(api, timers, id, 'what takes the most space? top 10');

    expect(answer.rows.find((row) => row.path === '/var')).toMatchObject({
      verdict: 'keep',
      source: 'guard',
      note: 'Left alone by the safety guard, not decided by the model.',
    });
  });
});

describe('askScan: the answer in segments', () => {
  /** The segments written as the contract says: each text as it is, each path between “ ”, a quote in a name replaced. */
  const written = (answer: ScanAnswerDto): string =>
    answer.segments
      .map((segment) =>
        segment.kind === 'text'
          ? segment.text
          : `“${segment.path.replaceAll('“', '‟').replaceAll('”', '〃').replaceAll('"', '〃')}”`,
      )
      .join('');

  const paths = (answer: ScanAnswerDto): Extract<ScanAnswerDto['segments'][number], { kind: 'path' }>[] =>
    answer.segments.flatMap((segment) => (segment.kind === 'path' ? [segment] : []));

  it.each([
    'biggest folders in ~/Projekty',
    'what can I clean? top 10',
    'what are my biggest caches?',
    'give me an overview',
    'why is ~/Projekty so big?',
    'where is node_modules?',
    'what can I clean in ~/Videos?',
    'what did the model label?',
    'biggest folders in Atlantis',
    'write me a poem',
  ])('writes %s as segments that give the answer back', async (question) => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, question);

    expect(answer.segments.length).toBeGreaterThan(0);
    expect(written(answer)).toBe(answer.answer);
    expect(answer.segments.every((segment) => segment.kind === 'path' || segment.text !== '')).toBe(true);
  });

  it('binds every path of the rows to its folder with the verdict and source of its row, in order', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'what can I clean? top 10');
    const named = paths(answer).filter((segment) => answer.rows.some((row) => row.node === segment.node));

    expect(named.length).toBeGreaterThan(1);

    for (const segment of named) {
      const row = answer.rows.find((candidate) => candidate.node === segment.node);

      expect(segment.path).toBe(row?.path);
      expect(segment.verdict).toBe(row?.verdict);
      expect(segment.source).toBe(row?.source);
    }

    // "Safe to clean: <paths>. For you to review: <paths>."; the first path is a safe row, the last a review row.
    expect(paths(answer)[0]?.verdict).toBe('safe');
    expect(paths(answer).at(-1)?.verdict).toBe('review');
  });

  it('binds the scope to its folder, and a path in a note to the folder it names', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const scoped = await ask(api, timers, id, 'biggest folders in ~/Projekty');

    expect(paths(scoped).find((segment) => segment.path === '~/Projekty')).toEqual({
      kind: 'path',
      node: scoped.understood.scope?.node,
      path: '~/Projekty',
    });

    const kept = await ask(api, timers, id, 'what can I clean in ~/Videos?');

    // The scope is the row with its path: one chip, with the verdict of that row.
    expect(kept.segments).toEqual([
      { kind: 'text', text: 'No: ' },
      { kind: 'path', node: kept.understood.scope?.node, path: '~/Videos', verdict: 'keep', source: 'guard' },
      { kind: 'text', text: expect.stringMatching(/^ \([\d.]+ [KMGT]?B\) is left alone by Platter\. /) },
    ]);
  });

  it('leaves verdict and source out for a folder nobody has an opinion on', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'biggest folders in ~/Projekty');
    const plain = paths(answer).find((segment) => segment.path === '~/Projekty/platter');

    expect(plain).toBeDefined();
    expect(plain).not.toHaveProperty('verdict');
    expect(plain).not.toHaveProperty('source');
  });

  it('keeps the whole off-topic text as one text segment', async () => {
    const { api, timers, scan } = setup();
    const answer = await ask(api, timers, await scan, 'write me a poem');

    expect(answer.segments).toEqual([{ kind: 'text', text: OFFTOPIC_TEXT }]);
  });
});

describe('askScan: a folder with quotes in its name and a path of 200 characters', () => {
  const LONG = 'x'.repeat(190);
  const tree = sketchTree({
    files: {
      '/srv/say “hi” ”again” "now"/data.bin': 3000,
      [`/srv/${LONG}/data.bin`]: 2000,
      '/srv/plain/data.bin': 1000,
    },
  });
  const context: ScanQuestionContext = {
    scan: 1,
    tree,
    user: [],
    labels: [],
    outdated: false,
    units: 'GB',
    lookup: {
      exact: (path) => (path === '/srv' ? nodeAt(tree, '/srv') : null),
      entry: () => null,
      search: () => [],
      searchEntries: () => [],
    },
    guard: () => null,
    summary: () => {
      throw new Error('not asked');
    },
  };
  const answer = answerQuestion(context, 'biggest folders in /srv');

  it('shows a quote of a name as ‟ or 〃 in the answer, and keeps the name as it is in its segment', () => {
    const hostile = answer.rows.find((row) => row.path.startsWith('/srv/say'));
    const segment = answer.segments.find(
      (candidate) => candidate.kind === 'path' && candidate.node === hostile?.node,
    );

    expect(hostile?.path).toBe('/srv/say “hi” ”again” "now"');
    expect(segment).toMatchObject({ path: '/srv/say “hi” ”again” "now"' });
    expect(answer.answer).toContain('“/srv/say ‟hi〃 〃again〃 〃now〃”');
    expect(answer.answer.match(/“/g)).toHaveLength(answer.segments.filter((s) => s.kind === 'path').length);
  });

  it('cuts a path over 160 characters with … in the middle, keeping its end, in the segment and the answer only', () => {
    const row = answer.rows.find((candidate) => candidate.path.endsWith(LONG));
    const segment = answer.segments.find(
      (candidate) => candidate.kind === 'path' && candidate.node === row?.node,
    );

    expect(row?.path).toBe(`/srv/${LONG}`);
    expect(Array.from(row?.path ?? '')).toHaveLength(195);
    expect(segment?.kind === 'path' ? Array.from(segment.path) : []).toHaveLength(160);
    expect(segment).toMatchObject({ path: `/srv/${'x'.repeat(75)}…${'x'.repeat(79)}` });
    expect(answer.answer).toContain(`“/srv/${'x'.repeat(75)}…${'x'.repeat(79)}”`);
    expect(answer.answer).not.toContain(LONG);
  });

  it('still writes the segments as the answer', () => {
    const text = answer.segments
      .map((segment) =>
        segment.kind === 'text'
          ? segment.text
          : `“${segment.path.replaceAll('“', '‟').replaceAll('”', '〃').replaceAll('"', '〃')}”`,
      )
      .join('');

    expect(text).toBe(answer.answer);
  });
});

describe('askScan: an outdated scan', () => {
  it('adds the note to every intent once a cleanup ran on the scan', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const questions = [
      'what takes the most space?',
      'what can I clean?',
      'what did the model label?',
      'why is ~/.local so big?',
      'where is node_modules?',
      'overview',
      'write me a poem',
    ];

    for (const question of questions) {
      expect((await ask(api, timers, id, question)).notes).not.toContain(OUTDATED_NOTE);
    }

    const plan = await api.planCleanup(id, ['trash']);
    const events: CleanupEvent[] = [];

    await api.startCleanup(plan.planId, (event) => events.push(event));
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));

    for (const question of questions) {
      const answer = await ask(api, timers, id, question);

      expect(answer.notes, question).toContain(OUTDATED_NOTE);
      expect(answer.notes.length).toBeLessThanOrEqual(6);
    }

    // The answer says it once, at the end; the notes keep it first.
    const answer = await ask(api, timers, id, 'what takes the most space?');

    expect(answer.answer.endsWith(` ${OUTDATED_NOTE}`)).toBe(true);
    expect(answer.answer.split(OUTDATED_NOTE)).toHaveLength(2);
  });

  it('puts the note first, before the note about a scope or a name the scan does not hold', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const plan = await api.planCleanup(id, ['trash']);
    const events: CleanupEvent[] = [];

    await api.startCleanup(plan.planId, (event) => events.push(event));
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe('finished'));

    const atlantis = await ask(api, timers, id, 'biggest folders in Atlantis');
    const nothing = await ask(api, timers, id, 'where is the folder called nothing-xyz?');

    expect(atlantis.notes).toEqual([OUTDATED_NOTE, 'No file or folder called “Atlantis” is in this scan']);
    expect(atlantis.answer).toBe(
      `No file or folder called \u201CAtlantis\u201D is in this scan. ${OUTDATED_NOTE}`,
    );
    expect(nothing.notes).toEqual([OUTDATED_NOTE, 'No file or folder called “nothing-xyz” is in this scan']);
    expect(nothing.answer).toBe(
      `No file or folder called \u201Cnothing-xyz\u201D is in this scan. ${OUTDATED_NOTE}`,
    );
  });
});

describe("askScan: the backend's natural questions, on an owner's disk", () => {
  const MIB = 2 ** 20;
  const GIB = 2 ** 30;
  const DAY = 86_400;
  const HOME = '/home/u';

  type Docker = 'readable' | 'unreadable' | 'absent';

  /**
   * The disk of the backend's `owner_disk` test fixture: a home with a Rust project (an old build, and a recent one), a
   * node project (an old one and a recent one), caches, Steam, a video, a small `~/.docker` config folder and Docker's
   * data in `/var/lib/docker` (unreadable, readable and big, or not there), so the answers can be read against the
   * backend's own strings.
   */
  function ownerDisk(docker: Docker, extra: Partial<ScanQuestionContext> = {}): ScanQuestionContext {
    const tree = sketchTree({
      files: {
        '/home/u/Projekty/app/Cargo.toml': 1,
        '/home/u/Projekty/app/src/main.rs': 2 * MIB,
        '/home/u/Projekty/app/target/debug/x': 400 * MIB,
        '/home/u/Projekty/site/package.json': 1,
        '/home/u/Projekty/site/package-lock.json': 1,
        '/home/u/Projekty/site/node_modules/m/x': 200 * MIB,
        '/home/u/Projekty/web/package.json': 1,
        '/home/u/Projekty/web/package-lock.json': 1,
        '/home/u/Projekty/web/node_modules/m/x': 50 * MIB,
        '/home/u/Projekty/live/Cargo.toml': 1,
        '/home/u/Projekty/live/target/x': 90 * MIB,
        '/home/u/.cache/huggingface/m.bin': GIB,
        '/home/u/.cache/pip/w': 50 * MIB,
        '/home/u/.docker/config.json': 1024,
        '/home/u/.local/share/Steam/steamapps/shadercache/s': 300 * MIB,
        '/home/u/.local/share/Steam/steamapps/common/g/data': 4 * GIB,
        '/home/u/Downloads/big.iso': 700 * MIB,
        '/home/u/Downloads/misc/a': 30 * MIB,
        '/home/u/Videos/film.mkv': 3 * GIB,
        '/home/u/Videos/clips/c.mp4': 100 * MIB,
        '/home/u/Backups/phone/img': 500 * MIB,
        '/var/cache/pacman/pkg/a.zst': 600 * MIB,
        ...(docker === 'readable' ? { '/var/lib/docker/overlay2/l/layer': 8 * GIB } : {}),
      },
      unreadable: docker === 'unreadable' ? ['/var/lib/docker'] : [],
      // The old projects have not been touched for months; the others were touched the day before the scan.
      mtimes: {
        '/home/u/Projekty/app/target': NOW - 90 * DAY,
        '/home/u/Projekty/site/node_modules': NOW - 200 * DAY,
      },
    });
    const absolute = (path: string): string => (path.startsWith('~') ? `${HOME}${path.slice(1)}` : path);

    return {
      scan: 1,
      tree,
      user: [],
      labels: [],
      outdated: false,
      units: 'GB',
      lookup: {
        exact: (path) => {
          try {
            return nodeAt(tree, absolute(path));
          } catch {
            return null;
          }
        },
        search: (text, limit) =>
          tree.nodes
            .filter((node) => node.kind === 'dir' && node.name === text)
            .sort((a, b) => b.allocated - a.allocated)
            .slice(0, limit),
        entry: () => null,
        searchEntries: () => [],
      },
      guard: (node) => {
        if (node.parent < 0) {
          return 'This is the folder that was scanned, so Platter never suggests cleaning it.';
        }

        if (node.unreadable) {
          return 'Platter could not read this folder, so it cannot tell what is in it.';
        }

        return protection(folderPathOf(tree, node), HOME) === null
          ? null
          : 'This is a protected place, or inside one. Platter does not clean it.';
      },
      summary: () => {
        throw new Error('not asked');
      },
      ...extra,
    };
  }

  /** The paths that are chips in an answer, with the verdict each shows. */
  const chips = (answer: ScanAnswerDto): [string, string | undefined][] =>
    answer.segments.flatMap((segment) => (segment.kind === 'path' ? [[segment.path, segment.verdict]] : []));

  it.each([1, 5])(
    'names the biggest folder first for "which folder does take the biggest amount of data?" (top %i)',
    (limit) => {
      const answer = answerQuestion(
        ownerDisk('readable'),
        `which folder does take the biggest amount of data? top ${limit}`,
      );

      expect(answer.rows).toHaveLength(Math.min(limit, 8));
      expect(answer.rows[0]?.path).toBe('/var/lib/docker');
      expect(answer.answer.startsWith('The biggest folder in this scan is “/var/lib/docker” (8.6 GB)')).toBe(
        true,
      );
      expect(answer.answer.includes(', then ')).toBe(limit > 1);
      expect(chips(answer)[0]).toEqual(['/var/lib/docker', 'review']);
    },
  );

  it('names the biggest folders for "what\'s eating my disk"', () => {
    const answer = answerQuestion(ownerDisk('readable'), "what's eating my disk");

    expect(answer.understood.intent).toBe('largest');
    expect(answer.answer.startsWith('The biggest folder in this scan is “/var/lib/docker” (8.6 GB), ')).toBe(
      true,
    );
    expect(answer.answer).toContain(' holds ');
    expect(chips(answer)).toHaveLength(answer.rows.length);
  });

  it.each([
    'can i safely delete the docker images',
    'can I delete docker?',
    'is it safe to remove the docker containers',
  ])('is about the docker data and not the config folder: %s', (question) => {
    const answer = answerQuestion(ownerDisk('readable'), question);

    expect(answer.rows.map((row) => row.path)).toEqual(['/var/lib/docker']);
    expect(answer.rows[0]).toMatchObject({ verdict: 'review', source: 'rule', note: 'Docker data' });
    expect(answer.answer).toBe(
      'No, not without a look: “/var/lib/docker” (8.6 GB) needs your review (“Docker data”). Images, containers and volumes. Volumes can hold data that exists nowhere else, so look before removing anything.',
    );
    expect(answer.answer).not.toContain('~/.docker');
    expect(chips(answer)).toEqual([['/var/lib/docker', 'review']]);
  });

  it('says that docker data the scan could not read is left alone and that its size is unknown', () => {
    const answer = answerQuestion(ownerDisk('unreadable'), 'can i safely delete the docker images');

    expect(answer.rows.map((row) => row.path)).toEqual(['/var/lib/docker']);
    expect(answer.answer).toBe(
      'No: “/var/lib/docker” is left alone by Platter. Platter could not read it, so its size is unknown.',
    );
    expect(chips(answer)).toEqual([['/var/lib/docker', 'keep']]);
  });

  it('says where docker keeps its data when the scan does not hold it, and calls docker "docker"', () => {
    const answer = answerQuestion(ownerDisk('absent'), 'can i safely delete the docker images');

    expect(answer.rows).toEqual([]);
    expect(answer.understood.scopeText).toBe('docker');
    expect(answer.answer).toBe(
      'No file or folder called “docker” is in this scan (“Docker data” lives in /var/lib/docker, so scan a folder that holds it. “Docker Desktop virtual machine” lives in ~/.docker/desktop/vms, so scan your home folder).',
    );
    expect(chips(answer)).toEqual([]);
  });

  it('says yes to the old node_modules and not without a look to the recent one, in one answer', () => {
    const answer = answerQuestion(ownerDisk('absent'), 'can I delete node_modules?');

    expect(answer.rows.map((row) => row.path)).toEqual([
      '~/Projekty/site/node_modules',
      '~/Projekty/web/node_modules',
    ]);
    expect(answer.answer).toBe(
      'Partly: “~/Projekty/site/node_modules” (210 MB) is safe to clean (“node_modules”) and “~/Projekty/web/node_modules” (52 MB) needs your review (“node_modules (recent)”).',
    );
    expect(chips(answer)).toEqual([
      ['~/Projekty/site/node_modules', 'safe'],
      ['~/Projekty/web/node_modules', 'review'],
    ]);
    expect(answer.understood.scope).toBeUndefined();
  });

  it.each([
    'is it safe to remove the Steam shader cache',
    'is the steam shader cache safe to delete?',
    'can i wipe the steam shader cache?',
  ])('is a yes about the Steam shader cache folder: %s', (question) => {
    const answer = answerQuestion(ownerDisk('absent'), question);

    expect(answer.rows.map((row) => row.path)).toEqual(['~/.local/share/Steam/steamapps/shadercache']);
    expect(answer.answer).toBe(
      'Yes: “~/.local/share/Steam/steamapps/shadercache” (315 MB) is safe to clean (“Steam shader cache”).',
    );
    expect(chips(answer)).toEqual([['~/.local/share/Steam/steamapps/shadercache', 'safe']]);
    expect(answer.understood.scope?.path).toBe('~/.local/share/Steam/steamapps/shadercache');
  });

  it('lists what is safe and what needs a look for "co mogę usunąć?", docker for review and never safe', () => {
    const answer = answerQuestion(ownerDisk('readable'), 'co mogę usunąć?');
    const safe = answer.rows.filter((row) => row.verdict === 'safe');

    expect(answer.understood).toMatchObject({ intent: 'cleanable', kind: 'any' });
    expect(
      answer.answer.startsWith('Safe to clean: 944 MB in 3 items, namely “~/Projekty/app/target” (419 MB)'),
    ).toBe(true);
    expect(answer.answer).toContain(' For you to review: ');
    expect(answer.answer).toContain('“/var/lib/docker” (8.6 GB)');
    expect(safe).toHaveLength(3);

    for (const [path, verdict] of chips(answer)) {
      expect(answer.rows.find((row) => row.path === path)?.verdict, path).toBe(verdict);
    }
  });

  it('answers a kept scope with its own row and a no', () => {
    const answer = answerQuestion(ownerDisk('absent'), 'can I clean ~/Videos');
    const [row] = answer.rows;

    expect(row).toMatchObject({ path: '~/Videos', verdict: 'keep' });
    expect(answer.answer).toBe(
      `No: “~/Videos” (${formatSize(row?.allocated ?? 0, 'GB')}) is left alone by Platter. This is a protected place, or inside one. Platter does not clean it.`,
    );
    expect(chips(answer)).toEqual([['~/Videos', 'keep']]);
  });

  it('finds the folders of a name, and prefers the place a rule knows to every folder with the name', () => {
    const found = answerQuestion(ownerDisk('readable'), 'where is node_modules?');

    expect(found.answer).toBe(
      'Matches for “node_modules”: “~/Projekty/site/node_modules” (210 MB) and “~/Projekty/web/node_modules” (52 MB).',
    );

    const docker = answerQuestion(ownerDisk('readable'), 'where is docker');

    expect(docker.rows.map((row) => row.path)).toEqual(['/var/lib/docker']);
    expect(docker.answer).toBe('Matches for “docker”: “/var/lib/docker” (8.6 GB).');

    const absent = answerQuestion(ownerDisk('absent'), 'where is docker');

    expect(absent.rows).toEqual([]);
    expect(
      absent.answer.startsWith(
        'No file or folder called “docker” is in this scan (“Docker data” lives in /var/lib/docker,',
      ),
    ).toBe(true);
  });

  it('is told the folders of a rule as the scope of a largest question', () => {
    const answer = answerQuestion(ownerDisk('absent'), 'what is the biggest node_modules?');

    expect(answer.answer).toBe(
      'node_modules: 262 MB in 2 items. The biggest folder among the “node_modules” folders is “~/Projekty/site/node_modules” (210 MB), then “~/Projekty/web/node_modules” (52 MB).',
    );
    expect(answer.understood.scope).toBeUndefined();
    expect(answer.understood.scopeText).toBeUndefined();
  });

  it('calls an off-topic question what it is, with no rows and one text', () => {
    const answer = answerQuestion(ownerDisk('readable'), 'write me a poem');

    expect(answer.understood.intent).toBe('offtopic');
    expect(answer.rows).toEqual([]);
    expect(answer.segments).toEqual([{ kind: 'text', text: OFFTOPIC_TEXT }]);
  });

  it('says an outdated scan once, last', () => {
    const answer = answerQuestion(ownerDisk('readable', { outdated: true }), "what's eating my disk");

    expect(answer.notes).toEqual([OUTDATED_NOTE]);
    expect(answer.answer.endsWith(` ${OUTDATED_NOTE}`)).toBe(true);
    expect(answer.answer.split(OUTDATED_NOTE)).toHaveLength(2);
  });
});

describe('askScan: questions the words settle, and questions asked back', () => {
  const STALE = 'that choice is no longer available, so ask again';
  const ASKED_BACK = 'can I delete the src folder?';

  it('answers a question the words settle at once, with no wait and no model job', async () => {
    const { api, scan } = setup();
    const id = await scan;

    // No timer is advanced: the answer is there, and the model was neither loaded nor held.
    const answer = await api.askScan(id, 'can i safely delete the docker images', 'GB');

    expect(answer.understood.intent).toBe('cleanable');
    expect(answer.rows.map((row) => row.path)).toEqual(['/var/lib/docker']);
    expect((await api.advisorStatus()).state).toBe('loaded');
    expect(await api.askScan(id, 'where is node_modules?', 'GB')).toMatchObject({
      understood: { intent: 'find' },
    });
  });

  it('does not load a model that is on disk for a question the words settle, and takes it for one they leave open', async () => {
    const { api, timers, scan } = setup({ state: 'disk' });
    const id = await scan;

    await api.askScan(id, 'can i safely delete the docker images', 'GB');

    expect((await api.advisorStatus()).state).toBe('onDisk');

    const open = api.askScan(id, 'what takes the most space?', 'GB');

    expect((await api.advisorStatus()).state).toBe('loading');

    timers.advance(LOAD_MS + ASK_MS);

    expect((await open).understood.intent).toBe('largest');
  });

  it('still refuses a question the words settle when the model is switched off, as it refuses any other', async () => {
    const { api, scan } = setup({ state: 'disk', options: SWITCHED_OFF });

    expect(
      await expectCode(api.askScan(await scan, 'can i safely delete the docker images', 'GB'), 'invalid'),
    ).toBe('the local model is switched off in the settings');
  });

  it('asks back for a name that fits several folders, with one choice each and no rows', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const answer = await ask(api, timers, id, ASKED_BACK);

    expect(answer.clarify?.prompt).toBe('I found more than one folder called “src”. Which one do you mean?');
    expect(answer.answer).toBe(answer.clarify?.prompt);
    expect(answer.clarify?.choices.length).toBeGreaterThan(1);
    expect(answer.clarify?.choices.map((choice) => choice.id)).toEqual(
      answer.clarify?.choices.map((_, index) => index),
    );
    expect(answer.rows).toEqual([]);
    expect(answer.notes).toEqual([]);
    expect(answer.more).toBe(0);

    for (const choice of answer.clarify?.choices ?? []) {
      expect(choice.path).toBe(choice.label);
      expect(choice.path).toMatch(/^~\/Projekty\/[^/]+\/src$/);
      expect((await api.getPath(id, choice.node ?? -1)).path).toBe(choice.path?.replace('~', HOME));
    }
  });

  it('answers about the folder of a pick, from the same question and nothing else', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const asked = await ask(api, timers, id, ASKED_BACK);
    const choice = asked.clarify?.choices[1];

    // The same question is read again, and a bare name still needs the model to read it, so this one waits too.
    const picked = api.askScan(id, ASKED_BACK, 'GB', choice?.id);

    timers.advance(ASK_MS);

    const answer = await picked;

    expect(answer.clarify).toBeUndefined();
    expect(answer.understood.scope).toEqual({ node: choice?.node, path: choice?.path });
    expect(answer.rows.length + answer.notes.length).toBeGreaterThan(0);
    await expectSoundRows(api, id, { ...answer, understood: { ...answer.understood, limit: 5 } });
  });

  it('refuses a pick that fits no choice, and a pick for a question that was not asked back', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const stale = api.askScan(id, ASKED_BACK, 'GB', 9);

    timers.advance(ASK_MS);

    expect(await expectCode(stale, 'invalid')).toBe(STALE);

    // A question the words settle names its folder: there is nothing to pick.
    expect(await expectCode(api.askScan(id, 'can I delete ~/Projekty?', 'GB', 0), 'invalid')).toBe(STALE);

    // And the model is free again after a refusal.
    expect((await ask(api, timers, id, 'what takes the most space?')).understood.intent).toBe('largest');
  });

  it('takes a pick only for a question it asked back, as the backend keeps the choices it showed', async () => {
    const { api, timers, scan } = setup();
    const id = await scan;
    const pickOf = async (question: string, choice: number): Promise<ScanAnswerDto> => {
      const picked = api.askScan(id, question, 'GB', choice);

      timers.advance(ASK_MS);

      return picked;
    };

    // Never asked back: refused, though this reading would ask back and has a choice under the id.
    expect(await expectCode(pickOf(ASKED_BACK, 0), 'invalid')).toBe(STALE);

    // Asked back once: its choices can be picked, also with the question written with other spaces.
    await ask(api, timers, id, ASKED_BACK);
    expect((await pickOf(ASKED_BACK, 0)).clarify).toBeUndefined();
    expect((await pickOf(`  ${ASKED_BACK.replace(' ', '  ')} `, 1)).clarify).toBeUndefined();
  });
});

describe('the keyword reading', () => {
  const names = new Set(['downloads', 'projekty', '.cache', '.local', 'videos']);
  const read = (question: string) => interpret(question, names);

  it("reads the intent, kind, limit and scope of the plan's examples", () => {
    expect(read('what takes the most space?')).toEqual({
      intent: 'largest',
      kind: 'any',
      limit: 5,
      scope: '',
    });
    expect(read('biggest folders in ~/Projekty')).toEqual({
      intent: 'largest',
      kind: 'any',
      limit: 5,
      scope: '~/Projekty',
    });
    expect(read('what are my biggest caches?')).toMatchObject({ intent: 'largest', kind: 'cache' });
    expect(read('how much do my videos take?')).toMatchObject({ intent: 'largest', kind: 'media' });
    expect(read('what can I clean in ~/Projekty?')).toMatchObject({
      intent: 'cleanable',
      scope: '~/Projekty',
    });
    expect(read('delete everything in /')).toMatchObject({ intent: 'cleanable', scope: '/' });
    expect(read('what did the model label in Downloads?')).toMatchObject({
      intent: 'labelled',
      scope: 'Downloads',
    });
    expect(read('why is ~/.local so big?')).toMatchObject({ intent: 'explain', scope: '~/.local' });
    expect(read('where is node_modules?')).toMatchObject({ intent: 'find', scope: 'node_modules' });
    expect(read('overview')).toMatchObject({ intent: 'general', scope: '' });
    expect(read('write me a poem')).toMatchObject({ intent: 'offtopic', scope: '' });
  });

  it('reads the name of a tool or place the rules know with the words after it, and never as a kind', () => {
    expect(read('can i safely delete the docker images')).toMatchObject({
      intent: 'cleanable',
      kind: 'any',
      scope: 'docker images',
    });
    expect(read('is it safe to remove the Steam shader cache')).toMatchObject({
      intent: 'cleanable',
      kind: 'any',
      scope: 'Steam shader cache',
    });
    expect(read('can I delete node_modules?')).toMatchObject({
      intent: 'cleanable',
      kind: 'any',
      scope: 'node_modules',
    });
    expect(read('can i delete the steam files')).toMatchObject({ scope: 'steam files' });
    expect(read('what are my biggest builds?')).toMatchObject({
      intent: 'largest',
      kind: 'build',
      scope: '',
    });
  });

  it('reads a question in another language by the few words it knows', () => {
    expect(read('co mogę usunąć?')).toMatchObject({ intent: 'cleanable', kind: 'any', scope: '' });
  });

  it('keeps a kind only for largest and cleanable, and takes a path before any word', () => {
    expect(read('why are my caches so big?')).toMatchObject({ intent: 'explain', kind: 'any' });
    expect(read('what can I clean in ~/.cache/pip')).toMatchObject({ kind: 'any', scope: '~/.cache/pip' });
    expect(read('where is the biggest cache?')).toMatchObject({ intent: 'largest', kind: 'cache' });
  });

  it('clamps the limit to 1..10', () => {
    expect(read('top 0 folders').limit).toBe(1);
    expect(read('top 3 biggest folders').limit).toBe(3);
    expect(read('the 50 biggest folders').limit).toBe(MAX_LIMIT);
  });

  describe('grounding', () => {
    const asked = (
      intent: ScanQuery['intent'],
      scope: string,
      kind: ScanQuery['kind'] = 'any',
    ): ScanQuery => ({
      intent,
      kind,
      limit: 5,
      scope,
    });

    it('keeps a scope the question holds as whole words, ignoring case, a leading ~/ and a trailing /', () => {
      const question = 'What is biggest in ~/Projekty/app?';

      expect(ground(question, asked('largest', '~/Projekty/app')).scope).toBe('~/Projekty/app');
      expect(ground(question, asked('largest', 'Projekty/app/')).scope).toBe('Projekty/app/');
      expect(ground('why is DOWNLOADS big', asked('explain', 'downloads')).scope).toBe('downloads');
      expect(ground('what is in my downloads folder', asked('explain', 'Downloads')).scope).toBe('Downloads');
    });

    it('drops a scope that is a piece of a word, one character, or a stop word', () => {
      expect(ground('what can I clean', asked('cleanable', 'an')).scope).toBe('');
      expect(ground('what is in Downloads', asked('explain', 'down')).scope).toBe('');
      expect(ground('what is in x', asked('explain', 'x')).scope).toBe('');
      expect(ground('what is the biggest', asked('largest', 'the')).scope).toBe('');
      expect(ground('what is the biggest', asked('largest', 'is')).scope).toBe('');
      expect(ground('what is the biggest', asked('largest', 'Atlantis')).scope).toBe('');
      expect(ground('what is the biggest', asked('largest', '')).scope).toBe('');
    });

    it('keeps ~ for home or ~ and / for a question with a slash', () => {
      expect(ground('what is in my home', asked('largest', '~')).scope).toBe('~');
      expect(ground('what is in ~', asked('largest', '~/')).scope).toBe('~/');
      expect(ground('what is biggest', asked('largest', '~')).scope).toBe('');
      expect(ground('delete everything in /', asked('cleanable', '/')).scope).toBe('/');
      expect(ground('delete everything', asked('cleanable', '/')).scope).toBe('');
    });

    it('keeps the kind for largest, cleanable and find only', () => {
      expect(ground('big caches', asked('largest', '', 'cache')).kind).toBe('cache');
      expect(ground('clean caches', asked('cleanable', '', 'cache')).kind).toBe('cache');
      expect(ground('why caches', asked('explain', '', 'cache')).kind).toBe('any');
      expect(ground('find caches', asked('find', '', 'cache')).kind).toBe('cache');
      expect(ground('overview of caches', asked('general', '', 'cache')).kind).toBe('any');
    });
  });

  it('uses only rules that exist, for the kind families', () => {
    for (const id of [...CACHE_FAMILY, ...BUILD_FAMILY]) {
      expect(RULE_IDS, id).toContain(id);
    }
  });
});
