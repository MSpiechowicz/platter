import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type Api } from '../api/client';
import type { UpdateCheck, UpdateOffer, UpdateProgress } from '../api/types';
import { platform } from './platform.svelte';
import { settings, type UpdateSchedule } from './settings.svelte';
import {
  DUE_CHECK_INTERVAL_MS,
  FIRST_CHECK_DELAY_MS,
  UPDATE_CHECKED_KEY,
  UpdaterStore,
} from './update.svelte';

const HOUR = 3_600_000;
const MINUTE = 60_000;
const START = Date.UTC(2026, 10, 17, 12);

const OFFER: UpdateOffer = {
  version: '0.2.0',
  notes: ['Faster scans'],
  sizeBytes: 14_000_000,
  canInstall: true,
};

class MemoryStorage {
  data = new Map<string, string>();

  getItem(key: string): string | null {
    return this.data.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.data.set(key, value);
  }
}

interface Setup {
  store: UpdaterStore;
  api: { checkUpdate: ReturnType<typeof vi.fn>; installUpdate: ReturnType<typeof vi.fn> } & Pick<
    Api,
    'restartApp' | 'openExternal'
  >;
  toasts: { title: string; description: string; kind?: string }[];
  storage: MemoryStorage;
  clock: { now: number };
  schedule: { value: UpdateSchedule };
  auto: { value: boolean };
}

function setup(update: UpdateOffer | null = OFFER): Setup {
  const storage = new MemoryStorage();
  const clock = { now: START };
  const schedule = { value: 'daily' as UpdateSchedule };
  const auto = { value: false };
  const toasts: Setup['toasts'] = [];
  const api = {
    checkUpdate: vi.fn(async (): Promise<UpdateCheck> => ({ current: '0.1.0', update })),
    installUpdate: vi.fn(async () => {}),
    restartApp: vi.fn(async () => {}),
    openExternal: vi.fn(async () => {}),
  };
  const store = new UpdaterStore({
    api: api as unknown as Api,
    ui: {
      toast: (title, description, kind) => {
        toasts.push({ title, description: description ?? '', kind });

        return toasts.length;
      },
    },
    storage,
    schedule: () => schedule.value,
    autoInstall: () => auto.value,
    now: () => clock.now,
  });

  return { store, api, toasts, storage, clock, schedule, auto };
}

describe('UpdaterStore: checking', () => {
  it('fills the notice with the offer: versions, size in the chosen units, notes and "just now"', async () => {
    const { store } = setup();

    await store.check({ manual: true });

    expect(store.info).toEqual({
      current: '0.1.0',
      next: '0.2.0',
      size: '14 MB',
      notes: ['Faster scans'],
      checked: 'just now',
      canInstall: true,
    });
    expect(store.result).toEqual({ kind: 'found', version: '0.2.0' });
    expect(store.current).toBe('0.1.0');
    expect(store.canInstall).toBe(true);
  });

  it('leaves the size empty when the release does not say', async () => {
    const { store } = setup({ ...OFFER, sizeBytes: null });

    await store.check();

    expect(store.info?.size).toBe('');
  });

  it('records that Platter is up to date, shows no notice, and remembers when it looked', async () => {
    const { store, storage } = setup(null);

    await store.check({ manual: true });

    expect(store.info).toBeNull();
    expect(store.result).toEqual({ kind: 'upToDate' });
    expect(store.current).toBe('0.1.0');
    expect(storage.getItem(UPDATE_CHECKED_KEY)).toBe(String(START));
    expect(store.checkedAt).toBe(START);
  });

  it('puts the error of a check the user asked for in the result', async () => {
    const { store, api, toasts } = setup();

    api.checkUpdate.mockRejectedValue(new ApiRequestError('io', 'Updates could not be checked: offline'));
    await store.check({ manual: true });

    expect(store.result).toEqual({ kind: 'error', message: 'Updates could not be checked: offline' });
    expect(store.info).toBeNull();
    expect(toasts).toEqual([]);
  });

  it('says nothing about an automatic check that fails, and keeps the last result', async () => {
    const { store, api, toasts, storage, clock } = setup(null);

    await store.check({ manual: true });
    clock.now += HOUR;
    api.checkUpdate.mockRejectedValue(new ApiRequestError('io', 'offline'));
    await store.check({ manual: false });

    expect(store.result).toEqual({ kind: 'upToDate' });
    expect(toasts).toEqual([]);
    expect(store.checking).toBe(false);
    // A failed check is not remembered as a check, so the next due look tries again.
    expect(storage.getItem(UPDATE_CHECKED_KEY)).toBe(String(START));
  });

  it('runs one check at a time', async () => {
    const { store, api } = setup();

    await Promise.all([store.check(), store.check()]);

    expect(api.checkUpdate).toHaveBeenCalledTimes(1);
  });

  it('hides the notice when a later check finds nothing', async () => {
    const { store, api } = setup();

    await store.check();
    api.checkUpdate.mockResolvedValue({ current: '0.2.0', update: null });
    await store.check();

    expect(store.info).toBeNull();
  });

  it('shows the notice again at the next check after "Later"', async () => {
    const { store } = setup();

    await store.check();
    store.later();

    expect(store.info).toBeNull();

    await store.check();

    expect(store.info?.next).toBe('0.2.0');
  });
});

describe('UpdaterStore: installing', () => {
  it('shows the download as a fraction, then "installing" without a measure, then ready', async () => {
    const { store, api } = setup();
    const seen: { phase: string; progress: number; indeterminate: boolean }[] = [];
    let finish: () => void = () => {};

    api.installUpdate.mockImplementation(
      (onProgress: (progress: UpdateProgress) => void) =>
        new Promise<void>((resolve) => {
          finish = () => resolve();
          const note = () =>
            seen.push({ phase: store.phase, progress: store.progress, indeterminate: store.indeterminate });

          onProgress({ kind: 'downloading', received: 3_500_000, total: 14_000_000 });
          note();
          onProgress({ kind: 'downloading', received: 14_000_000, total: 14_000_000 });
          note();
          onProgress({ kind: 'installing' });
          note();
        }),
    );

    await store.check();

    const done = store.install();

    finish();
    await done;

    expect(seen).toEqual([
      { phase: 'downloading', progress: 0.25, indeterminate: false },
      { phase: 'downloading', progress: 1, indeterminate: false },
      { phase: 'installing', progress: 1, indeterminate: true },
    ]);
    expect(store.phase).toBe('ready');
    expect(store.indeterminate).toBe(false);
  });

  it('has no fraction when the download size is unknown', async () => {
    const { store, api } = setup();

    api.installUpdate.mockImplementation(async (onProgress: (progress: UpdateProgress) => void) => {
      onProgress({ kind: 'downloading', received: 1_000, total: null });

      expect(store.indeterminate).toBe(true);
    });
    await store.check();
    await store.install();

    expect(store.phase).toBe('ready');
  });

  it('goes back to "available" and says so when the install fails', async () => {
    const { store, api, toasts } = setup();

    api.installUpdate.mockRejectedValue(
      new ApiRequestError('io', 'The update could not be downloaded: offline'),
    );
    await store.check();
    await store.install();

    expect(store.phase).toBe('available');
    expect(store.info?.next).toBe('0.2.0');
    expect(toasts).toEqual([
      {
        title: 'The update failed',
        description: 'The update could not be downloaded: offline',
        kind: 'error',
      },
    ]);
  });

  it('does not ask the backend to install what this copy cannot install', async () => {
    const { store, api } = setup({ ...OFFER, canInstall: false });

    await store.check();
    await store.install();

    expect(api.installUpdate).not.toHaveBeenCalled();
    expect(store.phase).toBe('available');
    expect(store.canInstall).toBe(false);
  });

  it('keeps the notice for the installed update when a newer check runs, and does not check while installing', async () => {
    const { store, api } = setup();

    await store.check();
    await store.install();
    api.checkUpdate.mockResolvedValue({ current: '0.1.0', update: { ...OFFER, version: '0.3.0' } });
    await store.check();

    expect(store.phase).toBe('ready');
    expect(store.info?.next).toBe('0.2.0');
    expect(store.result).toEqual({ kind: 'found', version: '0.3.0' });
  });

  it('restarts through the backend, and tells when that fails', async () => {
    const { store, api, toasts } = setup();

    await store.restart();

    expect(api.restartApp).toHaveBeenCalledTimes(1);

    (api.restartApp as ReturnType<typeof vi.fn>).mockRejectedValue(new ApiRequestError('io', 'no'));
    await store.restart();

    expect(toasts.map((toast) => toast.title)).toEqual(['Platter could not restart']);
  });

  it('opens the release page for an update a package manager installs', async () => {
    const { store, api } = setup({ ...OFFER, canInstall: false });

    await store.openReleases();

    expect(api.openExternal).toHaveBeenCalledWith('releases');
  });
});

describe('UpdaterStore: the schedule', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('checks 10 seconds after start when nothing was checked before', async () => {
    const { store, api } = setup();

    store.start();
    await vi.advanceTimersByTimeAsync(FIRST_CHECK_DELAY_MS - 1);

    expect(api.checkUpdate).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);

    expect(api.checkUpdate).toHaveBeenCalledTimes(1);
    store.stop();
  });

  it('checks at start even when the last check is younger than the interval, then again once that is older', async () => {
    const { store, api, storage, clock } = setup();

    storage.setItem(UPDATE_CHECKED_KEY, String(START - 23 * HOUR));
    store.start();
    await vi.advanceTimersByTimeAsync(FIRST_CHECK_DELAY_MS);

    expect(api.checkUpdate).toHaveBeenCalledTimes(1);

    // The start's check is the last one now: the next is due a day after it.
    clock.now = START + 2 * HOUR;
    await vi.advanceTimersByTimeAsync(DUE_CHECK_INTERVAL_MS);

    expect(api.checkUpdate).toHaveBeenCalledTimes(1);

    clock.now = START + 25 * HOUR;
    await vi.advanceTimersByTimeAsync(DUE_CHECK_INTERVAL_MS);

    expect(api.checkUpdate).toHaveBeenCalledTimes(2);
    store.stop();
  });

  it.each([
    ['sixHours', 5 * HOUR, 7 * HOUR],
    ['daily', 23 * HOUR, 25 * HOUR],
    ['weekly', 6 * 24 * HOUR, 8 * 24 * HOUR],
    ['halfHour', 29 * MINUTE, 31 * MINUTE],
    ['hourly', 59 * MINUTE, 61 * MINUTE],
  ] as const)('%s: waits for its interval', async (name, young, old) => {
    const { store, api, storage, schedule } = setup();

    schedule.value = name;
    storage.setItem(UPDATE_CHECKED_KEY, String(START - young));
    await store.checkIfDue();

    expect(api.checkUpdate).not.toHaveBeenCalled();

    storage.setItem(UPDATE_CHECKED_KEY, String(START - old));
    await store.checkIfDue();

    expect(api.checkUpdate).toHaveBeenCalledTimes(1);
  });

  it('never makes an automatic check with "never", and still checks when asked', async () => {
    const { store, api, schedule } = setup();

    schedule.value = 'never';
    store.start();
    await vi.advanceTimersByTimeAsync(FIRST_CHECK_DELAY_MS + 2 * DUE_CHECK_INTERVAL_MS);

    expect(api.checkUpdate).not.toHaveBeenCalled();

    await store.check({ manual: true });

    expect(api.checkUpdate).toHaveBeenCalledTimes(1);
    store.stop();
  });

  it('treats a last check in the future as due, and a garbled one as never checked', async () => {
    const { store, api, storage } = setup();

    storage.setItem(UPDATE_CHECKED_KEY, String(START + 5 * HOUR));
    await store.checkIfDue();
    storage.setItem(UPDATE_CHECKED_KEY, 'yesterday');
    await store.checkIfDue();

    expect(api.checkUpdate).toHaveBeenCalledTimes(2);
  });

  it('starts once, and stops', async () => {
    const { store, api } = setup();

    store.start();
    store.start();
    await vi.advanceTimersByTimeAsync(FIRST_CHECK_DELAY_MS);

    expect(api.checkUpdate).toHaveBeenCalledTimes(1);

    store.stop();
    api.checkUpdate.mockClear();
    await vi.advanceTimersByTimeAsync(7 * 24 * HOUR);

    expect(api.checkUpdate).not.toHaveBeenCalled();
  });

  it('keeps "checked" in the notice up to date while it stays on screen', async () => {
    const { store, clock } = setup();

    await store.check();
    store.start();
    // The start checks too: "checked" counts from that check.
    await vi.advanceTimersByTimeAsync(FIRST_CHECK_DELAY_MS);
    clock.now += 2 * HOUR;
    await vi.advanceTimersByTimeAsync(DUE_CHECK_INTERVAL_MS);

    expect(store.info?.checked).toBe('2 hours ago');
    store.stop();
  });
});

describe('UpdaterStore: storage that fails', () => {
  it('works without storage, and when storage throws', async () => {
    const throwing = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    };
    const api = { checkUpdate: vi.fn(async () => ({ current: '0.1.0', update: null })) };
    const store = new UpdaterStore({
      api: api as unknown as Api,
      storage: throwing,
      schedule: () => 'daily',
    });

    await store.checkIfDue();

    expect(store.result).toEqual({ kind: 'upToDate' });
    expect(store.checkedAt).toBeTruthy();
  });
});

describe('UpdaterStore: installing automatically', () => {
  it('installs an update a check found by itself when the setting is on, and then waits for the restart', async () => {
    const { store, api, auto } = setup();
    auto.value = true;

    await store.check();
    await vi.waitFor(() => expect(store.phase).toBe('ready'));

    expect(api.installUpdate).toHaveBeenCalledTimes(1);
    expect(api.restartApp).not.toHaveBeenCalled();
  });

  it('leaves the install to the user when the setting is off', async () => {
    const { store, api } = setup();

    await store.check();

    expect(api.installUpdate).not.toHaveBeenCalled();
    expect(store.phase).toBe('available');
  });

  it('follows the Settings switch on Linux and on Windows, where installing only downloads', async () => {
    const own = (): { store: UpdaterStore; install: ReturnType<typeof vi.fn> } => {
      const install = vi.fn(async () => {});
      const api = {
        checkUpdate: vi.fn(async (): Promise<UpdateCheck> => ({ current: '0.1.0', update: OFFER })),
        installUpdate: install,
        restartApp: vi.fn(async () => {}),
        openExternal: vi.fn(async () => {}),
      };

      return {
        store: new UpdaterStore({ api: api as unknown as Api, storage: new MemoryStorage() }),
        install,
      };
    };

    settings.set('autoInstallUpdates', true);

    const linux = own();

    await linux.store.check();
    await vi.waitFor(() => expect(linux.install).toHaveBeenCalledTimes(1));

    platform.os = 'windows';

    try {
      const windows = own();

      await windows.store.check();

      // On Windows `installUpdate` only downloads and checks the setup. It runs on the restart click.
      await vi.waitFor(() => expect(windows.install).toHaveBeenCalledTimes(1));
    } finally {
      platform.os = 'linux';
    }
  });

  it('does not install what a package manager installs', async () => {
    const { store, api, auto } = setup({ ...OFFER, canInstall: false });
    auto.value = true;

    await store.check();

    expect(api.installUpdate).not.toHaveBeenCalled();
  });

  it('says nothing when an automatic install fails, and offers the update again', async () => {
    const { store, api, toasts, auto } = setup();
    auto.value = true;
    api.installUpdate.mockRejectedValueOnce({ code: 'io', message: 'The update could not be downloaded.' });

    await store.check();
    await vi.waitFor(() => expect(api.installUpdate).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(store.phase).toBe('available'));

    expect(toasts).toEqual([]);
    expect(store.info?.next).toBe(OFFER.version);
  });

  it('starts one install only when the user clicks Install during an automatic one', async () => {
    const { store, api, auto } = setup();
    let finish: () => void = () => {};
    api.installUpdate.mockImplementation(() => new Promise<void>((resolve) => (finish = resolve)));
    auto.value = true;

    await store.check();
    await store.install();
    finish();
    await vi.waitFor(() => expect(store.phase).toBe('ready'));

    expect(api.installUpdate).toHaveBeenCalledTimes(1);
  });

  it('installs nothing until the next check when the setting is turned on later', async () => {
    const { store, api, auto } = setup();

    await store.check();
    auto.value = true;

    expect(api.installUpdate).not.toHaveBeenCalled();

    await store.check();
    await vi.waitFor(() => expect(api.installUpdate).toHaveBeenCalledTimes(1));
  });

  it('does not install after a check that failed', async () => {
    const { store, api, auto } = setup();
    auto.value = true;
    api.checkUpdate.mockRejectedValueOnce({ code: 'io', message: 'offline' });

    await store.check();

    expect(api.installUpdate).not.toHaveBeenCalled();
  });
});
