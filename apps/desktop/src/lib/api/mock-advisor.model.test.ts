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
    expect(status.options).toEqual({
      enabled: true,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: true,
    });
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
      useGpu: true,
    });

    expect(status.state).toBe('downloading');
    expect(status.options.unloadAfter).toBe('oneMinute');

    timers.advance(DOWNLOAD_MS);

    expect((await api.advisorStatus()).state).toBe('onDisk');
  });
});

describe('advisor mock: where the model runs', () => {
  const options = { enabled: true, unloadAfter: 'fiveMinutes', logQuestions: true } as const;

  it('runs a loaded model on the graphics card, with its name and video memory, and says nothing before it is loaded', async () => {
    const { api } = setup({ state: 'disk' });
    const before = await api.advisorStatus();

    expect(before.gpuSupported).toBe(true);
    expect(before.device).toBeUndefined();
    expect(before.videoBytes).toBeUndefined();

    const { api: running } = setup({ state: 'loaded' });
    const loaded = await running.advisorStatus();

    expect(loaded.device).toEqual({ kind: 'gpu', name: 'AMD Radeon RX 7900 XTX (RADV NAVI31)' });
    expect(loaded.videoBytes).toBe(3_200_000_000);
    expect(loaded.problem).toBeUndefined();
  });

  it('runs on the processor, with no video memory, when the graphics card is switched off', async () => {
    const { api } = setup({ state: 'loaded' });
    const status = await api.configureAdvisor({ ...options, useGpu: true });

    expect(status.device?.kind).toBe('gpu');

    // As the backend: a change of device unloads the model; the next load runs on the processor.
    const unloaded = await api.configureAdvisor({ ...options, useGpu: false });

    expect(unloaded.state).toBe('onDisk');
    expect(unloaded.device).toBeUndefined();

    const { api: cpu } = setup({ state: 'loaded', options: { ...options, useGpu: false } });
    const loaded = await cpu.advisorStatus();

    expect(loaded.device).toEqual({ kind: 'cpu' });
    expect(loaded.videoBytes).toBeUndefined();
    expect(loaded.problem).toBeUndefined();
  });

  it('does not unload the model for a change that is not about the graphics card, and takes a missing useGpu as on', async () => {
    const { api } = setup({ state: 'loaded' });

    expect((await api.configureAdvisor({ ...options, useGpu: true })).state).toBe('loaded');
    expect((await api.configureAdvisor({ ...options, unloadAfter: 'oneMinute', useGpu: true })).state).toBe(
      'loaded',
    );
    expect((await api.configureAdvisor(options as never)).options.useGpu).toBe(true);
  });

  it('rejects a useGpu that is not a boolean', async () => {
    const { api } = setup();

    await expect(api.configureAdvisor({ ...options, useGpu: 'yes' as never })).rejects.toMatchObject({
      code: 'invalid',
    });
  });

  it('falls back to the processor and says so while the card is on, and not once it is switched off', async () => {
    const { api } = setup({ state: 'loaded', gpu: 'fallback' });
    const loaded = await api.configureAdvisor({ ...options, useGpu: true });

    expect(loaded.device).toEqual({ kind: 'cpu' });
    expect(loaded.videoBytes).toBeUndefined();
    expect(loaded.problem).toBe('The graphics card could not run the model, so it runs on the processor.');

    await api.configureAdvisor({ ...options, useGpu: false });

    const { api: off } = setup({ state: 'loaded', gpu: 'fallback', options: { ...options, useGpu: false } });

    expect((await off.advisorStatus()).problem).toBeUndefined();
  });

  it('is a build without a graphics card on request: not supported, the model on the processor, no problem', async () => {
    const { api } = setup({ state: 'loaded', gpu: 'none' });
    const status = await api.advisorStatus();

    expect(status.gpuSupported).toBe(false);
    expect(status.device).toEqual({ kind: 'cpu' });
    expect(status.problem).toBeUndefined();
  });
});
