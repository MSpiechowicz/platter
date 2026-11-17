import { describe, expect, it } from 'vitest';
import { ApiRequestError } from './client';
import { createMockHistory } from './mock-history';

const GB = 1_000_000_000;
// 17 November 2026, midday: the dates in the answers are local days around it.
const NOW = Math.floor(new Date(2026, 10, 17, 12).getTime() / 1000);

const history = (variant?: 'thin' | 'empty' | 'broken' | 'flat') =>
  createMockHistory({ variant, now: () => NOW });

describe('the demo history', () => {
  it('has one sample and one "without" value per day, oldest first, ending today with the disk use', async () => {
    const overview = await history().historyOverview(90);

    expect(overview.samples).toHaveLength(90);
    expect(overview.without).toHaveLength(90);
    expect(overview.today).toBe('2026-11-17');
    expect(overview.samples.at(-1)).toMatchObject({ day: '2026-11-17', used: 1408 * GB });
    expect(overview.samples.map((sample) => sample.day)).toEqual(
      [...overview.samples.map((sample) => sample.day)].sort(),
    );
    expect(overview.volume).toEqual({ id: 'nvme0n1p2', label: 'nvme0n1p2' });
    expect(overview.thin).toBe(false);
  });

  it('shows what the cleanups saved as the gap between the two lines, and counts the trash run as freeing nothing', async () => {
    const overview = await history().historyOverview(90);
    const gap = (overview.without.at(-1) ?? 0) - (overview.samples.at(-1)?.used ?? 0);

    // 74 + 22 + 48 + 12: the 31 GB moved to the trash is not free space yet.
    expect(overview.freed).toBe(156 * GB);
    expect(Math.abs(gap - overview.freed)).toBeLessThan(10);
    expect(overview.cleanupCount).toBe(5);
    expect(overview.cleanups[0]).toMatchObject({ status: 'inTrash', freed: 31 * GB, restorable: 31 * GB });
    expect(overview.restorable).toBe(31 * GB);
  });

  it('lists the cleanups newest first, with the day they were made', async () => {
    const overview = await history().historyOverview(90);

    expect(overview.cleanups.map((cleanup) => cleanup.day)).toEqual([
      '2026-11-12',
      '2026-10-28',
      '2026-10-03',
      '2026-09-14',
      '2026-08-30',
    ]);
    expect(overview.cleanups[2]?.methods).toEqual([{ kind: 'tool', command: 'paccache -rk2' }]);
  });

  it('gives a growth rate near 3 GB a day and the time until the disk is full', async () => {
    const overview = await history().historyOverview(90);

    expect(overview.growthPerDay).toBeGreaterThan(2.5 * GB);
    expect(overview.growthPerDay).toBeLessThan(4 * GB);
    expect(overview.daysUntilFull).toBeCloseTo((1900 * GB - 1408 * GB) / (overview.growthPerDay ?? 1), 5);
  });

  it('keeps to the window: fewer days, fewer samples and only the cleanups inside', async () => {
    const overview = await history().historyOverview(30);

    expect(overview.samples).toHaveLength(30);
    expect(overview.cleanups.map((cleanup) => cleanup.id)).toEqual([5, 4]);
    expect(overview.cleanupCount).toBe(2);
    expect(overview.freed).toBe(74 * GB);
  });

  it('refuses a window of no days or more than 366', async () => {
    for (const days of [0, 367, 1.5]) {
      await expect(history().historyOverview(days)).rejects.toMatchObject({ code: 'invalid' });
    }
  });
});

describe('the other histories', () => {
  it('has three days and no growth rate while it is thin', async () => {
    const overview = await history('thin').historyOverview(90);

    expect(overview.thin).toBe(true);
    expect(overview.samples).toHaveLength(3);
    expect(overview.growthPerDay).toBeUndefined();
    expect(overview.daysUntilFull).toBeUndefined();
    expect(overview.cleanups).toEqual([]);
  });

  it('has nothing at all when it is empty', async () => {
    const overview = await history('empty').historyOverview(90);

    expect(overview).toMatchObject({ samples: [], without: [], cleanups: [], thin: true, freed: 0 });
    expect(overview.volume).toBeUndefined();
    expect(overview.problem).toBeUndefined();
  });

  it('names the problem and has nothing else when the history file cannot be used', async () => {
    const overview = await history('broken').historyOverview(90);

    expect(overview.problem).toMatch(/could not be read/);
    expect(overview.problem).not.toContain(';');
    expect(overview.samples).toEqual([]);
    expect(overview.thin).toBe(true);
  });

  it('has a full window and no growth rate for a disk that does not grow', async () => {
    const overview = await history('flat').historyOverview(90);

    expect(overview.thin).toBe(false);
    expect(overview.samples).toHaveLength(90);
    expect(overview.growthPerDay).toBeUndefined();
    expect(overview.daysUntilFull).toBeUndefined();
  });
});

describe('restoring a cleanup', () => {
  it('turns the run into "restored" and takes its size off what can be restored', async () => {
    const api = history();
    const answer = await api.restoreHistoryCleanup(5);

    expect(answer.restored).toBe(2);
    expect(answer.items.every((item) => item.restored)).toBe(true);

    const overview = await api.historyOverview(90);

    expect(overview.cleanups[0]).toMatchObject({ id: 5, status: 'restored', restorable: 0 });
    expect(overview.restorable).toBe(0);
    // A restore never changes what was saved.
    expect(overview.freed).toBe(156 * GB);
  });

  it('rejects with "cancelled" when the confirmation is declined, and leaves the run in the trash', async () => {
    const api = createMockHistory({ now: () => NOW, confirmRestore: () => false });

    await expect(api.restoreHistoryCleanup(5)).rejects.toMatchObject({ code: 'cancelled' });

    const overview = await api.historyOverview(90);

    expect(overview.cleanups[0]).toMatchObject({ id: 5, status: 'inTrash' });
  });

  it('says notFound the second time, or for a run that is permanent, as the backend does', async () => {
    const api = history();

    await api.restoreHistoryCleanup(5);

    await expect(api.restoreHistoryCleanup(5)).rejects.toMatchObject({ code: 'notFound' });
    await expect(api.restoreHistoryCleanup(4)).rejects.toMatchObject({ code: 'notFound' });
  });

  it('says notFound for a cleanup that is not in the history', async () => {
    const failure = await history()
      .restoreHistoryCleanup(99)
      .catch((reason: unknown) => reason);

    expect(failure).toBeInstanceOf(ApiRequestError);
    expect(failure).toMatchObject({ code: 'notFound' });
  });
});
