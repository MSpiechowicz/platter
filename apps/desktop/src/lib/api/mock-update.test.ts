import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { UpdateProgress } from './types';
import { createMockUpdate } from './mock-update';

describe('the mock updater', () => {
  it('finds no update by default, and refuses to install one', async () => {
    const update = createMockUpdate();

    expect(await update.checkUpdate()).toEqual({ current: '0.1.0', update: null });
    await expect(update.installUpdate(() => {})).rejects.toMatchObject({ code: 'invalid' });
  });

  it('offers the demo update that this copy installs itself, with the fields it is given changed', async () => {
    const found = await createMockUpdate({ offer: {}, current: '0.1.2' }).checkUpdate();

    expect(found.current).toBe('0.1.2');
    expect(found.update).toMatchObject({ version: '0.2.0', sizeBytes: 14_000_000, canInstall: true });
    expect(found.update?.notes).toHaveLength(3);

    const other = await createMockUpdate({ offer: { version: '1.0.0', sizeBytes: null } }).checkUpdate();

    expect(other.update).toMatchObject({ version: '1.0.0', sizeBytes: null, canInstall: true });
  });

  it('refuses to install an update before a check has found one', async () => {
    const update = createMockUpdate({ offer: {}, downloadMs: 0 });

    await expect(update.installUpdate(() => {})).rejects.toMatchObject({ code: 'invalid' });
  });

  it('refuses to install an update that a package manager installs', async () => {
    const update = createMockUpdate({ offer: { canInstall: false }, downloadMs: 0 });

    await update.checkUpdate();

    await expect(update.installUpdate(() => {})).rejects.toMatchObject({
      code: 'invalid',
      message: expect.stringContaining('package manager'),
    });
  });

  it('downloads in steps that end with everything received, then installs, and needs a new check for a second install', async () => {
    const update = createMockUpdate({ offer: {}, downloadMs: 0 });
    const events: UpdateProgress[] = [];

    await update.checkUpdate();
    await update.installUpdate((progress) => events.push(progress));

    expect(events).toEqual([
      { kind: 'downloading', received: 14_000_000, total: 14_000_000 },
      { kind: 'installing' },
    ]);
    await expect(update.installUpdate(() => {})).rejects.toMatchObject({ code: 'invalid' });
  });

  describe('with a download that takes time', () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it('takes about as long as asked and reports a growing number of bytes', async () => {
      const update = createMockUpdate({ offer: {}, downloadMs: 2000, tickMs: 50 });
      const received: number[] = [];
      let done = false;

      await update.checkUpdate();

      const installing = update
        .installUpdate((progress) => {
          if (progress.kind === 'downloading') {
            received.push(progress.received);
          }
        })
        .then(() => (done = true));

      await vi.advanceTimersByTimeAsync(1000);
      expect(done).toBe(false);
      expect(received.at(-1)).toBeGreaterThan(0);
      expect(received.at(-1)).toBeLessThan(14_000_000);

      await vi.advanceTimersByTimeAsync(1500);
      await installing;

      expect(received).toEqual([...received].sort((a, b) => a - b));
      expect(received.at(-1)).toBe(14_000_000);
    });

    it('reports no total when the release does not say how big it is', async () => {
      const update = createMockUpdate({ offer: { sizeBytes: null }, downloadMs: 100 });
      const totals = new Set<number | null>();

      await update.checkUpdate();

      const installing = update.installUpdate((progress) => {
        if (progress.kind === 'downloading') {
          totals.add(progress.total);
        }
      });

      await vi.advanceTimersByTimeAsync(500);
      await installing;

      expect([...totals]).toEqual([null]);
    });
  });
});
