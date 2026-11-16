import { describe, expect, it } from 'vitest';
import type { Api } from './client';
import { isApiError } from './client';
import { createMockApi } from './mock';
import { setMockAdvisorState, type MockAdvisorOptions } from './mock-advisor';
import { FakeTimers } from './mock-advisor.testkit';
import type { AdvisorStatusDto } from './types';

const MODEL_BYTES = 2_740_937_888;
const GIB = 1024 * 1024 * 1024;
const DOWNLOAD_MS = 3000;
const LOAD_MS = 400;

function setup(advisor: MockAdvisorOptions = {}): {
  api: Api;
  timers: FakeTimers;
  events: AdvisorStatusDto[];
} {
  const timers = new FakeTimers();
  const api = createMockApi({ advisor: { timers, ...advisor } });
  const events: AdvisorStatusDto[] = [];

  void api.watchAdvisor((status) => events.push(status));
  // The status pushed on subscription is covered by its own test; the others look at changes only.
  events.length = 0;

  return { api, timers, events };
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

async function expectInvalid(promise: Promise<unknown>): Promise<void> {
  const error = await rejection(promise);

  expect(isApiError(error) && error.code).toBe('invalid');
}

async function download(api: Api, timers: FakeTimers): Promise<void> {
  await api.startModelDownload();
  timers.advance(DOWNLOAD_MS);
}

async function load(api: Api, timers: FakeTimers): Promise<AdvisorStatusDto> {
  const loading = api.loadModel();
  timers.advance(LOAD_MS);

  return loading;
}

describe('advisor mock: status', () => {
  it('starts with the model not downloaded and describes the model the backend does', async () => {
    const { api } = setup();

    expect(await api.advisorStatus()).toEqual({
      state: 'notDownloaded',
      model: {
        name: 'Qwen3.5-4B',
        format: 'GGUF · Q4_K_M',
        bytes: MODEL_BYTES,
        host: 'huggingface.co',
      },
      options: { enabled: true, unloadAfter: 'fiveMinutes', logQuestions: true },
      ramTotal: 32 * GIB,
    });
  });

  it('hands out copies, so a caller cannot change the machine', async () => {
    const { api } = setup();
    const status = await api.advisorStatus();

    status.model.name = 'changed';
    status.options.enabled = false;

    const again = await api.advisorStatus();

    expect(again.model.name).toBe('Qwen3.5-4B');
    expect(again.options.enabled).toBe(true);
  });
});

describe('advisor mock: download', () => {
  it('reports progress in chunks over about three seconds and ends on disk', async () => {
    const { api, timers, events } = setup();

    await api.startModelDownload();

    expect(events.map((event) => event.state)).toEqual(['downloading']);
    expect(events[0]?.download).toEqual({ received: 0, total: MODEL_BYTES });

    timers.advance(DOWNLOAD_MS - 100);

    const progress = events
      .filter((event) => event.state === 'downloading')
      .map((event) => event.download?.received ?? 0);

    expect(progress.length).toBeGreaterThan(10);
    expect(progress).toEqual([...progress].sort((a, b) => a - b));
    expect(progress.at(-1)).toBeLessThan(MODEL_BYTES);
    expect((await api.advisorStatus()).state).toBe('downloading');

    timers.advance(100);

    const last = events.at(-1);

    expect(last?.state).toBe('onDisk');
    expect(last?.download).toBeUndefined();
    expect(timers.count).toBe(0);
  });

  it('keeps the received bytes on cancel and resumes from them', async () => {
    const { api, timers, events } = setup();

    await api.startModelDownload();
    timers.advance(1000);

    const before = (await api.advisorStatus()).download?.received ?? 0;

    expect(before).toBeGreaterThan(0);

    await api.cancelModelDownload();

    expect(await api.advisorStatus()).toMatchObject({
      state: 'notDownloaded',
      download: { received: before, total: MODEL_BYTES },
    });
    expect(timers.count).toBe(0);

    timers.advance(DOWNLOAD_MS);

    expect((await api.advisorStatus()).download?.received).toBe(before);

    events.length = 0;
    await api.startModelDownload();

    expect(events[0]).toMatchObject({ state: 'downloading', download: { received: before } });

    timers.advance(1000);

    expect((await api.advisorStatus()).download?.received).toBeGreaterThan(before);

    // Only what is left takes time: the whole download would still be running after 2.1 s in total.
    timers.advance(DOWNLOAD_MS - 1000 - 1000);

    expect((await api.advisorStatus()).state).toBe('onDisk');
  });

  it('treats a cancel with nothing downloading as nothing to do', async () => {
    const { api, events } = setup();

    await api.cancelModelDownload();

    expect(events).toEqual([]);
    expect((await api.advisorStatus()).state).toBe('notDownloaded');
  });

  it('refuses to start while downloading and once the model is on disk', async () => {
    const { api, timers } = setup();

    await api.startModelDownload();
    await expectInvalid(api.startModelDownload());

    timers.advance(DOWNLOAD_MS);

    await expectInvalid(api.startModelDownload());
  });
});

describe('advisor mock: the download confirmation', () => {
  it('rejects with cancelled when the dialog is declined, and leaves the model not downloaded', async () => {
    const { api, timers, events } = setup({ confirmDownload: () => false });
    const error = await rejection(api.startModelDownload());

    expect(isApiError(error) && error.code).toBe('cancelled');
    expect(events).toEqual([]);

    timers.advance(DOWNLOAD_MS);

    const status = await api.advisorStatus();

    expect(status.state).toBe('notDownloaded');
    expect(status.download).toBeUndefined();
  });

  it('refuses a new request for two seconds after a decline, without asking, then asks again', async () => {
    let asked = 0;
    const { api, timers } = setup({
      confirmDownload: () => {
        asked += 1;
        return false;
      },
    });

    await expectCode(api.startModelDownload(), 'cancelled');
    await expectInvalid(api.startModelDownload());
    expect(asked).toBe(1);

    timers.advance(2_000);

    await expectCode(api.startModelDownload(), 'cancelled');
    expect(asked).toBe(2);
  });

  it('starts once the dialog is confirmed, by default and on request', async () => {
    const asked = { count: 0 };
    const { api, timers } = setup({
      confirmDownload: () => {
        asked.count++;

        return asked.count > 1;
      },
    });

    await rejection(api.startModelDownload());
    timers.advance(2_000);
    await download(api, timers);

    expect(asked.count).toBe(2);
    expect((await api.advisorStatus()).state).toBe('onDisk');

    const plain = setup();

    await download(plain.api, plain.timers);

    expect((await plain.api.advisorStatus()).state).toBe('onDisk');
  });

  it('does not ask when a download cannot start anyway', async () => {
    const asked = { count: 0 };
    const { api } = setup({
      state: 'disk',
      confirmDownload: () => {
        asked.count++;

        return true;
      },
    });

    await expectInvalid(api.startModelDownload());

    expect(asked.count).toBe(0);
  });
});

describe('advisor mock: load and unload', () => {
  it('goes through loading to loaded with memory figures and no tokens per second', async () => {
    const { api, timers, events } = setup({ state: 'disk' });

    const loading = api.loadModel();

    expect(events.map((event) => event.state)).toEqual(['loading']);
    expect(events[0]?.ramBytes).toBeUndefined();

    timers.advance(LOAD_MS);

    const loaded = await loading;

    expect(loaded.state).toBe('loaded');
    expect(loaded.ramBytes).toBeCloseTo(2 * GIB, -8);
    expect(loaded.ramTotal).toBe(32 * GIB);
    expect(loaded.tokensPerSecond).toBeUndefined();
    expect(events.at(-1)).toEqual(loaded);
  });

  it('unloads back to disk and can load again', async () => {
    const { api, timers, events } = setup({ state: 'loaded' });

    const unloaded = await api.unloadModel();

    expect(unloaded.state).toBe('onDisk');
    expect(unloaded.ramBytes).toBeUndefined();
    expect(events.at(-1)).toEqual(unloaded);

    expect((await load(api, timers)).state).toBe('loaded');
  });

  it('walks the whole way from nothing to loaded and back', async () => {
    const { api, timers } = setup();

    await download(api, timers);
    expect((await api.advisorStatus()).state).toBe('onDisk');

    await load(api, timers);
    expect((await api.advisorStatus()).state).toBe('loaded');

    await api.unloadModel();
    expect((await api.advisorStatus()).state).toBe('onDisk');
  });
});

describe('advisor mock: invalid transitions', () => {
  it('rejects a load before the model is on disk and while loading; a loaded model answers', async () => {
    const { api, timers } = setup();

    await expectInvalid(api.loadModel());

    await api.startModelDownload();
    await expectInvalid(api.loadModel());

    timers.advance(DOWNLOAD_MS);

    const loading = api.loadModel();

    await expectInvalid(api.loadModel());

    timers.advance(LOAD_MS);
    await loading;

    expect((await api.loadModel()).state).toBe('loaded');
  });

  it('rejects a load while the model is switched off', async () => {
    const { api } = setup({ state: 'disk' });

    await api.configureAdvisor({ enabled: false, unloadAfter: 'fiveMinutes', logQuestions: true });

    await expectInvalid(api.loadModel());
    expect((await api.advisorStatus()).state).toBe('onDisk');
  });

  it('rejects an unload while loading; a model not in memory answers with its status', async () => {
    const { api, timers } = setup();

    expect((await api.unloadModel()).state).toBe('notDownloaded');

    await download(api, timers);
    expect((await api.unloadModel()).state).toBe('onDisk');

    const loading = api.loadModel();

    await expectInvalid(api.unloadModel());

    timers.advance(LOAD_MS);
    await loading;
  });

  it('rejects options that are not valid and leaves the old ones', async () => {
    const { api } = setup();

    await expectInvalid(
      api.configureAdvisor({ enabled: true, unloadAfter: 'never' as never, logQuestions: true }),
    );
    await expectInvalid(
      api.configureAdvisor({ enabled: 'yes' as never, unloadAfter: 'oneMinute', logQuestions: true }),
    );
    await expectInvalid(
      api.configureAdvisor({ enabled: true, unloadAfter: 'oneMinute', logQuestions: 'yes' as never }),
    );
    await expectInvalid(api.configureAdvisor({ enabled: true, unloadAfter: 'oneMinute' } as never));

    expect((await api.advisorStatus()).options).toEqual({
      enabled: true,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
    });
  });

  it('does not change the state or push anything when it rejects', async () => {
    const { api, events } = setup();

    await expectInvalid(api.loadModel());

    expect(events).toEqual([]);
    expect((await api.advisorStatus()).state).toBe('notDownloaded');
  });
});

describe('advisor mock: configure', () => {
  it('stores the options and returns the status', async () => {
    const { api } = setup();

    const status = await api.configureAdvisor({
      enabled: true,
      unloadAfter: 'oneMinute',
      logQuestions: true,
    });

    expect(status.options).toEqual({ enabled: true, unloadAfter: 'oneMinute', logQuestions: true });
    expect((await api.advisorStatus()).options).toEqual({
      enabled: true,
      unloadAfter: 'oneMinute',
      logQuestions: true,
    });
  });

  it('pushes a change of the question log option alone', async () => {
    const { api, events } = setup();

    await api.configureAdvisor({ enabled: true, unloadAfter: 'fiveMinutes', logQuestions: false });

    expect(events.map((status) => status.options.logQuestions)).toEqual([false]);
  });

  it('unloads a loaded model when it is switched off', async () => {
    const { api, events } = setup({ state: 'loaded' });

    const status = await api.configureAdvisor({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
    });

    expect(status).toMatchObject({ state: 'onDisk', options: { enabled: false } });
    expect(events.at(-1)?.state).toBe('onDisk');
  });

  it('stops a load in progress when it is switched off', async () => {
    const { api, timers } = setup({ state: 'disk' });

    const loading = api.loadModel();

    await api.configureAdvisor({ enabled: false, unloadAfter: 'fiveMinutes', logQuestions: true });

    const ended = await loading;

    expect(ended).toMatchObject({
      state: 'onDisk',
      problem: 'The local model is switched off in the settings.',
    });
    expect((await api.advisorStatus()).state).toBe('onDisk');
    expect(timers.count).toBe(0);
  });

  it('leaves an unloaded model alone when it is switched off, and pushes the new options', async () => {
    const { api, events } = setup({ state: 'disk' });

    const status = await api.configureAdvisor({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
    });

    expect(status.state).toBe('onDisk');
    expect(events.map((event) => [event.state, event.options.enabled])).toEqual([['onDisk', false]]);
  });

  it('pushes nothing when the options do not change', async () => {
    const { api, events } = setup({ state: 'disk' });

    await api.configureAdvisor({ enabled: true, unloadAfter: 'fiveMinutes', logQuestions: true });

    expect(events).toEqual([]);
  });
});

describe('advisor mock: idle unload', () => {
  it('unloads after five minutes by default and pushes the new status', async () => {
    const { api, timers, events } = setup();

    await download(api, timers);
    await load(api, timers);
    events.length = 0;

    timers.advance(5 * 60_000 - 1);

    expect((await api.advisorStatus()).state).toBe('loaded');
    expect(events).toEqual([]);

    timers.advance(1);

    expect((await api.advisorStatus()).state).toBe('onDisk');
    expect(events.map((event) => event.state)).toEqual(['onDisk']);
  });

  it('follows the configured time; the same options again do not restart the wait', async () => {
    const { api, timers } = setup({ state: 'loaded' });

    await api.configureAdvisor({ enabled: true, unloadAfter: 'oneMinute', logQuestions: true });
    timers.advance(59_999);

    expect((await api.advisorStatus()).state).toBe('loaded');

    await api.configureAdvisor({ enabled: true, unloadAfter: 'oneMinute', logQuestions: true });
    timers.advance(1);

    expect((await api.advisorStatus()).state).toBe('onDisk');
  });

  it('never unloads by itself on windowClose', async () => {
    const { api, timers } = setup({
      state: 'loaded',
      options: { enabled: true, unloadAfter: 'windowClose', logQuestions: true },
    });

    timers.advance(24 * 60 * 60_000);

    expect((await api.advisorStatus()).state).toBe('loaded');
    expect(timers.count).toBe(0);
  });

  it('forgets the wait when the model is unloaded by hand', async () => {
    const { api, timers, events } = setup({ state: 'loaded' });

    await api.unloadModel();
    events.length = 0;

    expect(timers.count).toBe(0);

    timers.advance(10 * 60_000);

    expect(events).toEqual([]);
  });
});

describe('advisor mock: watcher', () => {
  it('replaces the old watcher with the new one', async () => {
    const { api, timers } = setup({ state: 'disk' });
    const first: AdvisorStatusDto[] = [];
    const second: AdvisorStatusDto[] = [];

    await api.watchAdvisor((status) => first.push(status));
    await api.watchAdvisor((status) => second.push(status));

    await load(api, timers);

    expect(first.map((status) => status.state)).toEqual(['onDisk']);
    expect(second.map((status) => status.state)).toEqual(['onDisk', 'loading', 'loaded']);
  });

  it('pushes the current status on subscription, so nothing between reading and watching is lost', async () => {
    const { api } = setup({ state: 'disk' });
    const seen: AdvisorStatusDto[] = [];

    await api.watchAdvisor((status) => seen.push(status));

    expect(seen.map((status) => status.state)).toEqual(['onDisk']);
  });
});

describe('setMockAdvisorState', () => {
  it('puts the latest mock into each state and pushes it', async () => {
    const { api, events } = setup();

    setMockAdvisorState('disk');
    expect((await api.advisorStatus()).state).toBe('onDisk');

    setMockAdvisorState('loaded');
    expect(await api.advisorStatus()).toMatchObject({ state: 'loaded', ramBytes: 2 * GIB });

    setMockAdvisorState('none');
    expect(await api.advisorStatus()).toMatchObject({ state: 'notDownloaded' });
    expect((await api.advisorStatus()).download).toBeUndefined();

    expect(events.map((event) => event.state)).toEqual(['onDisk', 'loaded', 'notDownloaded']);
  });

  it('holds a download at 40% until it is cancelled, which keeps the bytes', async () => {
    const { api, timers } = setup();

    setMockAdvisorState('downloading');

    const held = (await api.advisorStatus()).download;

    expect(held?.received).toBe(Math.floor(MODEL_BYTES * 0.4));

    timers.advance(DOWNLOAD_MS);

    expect(await api.advisorStatus()).toMatchObject({ state: 'downloading', download: held });

    await api.cancelModelDownload();

    expect(await api.advisorStatus()).toMatchObject({ state: 'notDownloaded', download: held });
  });

  it('drops what was running: a download in progress does not finish into the new state', async () => {
    const { api, timers } = setup();

    await api.startModelDownload();
    setMockAdvisorState('none');
    timers.advance(DOWNLOAD_MS);

    expect((await api.advisorStatus()).state).toBe('notDownloaded');
  });

  it('can be the starting state of a mock', async () => {
    const { api } = setup({ state: 'loaded' });

    expect((await api.advisorStatus()).state).toBe('loaded');
  });
});

describe('advisor mock: clear_question_log', () => {
  it('is fine when there is no log', async () => {
    const { api } = setup();

    await expect(api.clearQuestionLog()).resolves.toBeUndefined();
    await expect(api.clearQuestionLog()).resolves.toBeUndefined();
  });

  it('fails with io when something that is not a file is at the log path', async () => {
    const { api } = setup({ questionLogBlocked: true });
    const error = await rejection(api.clearQuestionLog());

    expect(isApiError(error) && error.code).toBe('io');
  });
});
