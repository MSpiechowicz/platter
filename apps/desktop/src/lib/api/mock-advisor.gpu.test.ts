import { describe, expect, it } from 'vitest';
import type { Api } from './client';
import { createMockApi } from './mock';
import type { MockAdvisorOptions } from './mock-advisor';
import { FakeTimers } from './mock-advisor.testkit';

const options = { enabled: true, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true } as const;

const LOAD_MS = 400;

function setup(advisor: MockAdvisorOptions = {}): { api: Api; timers: FakeTimers } {
  const timers = new FakeTimers();
  const api = createMockApi({ durationMs: 0, advisor: { timers, ...advisor } });

  return { api, timers };
}

const pending = (what: 'crash' | 'hang' | 'error' = 'crash'): MockAdvisorOptions => ({
  state: 'loaded',
  gpuIncident: { what, pending: true },
});

describe('advisor mock: graphics card problems', () => {
  it('has no problem unless one is asked for', async () => {
    const { api } = setup({ state: 'loaded' });
    const status = await api.advisorStatus();

    expect(status.gpuProblem).toBeUndefined();
    expect(status.lastGpuProblem).toBeUndefined();
  });

  it('starts with a problem that waits: it is also the last one, and the model runs on the processor', async () => {
    const { api } = setup(pending('hang'));
    const status = await api.advisorStatus();

    expect(status.gpuProblem).toMatchObject({ what: 'hang', card: 'AMD Radeon RX 7900 XTX (RADV NAVI31)' });
    expect(status.gpuProblem?.id).toBeGreaterThan(0);
    expect(status.gpuProblem?.at).toBeGreaterThan(0);
    expect(status.lastGpuProblem).toEqual(status.gpuProblem);
    expect(status.device).toEqual({ kind: 'cpu' });
    expect(status.videoBytes).toBeUndefined();
    expect(status.problem).toBe('The graphics card could not run the model; it runs on the processor.');
  });

  it('starts with a last problem in the log and none waiting, the model on the card', async () => {
    const { api } = setup({ state: 'loaded', gpuIncident: { what: 'crash', pending: false } });
    const status = await api.advisorStatus();

    expect(status.gpuProblem).toBeUndefined();
    expect(status.lastGpuProblem?.what).toBe('crash');
    expect(status.device?.kind).toBe('gpu');
  });

  it('answers "use the processor": the problem is gone, the log keeps it, and the card stays blocked until the option goes off', async () => {
    const { api } = setup(pending());
    const waiting = (await api.advisorStatus()).gpuProblem;
    const answered = await api.answerGpuProblem(waiting?.id as number, 'useCpu');

    expect(answered.gpuProblem).toBeUndefined();
    expect(answered.lastGpuProblem).toEqual(waiting);
    expect(answered.device).toEqual({ kind: 'cpu' });

    // As the web view does next: the option goes off, and a model on the processor is not unloaded for it.
    const configured = await api.configureAdvisor({ ...options, useGpu: false });

    expect(configured.state).toBe('loaded');
    expect(configured.device).toEqual({ kind: 'cpu' });
    expect(configured.problem).toBeUndefined();
  });

  it('answers "try again": the model that is on the processor stays there, and the next load is on the card', async () => {
    const { api, timers } = setup(pending('error'));
    const waiting = (await api.advisorStatus()).gpuProblem;
    const answered = await api.answerGpuProblem(waiting?.id as number, 'tryAgain');

    expect(answered.gpuProblem).toBeUndefined();
    expect(answered.state).toBe('loaded');
    expect(answered.device).toEqual({ kind: 'cpu' });

    expect((await api.unloadModel()).state).toBe('onDisk');

    const loading = api.loadModel();

    timers.advance(LOAD_MS);

    expect((await loading).device?.kind).toBe('gpu');
  });

  it('refuses an id that is not the waiting problem, a second answer, and a choice that is not one of the two', async () => {
    const { api } = setup(pending());
    const id = (await api.advisorStatus()).gpuProblem?.id as number;

    await expect(api.answerGpuProblem(id + 1, 'useCpu')).rejects.toMatchObject({ code: 'invalid' });
    await expect(api.answerGpuProblem(id, 'maybe' as never)).rejects.toMatchObject({ code: 'invalid' });
    expect((await api.advisorStatus()).gpuProblem?.id).toBe(id);

    await api.answerGpuProblem(id, 'tryAgain');

    await expect(api.answerGpuProblem(id, 'tryAgain')).rejects.toMatchObject({ code: 'invalid' });
  });

  it('refuses an answer when nothing waits', async () => {
    const { api } = setup({ state: 'loaded', gpuIncident: { what: 'crash', pending: false } });
    const id = (await api.advisorStatus()).lastGpuProblem?.id as number;

    await expect(api.answerGpuProblem(id, 'useCpu')).rejects.toMatchObject({ code: 'invalid' });
  });

  it('answers a waiting problem "use the processor" by itself when the card is switched off', async () => {
    const { api } = setup(pending());
    const status = await api.configureAdvisor({ ...options, useGpu: false });

    expect(status.gpuProblem).toBeUndefined();
    expect(status.lastGpuProblem).toBeDefined();
    expect(status.state).toBe('loaded');
  });

  it('clears the last problem and keeps one that still waits; clearing nothing is fine', async () => {
    const { api } = setup(pending());

    await api.clearGpuLog();

    const status = await api.advisorStatus();

    expect(status.lastGpuProblem).toBeUndefined();
    expect(status.gpuProblem).toBeDefined();

    await expect(api.clearGpuLog()).resolves.toBeUndefined();
  });

  it('pushes the status when a problem is answered or the log is cleared', async () => {
    const { api } = setup(pending());
    const pushed: boolean[] = [];

    await api.watchAdvisor((status) => pushed.push(status.gpuProblem !== undefined));

    const id = (await api.advisorStatus()).gpuProblem?.id as number;

    await api.answerGpuProblem(id, 'tryAgain');
    await api.clearGpuLog();

    expect(pushed).toEqual([true, false, false]);
  });
});
