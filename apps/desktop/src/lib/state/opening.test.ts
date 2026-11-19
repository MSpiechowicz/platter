import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  LAST_BEAT_MS,
  opening,
  READY_GRACE_MS,
  SLOW_NOTE,
  STEP_LIMIT_MS,
  STEP_MIN_MS,
  type OpeningTasks,
} from './opening.svelte';
import { settings } from './settings.svelte';

const NOW = 1_750_000_000;
const HOUR = 3600;

function tasks(over: Partial<OpeningTasks> = {}): OpeningTasks {
  return {
    disks: async () => 3,
    lastScan: async () => ({ bytes: 983_000_000_000, scannedAt: NOW - 2 * HOUR - 60, outdated: false }),
    ...over,
  };
}

/** Resolves after `ms` of fake time. */
function later<T>(ms: number, value: T): Promise<T> {
  return new Promise((resolve) => setTimeout(() => resolve(value), ms));
}

const status = (index: number): string => opening.steps[index]?.status ?? '';
const note = (index: number): string | undefined => opening.steps[index]?.note;

describe('the opening steps', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(NOW * 1000));
    settings.set('units', 'GB');
    settings.reduceMotion = false;
    opening.reset('opening');
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('waits with all three steps until it starts', () => {
    expect(opening.steps.map((step) => step.status)).toEqual(['waiting', 'waiting', 'waiting']);
    expect(opening.steps.map((step) => step.label)).toEqual([
      'Checking your disks',
      'Looking for your last scan',
      'Checking it is still current',
    ]);
  });

  it('keeps every step running for at least a second, one after the other, even when the work is instant', async () => {
    const running = opening.run(tasks());

    await vi.advanceTimersByTimeAsync(0);
    expect([status(0), status(1), status(2)]).toEqual(['running', 'waiting', 'waiting']);

    await vi.advanceTimersByTimeAsync(STEP_MIN_MS - 1);
    expect([status(0), status(1)]).toEqual(['running', 'waiting']);

    await vi.advanceTimersByTimeAsync(1);
    expect([status(0), status(1), status(2)]).toEqual(['done', 'running', 'waiting']);
    expect(note(0)).toBe('3 disks');

    await vi.advanceTimersByTimeAsync(STEP_MIN_MS - 1);
    expect(status(1)).toBe('running');

    await vi.advanceTimersByTimeAsync(1);
    expect([status(1), status(2)]).toEqual(['done', 'running']);

    await vi.advanceTimersByTimeAsync(STEP_MIN_MS);
    expect(status(2)).toBe('done');

    await vi.advanceTimersByTimeAsync(LAST_BEAT_MS);
    await running;
    expect(opening.phase).toBe('landing');
  });

  it('keeps a step that really takes longer running until it is finished', async () => {
    const running = opening.run(tasks({ lastScan: () => later(4000, null) }));

    await vi.advanceTimersByTimeAsync(STEP_MIN_MS + 3900);
    expect(status(1)).toBe('running');

    await vi.advanceTimersByTimeAsync(100);
    expect(status(1)).toBe('done');
    expect(status(2)).toBe('skipped');

    await vi.advanceTimersByTimeAsync(LAST_BEAT_MS);
    await running;
  });

  it('says what the saved scan holds and how old it is, and that it is up to date', async () => {
    const running = opening.run(tasks());

    await vi.advanceTimersByTimeAsync(3 * STEP_MIN_MS);

    expect(note(1)).toBe('983 GB · 2 hours ago');
    expect(note(2)).toBe('up to date');
    expect(opening.steps[2]?.warn).toBe(false);

    await vi.advanceTimersByTimeAsync(2 * LAST_BEAT_MS);
    await running;
  });

  it('warns that an outdated scan needs a rescan', async () => {
    const running = opening.run(
      tasks({ lastScan: async () => ({ bytes: 5e9, scannedAt: NOW - 60, outdated: true }) }),
    );

    await vi.advanceTimersByTimeAsync(3 * STEP_MIN_MS);

    expect(note(2)).toBe('outdated, rescan to refresh');
    expect(opening.steps[2]?.warn).toBe(true);

    await vi.advanceTimersByTimeAsync(2 * LAST_BEAT_MS);
    await running;
  });

  it('counts "1 disk" in the singular', async () => {
    const running = opening.run(tasks({ disks: async () => 1 }));

    await vi.advanceTimersByTimeAsync(STEP_MIN_MS);
    expect(note(0)).toBe('1 disk');

    await vi.advanceTimersByTimeAsync(5000);
    await running;
  });

  it('skips the third step when there is no saved scan: no running, no tick', async () => {
    const running = opening.run(tasks({ lastScan: async () => null }));

    await vi.advanceTimersByTimeAsync(2 * STEP_MIN_MS);

    expect(note(1)).toBe('none kept');
    expect([status(0), status(1), status(2)]).toEqual(['done', 'done', 'skipped']);
    expect(note(2)).toBeUndefined();

    await vi.advanceTimersByTimeAsync(2 * LAST_BEAT_MS);
    await running;
  });

  it('starts the rescan in the third step when "Rescan when Platter opens" asks for it, and says so', async () => {
    const rescan = vi.fn().mockResolvedValue(undefined);
    const running = opening.run(tasks({ rescan }));

    await vi.advanceTimersByTimeAsync(2 * STEP_MIN_MS);
    expect(rescan).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(STEP_MIN_MS);
    expect(note(2)).toBe('rescanning');

    await vi.advanceTimersByTimeAsync(LAST_BEAT_MS);
    await running;
    expect(opening.phase).toBe('landing');
  });

  it('does not rescan when there is no saved scan', async () => {
    const rescan = vi.fn().mockResolvedValue(undefined);
    const running = opening.run(tasks({ lastScan: async () => null, rescan }));

    await vi.advanceTimersByTimeAsync(10_000);
    await running;

    expect(rescan).not.toHaveBeenCalled();
  });

  it('lands even when steps fail: no disks, no saved scan, and the start screen is what shows', async () => {
    const running = opening.run({
      disks: () => Promise.reject(new Error('no disks')),
      lastScan: () => Promise.reject(new Error('unreadable')),
    });

    await vi.advanceTimersByTimeAsync(10_000);
    await running;

    expect([status(0), status(1), status(2)]).toEqual(['done', 'done', 'skipped']);
    expect(note(0)).toBeUndefined();
    expect(opening.phase).toBe('landing');
  });

  it('lands even when the rescan fails', async () => {
    const running = opening.run(tasks({ rescan: () => Promise.reject(new Error('busy')) }));

    await vi.advanceTimersByTimeAsync(10_000);
    await running;

    expect(status(2)).toBe('done');
    expect(opening.phase).toBe('landing');
  });

  it('waits a beat after the last step, and until the logo is drawn, before it lands', async () => {
    // Two steps: 2000 ms of steps and a beat of 500 ms. The logo needs 2000 ms, so the beat is what is waited for.
    const running = opening.run(tasks({ lastScan: async () => null }));

    await vi.advanceTimersByTimeAsync(2 * STEP_MIN_MS + LAST_BEAT_MS - 1);
    expect(opening.phase).toBe('opening');

    await vi.advanceTimersByTimeAsync(1);
    await running;
    expect(opening.phase).toBe('landing');
  });

  it('waits for the background start-up before it lands, but not for long', async () => {
    const stuck = new Promise<void>(() => {});
    const running = opening.run(tasks({ ready: stuck }));

    await vi.advanceTimersByTimeAsync(3 * STEP_MIN_MS + LAST_BEAT_MS + READY_GRACE_MS - 1);
    expect(opening.phase).toBe('opening');

    await vi.advanceTimersByTimeAsync(1);
    await running;
    expect(opening.phase).toBe('landing');
  });

  it('goes straight to the app with reduced motion, after the steps have shown', async () => {
    settings.reduceMotion = true;

    const running = opening.run(tasks({ lastScan: async () => null }));

    await vi.advanceTimersByTimeAsync(2 * STEP_MIN_MS);
    expect(status(1)).toBe('done');
    expect(opening.phase).toBe('opening');

    await vi.advanceTimersByTimeAsync(LAST_BEAT_MS);
    await running;
    expect(opening.phase).toBe('done');
  });

  it('waits for the background start-up with reduced motion too, but not for long', async () => {
    settings.reduceMotion = true;

    const running = opening.run({
      ...tasks({ lastScan: async () => null }),
      ready: new Promise(() => undefined),
    });

    await vi.advanceTimersByTimeAsync(2 * STEP_MIN_MS + LAST_BEAT_MS);
    expect(opening.phase).toBe('opening');

    await vi.advanceTimersByTimeAsync(READY_GRACE_MS);
    await running;
    expect(opening.phase).toBe('done');
  });

  it('opens the app when a step never ends (a saved scan of a network folder whose server is gone)', async () => {
    const running = opening.run(tasks({ lastScan: () => new Promise(() => undefined) }));

    await vi.advanceTimersByTimeAsync(STEP_MIN_MS + STEP_LIMIT_MS - 1);
    expect(status(1)).toBe('running');

    await vi.advanceTimersByTimeAsync(1);
    expect(status(1)).toBe('done');
    expect(note(1)).toBe(SLOW_NOTE);
    expect(opening.steps[1]?.warn).toBe(true);
    // Without a scan that opened, there is nothing to check: the start screen is what lands.
    expect(status(2)).toBe('skipped');

    await vi.advanceTimersByTimeAsync(LAST_BEAT_MS);
    await running;
    expect(opening.phase).toBe('landing');
  });

  it('opens the app when the rescan never starts', async () => {
    settings.rescanOnOpen = true;

    try {
      const running = opening.run({ ...tasks(), rescan: () => new Promise(() => undefined) });

      await vi.advanceTimersByTimeAsync(2 * STEP_MIN_MS + STEP_LIMIT_MS + LAST_BEAT_MS);
      await running;
      expect(note(2)).toBe(SLOW_NOTE);
      expect(opening.phase).toBe('landing');
    } finally {
      settings.rescanOnOpen = false;
    }
  });

  it('is over when the landing is finished', () => {
    opening.reset('landing');
    opening.finish();

    expect(opening.phase).toBe('done');
  });
});
