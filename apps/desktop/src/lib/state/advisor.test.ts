import { describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type Api } from '../api/client';
import { createMockApi } from '../api/mock';
import type { MockAdvisorOptions } from '../api/mock-advisor';
import { FakeTimers } from '../api/mock-advisor.testkit';
import type { AdvisorLabelsDto, AdvisorStatusDto, ScanEvent, ScanId } from '../api/types';
import { statusOf } from '../features/advisor/fixtures';
import { AdvisorStore } from './advisor.svelte';
import { SETTINGS_KEY, SettingsStore } from './settings.svelte';

const NOW = 1_750_000_000;
const DOWNLOAD_MS = 100 * 30;
const LOAD_MS = 400;
const LABEL_MS = 300;
const MODEL_BYTES = 2_740_937_888;

class MemoryStorage {
  data = new Map<string, string>();

  getItem(key: string): string | null {
    return this.data.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.data.set(key, value);
  }
}

function scanOf(api: Api, root = '/'): Promise<ScanId> {
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

/** A store over the mock model with timers that only move when the test says so. */
function setup(advisor: MockAdvisorOptions = {}, wrap: (api: Api) => Api = (api) => api) {
  const timers = new FakeTimers();
  const mock = createMockApi({ durationMs: 0, now: () => NOW, advisor: { timers, ...advisor } });
  const api = wrap(mock);
  const storage = new MemoryStorage();
  const settings = new SettingsStore(storage);
  const store = new AdvisorStore(api, settings);

  return { api, mock, timers, settings, storage, store };
}

/** An API whose first `configureAdvisor` answers with the status it saw when called, but only once `release` is called. */
function gatedConfigure(): { wrap: (api: Api) => Api; release: () => void } {
  let release: () => void = () => undefined;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });

  let held = false;

  return {
    release,
    wrap: (api) => ({
      ...api,
      configureAdvisor: async (options) => {
        const answer = await api.configureAdvisor(options);

        if (!held) {
          held = true;
          await gate;
        }

        return answer;
      },
    }),
  };
}

describe('an answer to a command and a status pushed meanwhile', () => {
  it('does not let the answer of a command overwrite a status that was pushed after the command began', async () => {
    const gated = gatedConfigure();
    const { store, timers, settings } = setup({}, gated.wrap);

    await store.watch();
    await store.download();

    const pending = store.configure();

    timers.advance(DOWNLOAD_MS + 100);

    expect(store.status?.state).toBe('onDisk');
    expect(settings.advisorEnabled).toBe(true);

    gated.release();
    await pending;

    expect(store.status?.state).toBe('onDisk');
  });

  it('does not read a labelling as ended because an older answer says there is none', async () => {
    const gated = gatedConfigure();
    const { store, settings, api } = setup({ state: 'loaded' }, gated.wrap);
    const scan = await scanOf(api);

    settings.set('advisorEnabled', true);
    await store.watch();

    const pending = store.configure();
    const version = store.suggestionsVersion;

    await store.label(scan);
    expect(store.status?.labelling?.scan).toBe(scan);

    gated.release();
    await pending;

    expect(store.status?.labelling?.scan).toBe(scan);
    expect(store.labelsPending).toBe(false);
    expect(store.suggestionsVersion).toBe(version);
  });

  it('applies the answer of a command when nothing was pushed meanwhile', async () => {
    const { store } = setup({ state: 'disk' });

    await store.watch();
    await store.setEnabled(true);

    expect(store.status?.options.enabled).toBe(true);
  });

  it('switches the model on when the answer of a command is the first to carry the finished download', async () => {
    const answers: AdvisorStatusDto[] = [
      statusOf('onDisk', {
        options: { enabled: false, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true },
      }),
    ];
    const api = {
      watchAdvisor: async (onEvent: (status: AdvisorStatusDto) => void) =>
        onEvent(
          statusOf('downloading', {
            options: { enabled: false, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true },
          }),
        ),
      configureAdvisor: async () => answers.shift() ?? statusOf('onDisk'),
    } as unknown as Api;
    const settings = new SettingsStore(new MemoryStorage());
    const store = new AdvisorStore(api, settings);

    await store.watch();
    await store.configure();

    expect(store.status?.state).toBe('onDisk');
    expect(settings.advisorEnabled).toBe(true);
  });

  it('syncs the backend when the finished download arrives and the setting is on but the backend is off', async () => {
    const off = { enabled: false, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true } as const;
    const configure = vi.fn(async () => statusOf('onDisk'));
    let push: (status: AdvisorStatusDto) => void = () => undefined;
    const api = {
      watchAdvisor: async (onEvent: (status: AdvisorStatusDto) => void) => {
        push = onEvent;
        onEvent(statusOf('downloading', { options: off }));
      },
      configureAdvisor: configure,
    } as unknown as Api;
    const settings = new SettingsStore(new MemoryStorage());
    const store = new AdvisorStore(api, settings);

    settings.set('advisorEnabled', true);
    await store.watch();
    push(statusOf('onDisk', { options: off }));

    await vi.waitFor(() => expect(store.status?.options.enabled).toBe(true));
    expect(configure).toHaveBeenCalledWith({
      enabled: true,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });
  });
});

describe('the failure of the last action', () => {
  it('is cleared by a configure that succeeds, by a cancelled download and when a new action starts', async () => {
    const { store, timers } = setup({ state: 'disk' });

    await store.watch();

    store.failure = 'It failed.';
    await store.configure();
    expect(store.failure).toBeUndefined();

    store.failure = 'It failed.';
    await store.cancelDownload();
    expect(store.failure).toBeUndefined();

    store.failure = 'It failed.';
    await store.setEnabled(true);
    expect(store.failure).toBeUndefined();

    store.failure = 'It failed.';

    const loading = store.load();

    expect(store.failure).toBeUndefined();
    timers.advance(LOAD_MS);
    await loading;
  });

  it('is cleared when a download starts', async () => {
    const { store } = setup();

    await store.watch();

    store.failure = 'It failed.';

    const downloading = store.download();

    expect(store.failure).toBeUndefined();
    await downloading;
  });
});

describe('watch and configure', () => {
  it('has the status at once when it subscribes', async () => {
    const { store } = setup({ state: 'disk' });

    expect(store.status).toBeUndefined();

    await store.watch();

    expect(store.status?.state).toBe('onDisk');
    expect(store.loaded).toBe(false);
  });

  it('follows every push of the backend', async () => {
    const { store, timers } = setup({ state: 'disk' });

    await store.watch();
    await store.setEnabled(true);

    const loading = store.load();

    expect(store.status?.state).toBe('loading');
    timers.advance(LOAD_MS);
    await loading;

    expect(store.status?.state).toBe('loaded');
    expect(store.loaded).toBe(true);
  });

  it('sends the question log option of the settings, and a change of it', async () => {
    const { store, settings, api } = setup({ state: 'disk' });
    const configure = vi.spyOn(api, 'configureAdvisor');

    await store.configure();
    expect(configure).toHaveBeenLastCalledWith(expect.objectContaining({ logQuestions: true }));

    await store.setLogQuestions(false);

    expect(settings.advisorLogQuestions).toBe(false);
    expect(configure).toHaveBeenLastCalledWith(expect.objectContaining({ logQuestions: false }));
    expect(store.status?.options.logQuestions).toBe(false);
  });

  it('sends the graphics card option of the settings, and the mock unloads the model when it changes', async () => {
    const { store, settings, api } = setup({ state: 'loaded' });
    const configure = vi.spyOn(api, 'configureAdvisor');

    settings.set('advisorEnabled', true);
    await store.watch();
    await store.configure();
    expect(configure).toHaveBeenLastCalledWith(expect.objectContaining({ useGpu: true }));
    expect(store.status?.device?.kind).toBe('gpu');

    await store.setUseGpu(false);

    expect(settings.advisorUseGpu).toBe(false);
    expect(configure).toHaveBeenLastCalledWith(expect.objectContaining({ useGpu: false }));
    expect(store.status?.options.useGpu).toBe(false);
    expect(store.status?.state).toBe('onDisk');
    expect(store.status?.device).toBeUndefined();
  });

  it('clears the question log, and keeps the refusal of a log it cannot remove', async () => {
    const ok = setup({ state: 'disk' });

    expect(await ok.store.clearQuestionLog()).toBe(true);
    expect(ok.store.failure).toBeUndefined();

    const blocked = setup({ state: 'disk', questionLogBlocked: true });

    expect(await blocked.store.clearQuestionLog()).toBe(false);
    expect(blocked.store.failure).toBe('The question log could not be removed.');
  });

  it('takes a no in the dialog before a log is cleared quietly', async () => {
    const declined = setup({ state: 'disk', confirmClearLog: () => false });

    expect(await declined.store.clearQuestionLog()).toBe(false);
    expect(declined.store.failure).toBeUndefined();
    expect(await declined.store.clearGpuLog()).toBe(false);
    expect(declined.store.failure).toBeUndefined();
  });

  it('sends the options of the settings, and saves a change before it sends it', async () => {
    const { store, settings, storage, api } = setup({ state: 'disk' });
    const configure = vi.spyOn(api, 'configureAdvisor');

    await store.configure();

    expect(configure).toHaveBeenLastCalledWith({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });
    expect(store.status?.options).toEqual({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });

    await store.setEnabled(true);
    await store.setUnloadAfter('windowClose');

    expect(configure).toHaveBeenLastCalledWith({
      enabled: true,
      unloadAfter: 'windowClose',
      logQuestions: true,
      useGpu: true,
    });
    expect(settings.advisorEnabled).toBe(true);
    expect(JSON.parse(storage.getItem(SETTINGS_KEY) ?? '{}')).toMatchObject({
      advisorEnabled: true,
      advisorUnloadAfter: 'windowClose',
    });
  });

  it('bumps the suggestions version when the model is switched on or off, not for other options', async () => {
    const { store } = setup({ state: 'disk' });

    await store.configure();
    await store.watch();

    const start = store.suggestionsVersion;

    await store.setUnloadAfter('oneMinute');
    expect(store.suggestionsVersion).toBe(start);

    await store.setEnabled(true);
    expect(store.suggestionsVersion).toBe(start + 1);

    await store.setEnabled(false);
    expect(store.suggestionsVersion).toBe(start + 2);
  });
});

describe('download', () => {
  it('starts a download that the backend confirmed, and shows its progress', async () => {
    const { store, timers } = setup();

    await store.watch();
    await store.download();

    expect(store.status?.state).toBe('downloading');
    expect(store.failure).toBeUndefined();

    timers.advance(DOWNLOAD_MS / 2);

    expect(store.status?.download?.received).toBeGreaterThan(0);
  });

  it('says nothing when the user declines the backend dialog, and the model stays as it was', async () => {
    const { store } = setup({ confirmDownload: () => false });

    await store.watch();
    await store.download();

    expect(store.status?.state).toBe('notDownloaded');
    expect(store.failure).toBeUndefined();
    expect(store.asking).toBe(false);
  });

  it('shows the message of a refused request as text', async () => {
    const { store } = setup({}, (api) => ({
      ...api,
      startModelDownload: () => Promise.reject(new ApiRequestError('invalid', 'One dialog is open already.')),
    }));

    await store.watch();
    await store.download();

    expect(store.failure).toBe('One dialog is open already.');
    expect(store.asking).toBe(false);
  });

  it('asks for one dialog at a time, and only while the model is not downloaded', async () => {
    const { store, api } = setup({ state: 'disk' });
    const start = vi.spyOn(api, 'startModelDownload');

    await store.watch();
    await store.download();
    expect(start).not.toHaveBeenCalled();
  });

  it('switches the model on when the download is finished', async () => {
    const { store, timers, settings } = setup();

    await store.configure();
    await store.watch();
    await store.download();

    expect(settings.advisorEnabled).toBe(false);

    timers.advance(DOWNLOAD_MS + 100);

    expect(store.status?.state).toBe('onDisk');
    expect(settings.advisorEnabled).toBe(true);
    await vi.waitFor(() => expect(store.status?.options.enabled).toBe(true));
  });

  it('keeps a model the user switched off when it was only loaded or unloaded', async () => {
    const { store, settings, timers } = setup({ state: 'disk' });

    settings.set('advisorEnabled', true);
    await store.configure();
    await store.watch();
    await store.setEnabled(false);
    await store.load();
    timers.advance(LOAD_MS);

    expect(settings.advisorEnabled).toBe(false);
  });

  it('cancels a download and keeps what arrived, so it can continue', async () => {
    const { store, timers } = setup();

    await store.watch();
    await store.download();
    timers.advance(DOWNLOAD_MS / 3);
    await store.cancelDownload();

    expect(store.status?.state).toBe('notDownloaded');
    expect(store.status?.download).toBeDefined();
  });
});

describe('load and unload', () => {
  it('loads a model on disk and unloads it again', async () => {
    const { store, timers, settings } = setup({ state: 'disk' });

    settings.set('advisorEnabled', true);
    await store.configure();
    await store.watch();

    const loading = store.load();

    expect(store.switching).toBe(true);
    timers.advance(LOAD_MS);
    await loading;

    expect(store.status?.state).toBe('loaded');
    expect(store.status?.ramBytes).toBeGreaterThan(0);
    expect(store.switching).toBe(false);

    await store.unload();

    expect(store.status?.state).toBe('onDisk');
  });

  it('shows why a load was refused', async () => {
    const { store } = setup({ state: 'disk' });

    await store.configure();
    await store.watch();
    await store.load();

    expect(store.failure).toBe('The local model is switched off in the settings.');
    expect(store.status?.state).toBe('onDisk');
  });
});

describe('labels', () => {
  async function labelled() {
    const fixture = setup({ state: 'loaded' });

    fixture.settings.set('advisorEnabled', true);
    await fixture.store.configure();
    await fixture.store.watch();

    return { ...fixture, scan: await scanOf(fixture.api) };
  }

  it('fetches the labels and bumps the suggestions version when a labelling ends', async () => {
    const { store, timers, scan } = await labelled();
    const before = store.suggestionsVersion;

    await store.loadLabels(scan);
    const { candidates } = store.labelsOf(scan) ?? { candidates: 0 };

    expect(candidates).toBeGreaterThan(0);
    expect(store.labelsOf(scan)?.labelled).toBe(0);

    await store.label(scan);

    expect(store.status?.labelling).toEqual({ scan, done: 0, total: candidates });

    timers.advance(LABEL_MS * (candidates + 1));

    await vi.waitFor(() => expect(store.labelsOf(scan)?.labelled).toBe(candidates));
    await vi.waitFor(() => expect(store.suggestionsVersion).toBe(before + 1));
    expect(store.status?.labelling).toBeUndefined();
  });

  it('moves the suggestions version and reads the labels again when the user rules change', async () => {
    const { store, api, timers, scan } = await labelled();

    await store.loadLabels(scan);
    await store.label(scan);
    timers.advance(LABEL_MS * 20);
    await vi.waitFor(() => expect(store.labelsOf(scan)?.labelled).toBeGreaterThan(0));

    const target = store.labelsOf(scan)?.labels[0];
    const before = store.suggestionsVersion;

    expect(target?.decided).toBeUndefined();

    await api.addUserRule(scan, target?.node as number, 'keep');
    store.rulesChanged();

    expect(store.suggestionsVersion).toBe(before + 1);
    await vi.waitFor(() =>
      expect(store.labelsOf(scan)?.labels.find((entry) => entry.node === target?.node)?.decided).toBe('keep'),
    );
  });

  it('only moves the suggestions version when the user rules change before any labels were read', async () => {
    const { store } = setup();
    const before = store.suggestionsVersion;

    store.rulesChanged();

    expect(store.suggestionsVersion).toBe(before + 1);
    expect(store.labels).toBeUndefined();
  });

  it('shows the labels only for their own scan', async () => {
    const { store, scan } = await labelled();

    await store.loadLabels(scan);

    expect(store.labelsOf(scan)).toBeDefined();
    expect(store.labelsOf(scan + 1)).toBeUndefined();
  });

  it('shows why a labelling was refused, and clears it on the next try', async () => {
    const { store, scan, timers } = await labelled();

    await store.label(scan);
    await store.label(scan);

    expect(store.labelFailure).toBe('The model is busy with another job.');

    await store.cancelLabelling();
    timers.advance(LABEL_MS);
    await store.label(scan);

    expect(store.labelFailure).toBeUndefined();
  });

  it('stops a labelling and keeps what was labelled', async () => {
    const { store, scan, timers } = await labelled();

    await store.label(scan);
    timers.advance(LABEL_MS * 3);
    await store.cancelLabelling();

    await vi.waitFor(() => expect(store.labelsOf(scan)?.stopped).toBe('cancelled'));
    expect(store.labelsOf(scan)?.labelled).toBe(3);
  });

  it('drops the answer of an older labels request', async () => {
    const { store, scan } = await labelled();
    const first = store.loadLabels(scan);
    const second = store.loadLabels(scan);

    expect(await Promise.all([first, second])).toEqual([false, true]);
    expect(store.labelsOf(scan)).toBeDefined();
  });
});

describe('overlapping labels requests', () => {
  interface Call {
    resolve: (labels: AdvisorLabelsDto) => void;
    reject: () => void;
  }

  const calls: Call[] = [];

  /** A store whose label requests wait for the test; `real` is what the mock would have answered. */
  async function held() {
    const fixture = setup({ state: 'loaded' }, (mock) => {
      calls.length = 0;

      return {
        ...mock,
        getAdvisorLabels: () =>
          new Promise<AdvisorLabelsDto>((resolve, reject) => {
            calls.push({ resolve, reject: () => reject(new Error('unreadable')) });
          }),
      };
    });
    const scan = await scanOf(fixture.api);
    const labels = await fixture.mock.getAdvisorLabels(scan);

    fixture.settings.set('advisorEnabled', true);
    await fixture.store.configure();
    await fixture.store.watch();

    return { ...fixture, scan, labels };
  }

  it('stores only the latest, and says so, so a dropped one settles nothing', async () => {
    const { store, scan, labels } = await held();
    const first = store.loadLabels(scan);
    const second = store.loadLabels(scan);

    calls[0]?.resolve(labels);
    expect(await first).toBe(false);
    expect(store.labelsOf(scan)).toBeUndefined();
    expect(store.labelsLoading).toBe(true);

    calls[1]?.resolve(labels);
    expect(await second).toBe(true);
    expect(store.labelsOf(scan)).toEqual(labels);
    expect(store.labelsLoading).toBe(false);
  });

  it('stores the failure of the latest request, so the labels read as missing, not as loading', async () => {
    const { store, scan, labels } = await held();
    const first = store.loadLabels(scan);
    const second = store.loadLabels(scan);

    calls[0]?.reject();
    calls[1]?.reject();

    expect(await first).toBe(false);
    expect(await second).toBe(true);
    expect(store.labelsOf(scan)).toBeUndefined();
    expect(store.labelsLoading).toBe(false);

    const third = store.loadLabels(scan);

    calls[2]?.resolve(labels);
    await third;
    expect(store.labelsOf(scan)).toEqual(labels);
  });

  it('keeps "reading the labels" and holds the suggestions until the latest request has stored', async () => {
    const { store, scan, labels, timers } = await held();
    const version = store.suggestionsVersion;

    await store.label(scan);
    timers.advance(LABEL_MS * 100);

    expect(store.status?.labelling).toBeUndefined();
    expect(store.labelsPending).toBe(true);
    expect(calls).toHaveLength(1);

    const newer = store.loadLabels(scan);

    calls[0]?.resolve(labels);
    await Promise.resolve();

    expect(store.labelsPending).toBe(true);
    expect(store.suggestionsVersion).toBe(version);

    calls[1]?.resolve(labels);
    await newer;

    expect(store.labelsPending).toBe(false);
    expect(store.suggestionsVersion).toBe(version + 1);
  });
});

describe('questions', () => {
  it('shows the question at once and the answer when it comes, with no wait for a model', async () => {
    const { store, settings, api } = setup({ state: 'loaded' });
    const askAdvisor = vi.spyOn(api, 'askAdvisor');

    settings.set('advisorEnabled', true);
    settings.set('units', 'GiB');
    await store.configure();
    const scan = await scanOf(api, '/home/demo');
    const [match] = await store.find(scan, 'Downloads');

    expect(match?.path).toContain('Downloads');

    const asking = store.ask(scan, match?.node as number, match?.path as string);

    expect(store.exchangesOf(scan)).toHaveLength(1);
    expect(store.exchangesOf(scan)[0]).toMatchObject({ kind: 'folder', path: match?.path });
    expect(store.exchangesOf(scan)[0]?.answer).toBeUndefined();

    // No timer is advanced: the rules answer.
    await asking;

    expect(askAdvisor).toHaveBeenCalledWith(scan, match?.node, 'GiB');

    const [exchange] = store.exchangesOf(scan);

    expect(exchange?.kind === 'folder' && exchange.answer?.node).toBe(match?.node);
    expect(exchange?.error).toBeUndefined();
    expect(store.exchangesOf(scan + 1)).toEqual([]);
  });

  it('answers a folder question with the model not downloaded', async () => {
    const { store, api } = setup({ state: 'none' });
    const scan = await scanOf(api, '/home/demo');
    const [match] = await store.find(scan, 'Downloads');

    await store.configure();
    await store.ask(scan, match?.node as number, match?.path as string);

    expect(store.exchangesOf(scan)[0]?.error).toBeUndefined();
    expect(store.exchangesOf(scan)[0]?.answer).toBeDefined();
  });

  it('answers a folder question with the model switched off', async () => {
    const { store, settings, api } = setup({ state: 'loaded' });
    const scan = await scanOf(api, '/home/demo');
    const [match] = await store.find(scan, 'Downloads');

    settings.set('advisorEnabled', false);
    await store.configure();
    await store.ask(scan, match?.node as number, match?.path as string);

    expect(store.exchangesOf(scan)[0]?.error).toBeUndefined();
    expect(store.exchangesOf(scan)[0]?.answer).toBeDefined();
  });

  it('answers a folder question while folders are being labelled', async () => {
    const { store, settings, api } = setup({ state: 'loaded' });
    const scan = await scanOf(api, '/home/demo');
    const [match] = await store.find(scan, 'Downloads');

    settings.set('advisorEnabled', true);
    await store.configure();
    await store.watch();
    await store.label(scan);

    expect(store.status?.labelling).toBeDefined();

    await store.ask(scan, match?.node as number, match?.path as string);

    expect(store.exchangesOf(scan)[0]?.error).toBeUndefined();
    expect(store.exchangesOf(scan)[0]?.answer).toBeDefined();
  });

  it('keeps the reason a folder question had no answer as text', async () => {
    const { store, api } = setup({ state: 'loaded' }, (mock) => ({
      ...mock,
      askAdvisor: async () => {
        throw new ApiRequestError('invalid', 'only a folder can be asked about');
      },
    }));
    const scan = await scanOf(api, '/home/demo');
    const [match] = await store.find(scan, 'Downloads');

    await store.ask(scan, match?.node as number, match?.path as string);

    const [exchange] = store.exchangesOf(scan);

    expect(exchange?.answer).toBeUndefined();
    expect(exchange?.error).toBe('only a folder can be asked about');
  });

  it('asks a free question: it shows at once, trimmed, with the units of the settings, and the answer joins it', async () => {
    const { store, timers, settings, api } = setup({ state: 'loaded' });
    const askScan = vi.spyOn(api, 'askScan');

    settings.set('advisorEnabled', true);
    settings.set('units', 'GiB');
    await store.configure();
    const scan = await scanOf(api, '/home/demo');

    const asking = store.askScan(scan, '  what takes the most space?  ');

    expect(store.exchangesOf(scan)).toMatchObject([{ kind: 'scan', question: 'what takes the most space?' }]);
    expect(store.exchangesOf(scan)[0]?.answer).toBeUndefined();

    timers.advance(2000);
    await asking;

    const [exchange] = store.exchangesOf(scan);

    expect(askScan).toHaveBeenCalledWith(scan, 'what takes the most space?', 'GiB');
    expect(exchange?.kind === 'scan' && exchange.answer?.rows.length).toBeGreaterThan(0);
    expect(exchange?.error).toBeUndefined();
  });

  it('answers a question the words settle at once, with no wait for the model', async () => {
    const { store, settings, api } = setup({ state: 'loaded' });

    settings.set('advisorEnabled', true);
    await store.configure();
    const scan = await scanOf(api);

    // No timer is advanced: the model reads nothing.
    await store.askScan(scan, 'can i safely delete the docker images');

    const [exchange] = store.exchangesOf(scan);

    expect(exchange?.error).toBeUndefined();
    expect(exchange?.kind === 'scan' && exchange.answer?.rows.map((row) => row.path)).toEqual([
      '/var/lib/docker',
    ]);
    expect(store.waiting).toBe(false);
  });

  it('asks the same question again with the id of a choice of the answer that asked back, as a new exchange', async () => {
    const { store, timers, settings, api } = setup({ state: 'loaded' });
    const askScan = vi.spyOn(api, 'askScan');

    settings.set('advisorEnabled', true);
    settings.set('units', 'GB');
    await store.configure();
    const scan = await scanOf(api, '/home/demo');

    const first = store.askScan(scan, 'can I delete the src folder?');

    timers.advance(2000);
    await first;

    const [asked] = store.exchangesOf(scan);
    const clarify = asked?.kind === 'scan' ? asked.answer?.clarify : undefined;
    const choice = clarify?.choices[1];

    expect(clarify?.choices.length).toBeGreaterThan(1);
    expect(asked?.kind === 'scan' && asked.pick).toBeUndefined();

    const picking = store.askScan(scan, 'can I delete the src folder?', choice);

    timers.advance(2000);
    await picking;

    const [, picked] = store.exchangesOf(scan);

    // The text and the id; the choice's path and node never go back.
    expect(askScan.mock.calls).toEqual([
      [scan, 'can I delete the src folder?', 'GB'],
      [scan, 'can I delete the src folder?', 'GB', choice?.id],
    ]);
    expect(picked).toMatchObject({ kind: 'scan', question: 'can I delete the src folder?', pick: { id: 1 } });
    expect(picked?.kind === 'scan' && picked.pick).toEqual({ id: choice?.id, label: choice?.label });
    expect(picked?.kind === 'scan' && picked.answer?.clarify).toBeUndefined();
    expect(picked?.kind === 'scan' && picked.answer?.understood.scope).toEqual({
      node: choice?.node,
      path: choice?.path,
    });
    expect(store.exchangesOf(scan)).toHaveLength(2);
  });

  it('keeps the reason a choice is refused as text, with the choice that was taken', async () => {
    const { store, timers, settings, api } = setup({ state: 'loaded' });

    settings.set('advisorEnabled', true);
    await store.configure();
    const scan = await scanOf(api, '/home/demo');

    // No question was asked back: nothing is there to pick.
    const stale = store.askScan(scan, 'can I delete ~/Projekty?', { id: 3, label: '~/old/choice' });

    timers.advance(2000);
    await stale;

    const [failed] = store.exchangesOf(scan);

    expect(failed?.error).toBe('that choice is no longer available, so ask again');
    expect(failed?.kind === 'scan' && failed.pick).toEqual({ id: 3, label: '~/old/choice' });
  });

  it('asks nothing for a blank question', async () => {
    const { store, api } = setup({ state: 'loaded' });
    const askScan = vi.spyOn(api, 'askScan');
    const scan = await scanOf(api, '/home/demo');

    await store.askScan(scan, '   ');

    expect(askScan).not.toHaveBeenCalled();
    expect(store.exchangesOf(scan)).toEqual([]);
  });

  it('keeps the reason a free question had no answer, and asks it again on retry', async () => {
    let failing = true;
    const { store, timers, settings, api } = setup({ state: 'loaded' }, (mock) => ({
      ...mock,
      askScan: async (scan, question, units) => {
        if (failing) {
          throw new ApiRequestError('invalid', 'The model is busy.');
        }

        return mock.askScan(scan, question, units);
      },
    }));

    settings.set('advisorEnabled', true);
    await store.configure();
    const scan = await scanOf(api, '/home/demo');

    await store.askScan(scan, 'biggest folders');

    const [failed] = store.exchangesOf(scan);

    expect(failed?.error).toBe('The model is busy.');

    failing = false;

    const again = store.retry(failed?.id as number);

    expect(store.exchangesOf(scan)[0]?.error).toBeUndefined();
    expect(store.exchangesOf(scan)).toHaveLength(1);

    timers.advance(2000);
    await again;

    expect(store.exchangesOf(scan)[0]?.answer).toBeDefined();
  });

  it('takes one free question at a time across scans; a folder question is never held back by it', async () => {
    const { store, timers, settings, api } = setup({ state: 'loaded' });
    const askScan = vi.spyOn(api, 'askScan');
    const askAdvisor = vi.spyOn(api, 'askAdvisor');

    settings.set('advisorEnabled', true);
    await store.configure();

    const first = await scanOf(api, '/home/demo');
    const second = await scanOf(api, '/home/demo');
    const [folder] = await store.find(second, 'Downloads');
    const running = store.askScan(first, 'what takes the most space?');

    expect(store.waiting).toBe(true);
    expect(store.scanWaiting).toBe(true);

    await store.askScan(second, 'overview');

    expect(askScan).toHaveBeenCalledTimes(1);
    expect(store.exchangesOf(second)).toEqual([]);
    expect(store.exchanges).toHaveLength(1);

    // The rules answer a folder at once, while the model reads the other question.
    await store.ask(second, folder?.node as number, folder?.path as string);

    expect(askAdvisor).toHaveBeenCalledTimes(1);
    expect(store.exchangesOf(second)).toMatchObject([{ kind: 'folder' }]);
    expect(store.exchangesOf(second)[0]?.answer).toBeDefined();
    expect(store.scanWaiting).toBe(true);

    timers.advance(2000);
    await running;

    expect(store.waiting).toBe(false);
    expect(store.scanWaiting).toBe(false);

    const next = store.askScan(second, 'overview');

    timers.advance(2000);
    await next;

    expect(askScan).toHaveBeenCalledTimes(2);
    expect(store.exchangesOf(second)).toHaveLength(2);
  });

  it('retries a failed folder question while a free one is on its way', async () => {
    let failing = true;
    const { store, timers, settings, api } = setup({ state: 'loaded' }, (mock) => ({
      ...mock,
      askAdvisor: async (scan, node, units) => {
        if (failing) {
          throw new ApiRequestError('invalid', 'Nothing could be read.');
        }

        return mock.askAdvisor(scan, node, units);
      },
    }));

    settings.set('advisorEnabled', true);
    await store.configure();
    const scan = await scanOf(api, '/home/demo');
    const [folder] = await store.find(scan, 'Downloads');

    await store.ask(scan, folder?.node as number, folder?.path as string);

    const [failed] = store.exchangesOf(scan);

    expect(failed?.error).toBe('Nothing could be read.');

    failing = false;

    const running = store.askScan(scan, 'what takes the most space?');

    await store.retry(failed?.id as number);

    expect(store.exchangesOf(scan)[0]?.error).toBeUndefined();
    expect(store.exchangesOf(scan)[0]?.answer).toBeDefined();

    timers.advance(2000);
    await running;
  });

  it('does not retry a failed question while another is on its way, and retries it once that is answered', async () => {
    let failing = true;
    const { store, timers, settings, api } = setup({ state: 'loaded' }, (mock) => ({
      ...mock,
      askScan: async (scan, question, units) => {
        if (failing && question === 'biggest folders') {
          throw new ApiRequestError('invalid', 'The model is busy.');
        }

        return mock.askScan(scan, question, units);
      },
    }));

    settings.set('advisorEnabled', true);
    await store.configure();
    const scan = await scanOf(api, '/home/demo');

    await store.askScan(scan, 'biggest folders');
    failing = false;

    const [failed] = store.exchangesOf(scan);
    const other = store.askScan(scan + 1, 'overview');

    await store.retry(failed?.id as number);

    expect(store.exchangesOf(scan)[0]?.error).toBe('The model is busy.');

    timers.advance(2000);
    await other;

    const again = store.retry(failed?.id as number);

    timers.advance(2000);
    await again;

    expect(store.exchangesOf(scan)[0]?.error).toBeUndefined();
    expect(store.exchangesOf(scan)[0]?.answer).toBeDefined();
  });

  it('finds folders through the API, and rejects what the API rejects', async () => {
    const { store, api } = setup();
    const scan = await scanOf(api, '/home/demo');

    expect((await store.find(scan, 'Downloads')).length).toBeGreaterThan(0);
    await expect(store.find(scan, 'x'.repeat(1025))).rejects.toMatchObject({ code: 'invalid' });
  });
});

describe('the one model', () => {
  it('downloads Qwen3.5-4B when the user asks for it', async () => {
    const { store, timers } = setup({ state: 'none' });

    await store.configure();
    await store.watch();
    await store.download();
    timers.advance(500);

    expect(store.status).toMatchObject({
      state: 'downloading',
      model: { name: 'Qwen3.5-4B' },
      download: { total: MODEL_BYTES },
    });
  });

  it('shows the refusal of the backend when the options are not taken', async () => {
    const { store } = setup({ state: 'none' }, (mock) => ({
      ...mock,
      configureAdvisor: async () => {
        throw new ApiRequestError('invalid', 'The advisor options are not valid.');
      },
    }));

    await store.configure();

    expect(store.failure).toBe('The advisor options are not valid.');
  });
});

describe('graphics card problems', () => {
  const incident: MockAdvisorOptions = { state: 'loaded', gpuIncident: { what: 'crash', pending: true } };

  /** The store after the first status: the mock model is loaded and a problem waits. */
  async function waiting(wrap?: (api: Api) => Api) {
    const setUp = setup(incident, wrap);

    setUp.settings.set('advisorEnabled', true);
    await setUp.store.watch();

    return setUp;
  }

  it('offers the problem that waits, and the last one of the log', async () => {
    const { store } = await waiting();

    expect(store.gpuProblem).toMatchObject({ what: 'crash' });
    expect(store.lastGpuProblem).toEqual(store.gpuProblem);
  });

  it('answers "use the processor", then saves the card off and configures; the problem is gone', async () => {
    const calls: string[] = [];
    const { store, settings } = await waiting((api) => ({
      ...api,
      answerGpuProblem: async (id, choice) => {
        calls.push(`answer:${choice}:${settings.advisorUseGpu}`);

        return api.answerGpuProblem(id, choice);
      },
      configureAdvisor: async (options) => {
        calls.push(`configure:${options.useGpu}`);

        return api.configureAdvisor(options);
      },
    }));
    const id = store.gpuProblem?.id;

    await store.answerGpuProblem('useCpu');

    expect(calls).toEqual(['answer:useCpu:true', 'configure:false']);
    expect(settings.advisorUseGpu).toBe(false);
    expect(store.gpuProblem).toBeUndefined();
    expect(store.lastGpuProblem?.id).toBe(id);
    expect(store.status?.state).toBe('loaded');
    expect(store.failure).toBeUndefined();
  });

  it('answers "try again" and nothing else: the option and the options sent stay as they are', async () => {
    const { store, settings, api } = await waiting();
    const answer = vi.spyOn(api, 'answerGpuProblem');
    const configure = vi.spyOn(api, 'configureAdvisor');
    const id = store.gpuProblem?.id;

    await store.answerGpuProblem('tryAgain');

    expect(answer).toHaveBeenCalledExactlyOnceWith(id, 'tryAgain');
    expect(configure).not.toHaveBeenCalled();
    expect(settings.advisorUseGpu).toBe(true);
    expect(store.gpuProblem).toBeUndefined();
  });

  it('answers once when the button is pressed twice', async () => {
    const { store, api } = await waiting();
    const answer = vi.spyOn(api, 'answerGpuProblem');

    await Promise.all([store.answerGpuProblem('tryAgain'), store.answerGpuProblem('tryAgain')]);

    expect(answer).toHaveBeenCalledTimes(1);
  });

  it('answers nothing when closed: it does not come back this session, and the backend still waits for the answer', async () => {
    const { store, api } = await waiting();
    const answer = vi.spyOn(api, 'answerGpuProblem');

    store.dismissGpuProblem();

    expect(store.gpuProblem).toBeUndefined();
    expect(answer).not.toHaveBeenCalled();

    // Other pushes of the status do not bring it back.
    await store.refresh();
    await store.setLogQuestions(false);

    expect(store.gpuProblem).toBeUndefined();
    expect(store.status?.gpuProblem).toBeDefined();
  });

  it('asks again about a new problem after one was closed', async () => {
    const { store } = await waiting();
    const first = store.gpuProblem?.id as number;

    store.dismissGpuProblem();

    store.status = {
      ...(store.status as AdvisorStatusDto),
      gpuProblem: { id: first + 1, at: first + 1, what: 'hang' },
    };

    expect(store.gpuProblem).toMatchObject({ id: first + 1, what: 'hang' });
  });

  it('shows the refusal of a problem that was answered elsewhere, and does not ask about it again', async () => {
    const { store, settings } = await waiting((api) => ({
      ...api,
      answerGpuProblem: async () => {
        throw new ApiRequestError('invalid', 'That graphics card problem is not waiting for an answer.');
      },
    }));

    await store.answerGpuProblem('useCpu');

    expect(store.failure).toBe('That graphics card problem is not waiting for an answer.');
    expect(store.gpuProblem).toBeUndefined();
    expect(settings.advisorUseGpu).toBe(true);
  });

  it('clears the log: the last problem goes, a waiting one stays', async () => {
    const { store, api } = await waiting();
    const clear = vi.spyOn(api, 'clearGpuLog');

    expect(await store.clearGpuLog()).toBe(true);
    expect(clear).toHaveBeenCalledTimes(1);
    expect(store.lastGpuProblem).toBeUndefined();
    expect(store.gpuProblem).toBeDefined();
  });

  it('says why the log was not cleared', async () => {
    const { store } = await waiting((api) => ({
      ...api,
      clearGpuLog: async () => {
        throw new ApiRequestError('io', 'The graphics card problem log could not be removed.');
      },
    }));

    expect(await store.clearGpuLog()).toBe(false);
    expect(store.failure).toBe('The graphics card problem log could not be removed.');
    expect(store.lastGpuProblem).toBeDefined();
  });
});
