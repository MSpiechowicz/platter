import { describe, expect, it } from 'vitest';
import { createMockApi } from './mock';
import type { MockAdvisorOptions } from './mock-advisor';
import { FakeTimers } from './mock-advisor.testkit';
import type { Api } from './client';

const BYTES_4B = 2_740_937_888;
const DOWNLOAD_MS = 3000;

function setup(advisor: MockAdvisorOptions = {}): { api: Api; timers: FakeTimers } {
  const timers = new FakeTimers();
  const api = createMockApi({ durationMs: 0, advisor: { timers, ...advisor } });

  return { api, timers };
}

describe('advisor mock: the one model', () => {
  it('offers Qwen3.5-4B and no choice of model', async () => {
    const { api } = setup();
    const status = await api.advisorStatus();

    expect(status.model).toEqual({
      name: 'Qwen3.5-4B',
      format: 'GGUF · Q4_K_M',
      bytes: BYTES_4B,
      host: 'huggingface.co',
    });
    expect(status).not.toHaveProperty('models');
    expect(status).not.toHaveProperty('recommended');
    expect(status.options).toEqual({ enabled: true, unloadAfter: 'fiveMinutes', logQuestions: true });
  });

  it('downloads the 4B file, reporting its size while it comes', async () => {
    const { api, timers } = setup();

    await api.startModelDownload();
    timers.advance(1000);

    const during = await api.advisorStatus();

    expect(during.state).toBe('downloading');
    expect(during.download?.total).toBe(BYTES_4B);
    expect(during.download?.received).toBeGreaterThan(0);

    timers.advance(DOWNLOAD_MS);

    expect(await api.advisorStatus()).toMatchObject({ state: 'onDisk', model: { bytes: BYTES_4B } });
  });

  it('takes a change of options while a download runs, and the download goes on', async () => {
    const { api, timers } = setup();

    await api.startModelDownload();
    timers.advance(500);

    const status = await api.configureAdvisor({
      enabled: true,
      unloadAfter: 'oneMinute',
      logQuestions: true,
    });

    expect(status.state).toBe('downloading');
    expect(status.options.unloadAfter).toBe('oneMinute');

    timers.advance(DOWNLOAD_MS);

    expect((await api.advisorStatus()).state).toBe('onDisk');
  });
});
