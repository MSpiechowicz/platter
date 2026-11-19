import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Each test loads `bootstrap` afresh with its own URL hooks.
const state = vi.hoisted(() => ({ calls: [] as string[] }));

vi.mock('./api', () => ({
  api: {
    listVolumes: vi.fn().mockResolvedValue([]),
  },
}));

vi.mock('./api/mock-advisor', () => ({
  setMockAdvisorState: (value: string) => state.calls.push(`mock:${value}`),
}));

/** Longer than the opening screen takes with all its steps, so a boot with nothing held up has finished by then. */
const OPENING_MS = 10_000;

interface BootOptions {
  restored?: boolean;
  outdated?: boolean;
  rescanOnOpen?: boolean;
  /** The advisor's status channel opens only when this resolves. */
  advisorGate?: Promise<void>;
}

/** A scan as `scan.restore` leaves it when it opened one. */
function readyScan() {
  return {
    status: 'ready' as const,
    options: { root: '/', oneFileSystem: true },
    scanId: 1,
    rootId: 0,
    totals: { allocated: 983_000_000_000, apparent: 0, files: 0, dirs: 0, errors: 0 },
    elapsedMs: 0,
    scannedAt: Math.floor(Date.now() / 1000) - 7200,
    warnings: [],
    outdated: false,
  };
}

/** Loads `bootstrap` with the given URL hooks and spies on what it calls. Nothing runs until `bootstrap()`. */
async function prepare(
  hooks: Record<string, unknown>,
  os?: 'windows' | 'macos',
  { restored = false, outdated = false, rescanOnOpen = false, advisorGate }: BootOptions = {},
) {
  vi.resetModules();
  vi.doMock('./hooks', () => ({
    hooks: {
      view: null,
      mini: false,
      update: null,
      overlays: false,
      root: null,
      advisor: null,
      labels: false,
      ...hooks,
    },
  }));

  const { advisor } = await import('./state/advisor.svelte');
  const { opening } = await import('./state/opening.svelte');
  const { platform } = await import('./state/platform.svelte');
  const { scan } = await import('./state/scan.svelte');
  const { settings } = await import('./state/settings.svelte');
  const { updater } = await import('./state/update.svelte');
  const { bootstrap } = await import('./bootstrap');

  const check = vi.spyOn(updater, 'check').mockResolvedValue();
  const start = vi.spyOn(updater, 'start').mockImplementation(() => {});

  vi.spyOn(advisor, 'configure').mockImplementation(async () => {
    state.calls.push(`configure:${settings.advisorEnabled}`);
  });
  vi.spyOn(advisor, 'watch').mockImplementation(async () => {
    state.calls.push('watch');

    await advisorGate;

    if (os) {
      advisor.status = { os } as typeof advisor.status;
    }
  });

  const startScan = vi.spyOn(scan, 'start').mockResolvedValue();
  const restore = vi.spyOn(scan, 'restore').mockImplementation(async () => {
    if (restored) {
      scan.state = {
        status: 'ready',
        options: { root: '/', oneFileSystem: true },
        scanId: 1,
        rootId: 0,
        totals: { allocated: 983_000_000_000, apparent: 0, files: 0, dirs: 0, errors: 0 },
        elapsedMs: 0,
        scannedAt: Math.floor(Date.now() / 1000) - 7200,
        warnings: [],
        outdated,
      };
    }

    return restored;
  });
  const rescan = vi.spyOn(scan, 'rescan').mockResolvedValue();

  settings.rescanOnOpen = rescanOnOpen;

  return { advisor, settings, check, start, platform, startScan, restore, rescan, opening, scan, bootstrap };
}

/** Boots the app and lets the opening screen run its course, in fake time. */
async function boot(...args: Parameters<typeof prepare>) {
  const loaded = await prepare(...args);

  vi.useFakeTimers();

  try {
    const booting = loaded.bootstrap();

    await vi.advanceTimersByTimeAsync(OPENING_MS);
    await booting;
  } finally {
    vi.useRealTimers();
  }

  return loaded;
}

describe('bootstrap: the advisor', () => {
  beforeEach(() => {
    state.calls = [];
    localStorage.clear();
  });

  it('sends the stored options first, then subscribes to the status, and asks nothing at start', async () => {
    await boot({});

    expect(state.calls).toEqual(['configure:false', 'watch']);
  });

  it('asks nothing at start either when a view or a model state was asked for in the URL', async () => {
    await boot({ view: 'settings' });
    expect(state.calls).toEqual(['configure:false', 'watch']);

    state.calls = [];
    await boot({ advisor: 'none' });
    expect(state.calls).toEqual(['configure:false', 'mock:none', 'watch']);
  });

  it('starts the mock model loaded, and on in the settings, for a graphics card problem that waits', async () => {
    const { settings } = await boot({ gpuProblem: 'hang' });

    expect(settings.advisorEnabled).toBe(true);
    expect(state.calls).toEqual(['configure:true', 'mock:loaded', 'watch']);

    state.calls = [];
    await boot({ gpuProblem: 'hang', advisor: 'disk' });
    expect(state.calls).toEqual(['configure:true', 'mock:disk', 'watch']);
  });

  it('switches the model on for a mock state on disk without saving that, and sets the state after the options', async () => {
    const { settings } = await boot({ advisor: 'loaded' });

    expect(settings.advisorEnabled).toBe(true);
    expect(state.calls).toEqual(['configure:true', 'mock:loaded', 'watch']);
    expect(localStorage.getItem('platter.settings')).toBeNull();
  });
});

describe('bootstrap: the system', () => {
  beforeEach(() => {
    state.calls = [];
    localStorage.clear();
  });

  it('takes the system from the advisor status, Linux without one', async () => {
    expect((await boot({})).platform.os).toBe('linux');
    expect((await boot({}, 'windows')).platform.os).toBe('windows');
    expect((await boot({}, 'macos')).platform.os).toBe('macos');
  });

  it('scans the drive root of Windows when there is no volume and no root in the URL', async () => {
    const windows = await boot({ view: 'overview' }, 'windows');

    expect(windows.startScan).toHaveBeenCalledWith(expect.objectContaining({ root: 'C:\\' }));

    const linux = await boot({ view: 'overview' });

    expect(linux.startScan).toHaveBeenCalledWith(expect.objectContaining({ root: '/' }));
  });
});

describe('bootstrap: the update schedule', () => {
  beforeEach(() => {
    state.calls = [];
    localStorage.clear();
  });

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it('does not check for updates in the mock', async () => {
    const { check, start } = await boot({});

    expect(start).not.toHaveBeenCalled();
    expect(check).not.toHaveBeenCalled();
  });

  it('starts the schedule with the real backend, and leaves the first check to it', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true });

    const { check, start } = await boot({});

    expect(start).toHaveBeenCalledTimes(1);
    expect(check).not.toHaveBeenCalled();
  });

  it('checks at once and starts the schedule in the mock when the update hook is set', async () => {
    const { check, start } = await boot({ update: 'install' });

    expect(check).toHaveBeenCalledWith({ manual: false });
    expect(start).toHaveBeenCalledTimes(1);
  });
});

describe('bootstrap: the theme', () => {
  beforeEach(() => {
    state.calls = [];
    localStorage.clear();
  });

  afterEach(() => {
    document.documentElement.removeAttribute('data-theme');
  });

  it('shows the theme of the URL hook without saving it', async () => {
    await boot({ theme: 'light' });

    expect(document.documentElement.dataset.theme).toBe('light');
    expect(localStorage.getItem('platter.settings')).toBeNull();
  });

  it('leaves the theme alone without the hook', async () => {
    document.documentElement.dataset.theme = 'dark';

    await boot({});

    expect(document.documentElement.dataset.theme).toBe('dark');
  });
});

describe('bootstrap: the last scan', () => {
  beforeEach(() => {
    state.calls = [];
    localStorage.clear();
  });

  it('opens the saved scan and leaves it as it is by default', async () => {
    const { restore, rescan } = await boot({}, undefined, { restored: true });

    expect(restore).toHaveBeenCalledTimes(1);
    expect(rescan).not.toHaveBeenCalled();
  });

  it('scans the opened scan again when "Rescan when Platter opens" is on', async () => {
    const { rescan } = await boot({}, undefined, { restored: true, rescanOnOpen: true });

    expect(rescan).toHaveBeenCalledTimes(1);
  });

  it('scans nothing at start when there is no saved scan, whatever the setting', async () => {
    const { restore, rescan, startScan } = await boot({}, undefined, { restored: false, rescanOnOpen: true });

    expect(restore).toHaveBeenCalledTimes(1);
    expect(rescan).not.toHaveBeenCalled();
    expect(startScan).not.toHaveBeenCalled();
  });
});

describe('bootstrap: the opening screen', () => {
  beforeEach(() => {
    state.calls = [];
    localStorage.clear();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows the steps with what they found, and lands on the app', async () => {
    const loaded = await prepare({}, undefined, { restored: true });
    const { api } = await import('./api');

    vi.mocked(api.listVolumes).mockResolvedValueOnce([{}, {}] as never);
    vi.useFakeTimers();

    const booting = loaded.bootstrap();

    await vi.advanceTimersByTimeAsync(OPENING_MS);
    await booting;

    expect(loaded.opening.steps.map((step) => step.note)).toEqual([
      '2 disks',
      '983 GB · 2 hours ago',
      'up to date',
    ]);
    expect(loaded.opening.phase).toBe('landing');
  });

  it('tells that the opened scan is outdated, in amber', async () => {
    const { opening } = await boot({}, undefined, { restored: true, outdated: true });

    expect(opening.steps[2]?.note).toBe('outdated, rescan to refresh');
    expect(opening.steps[2]?.warn).toBe(true);
  });

  it('skips the last step without a saved scan, and lands on the start screen', async () => {
    const { opening, restore, rescan } = await boot({}, undefined, { restored: false, rescanOnOpen: true });

    expect(restore).toHaveBeenCalledTimes(1);
    expect(opening.steps.map((step) => step.status)).toEqual(['done', 'done', 'skipped']);
    expect(opening.steps[1]?.note).toBe('none kept');
    expect(rescan).not.toHaveBeenCalled();
    expect(opening.phase).toBe('landing');
  });

  it('starts the rescan before it lands, so the progress screen is what shows', async () => {
    const { opening, rescan } = await boot({}, undefined, { restored: true, rescanOnOpen: true });

    expect(rescan).toHaveBeenCalledTimes(1);
    expect(opening.steps[2]?.note).toBe('rescanning');
    expect(opening.phase).toBe('landing');
  });

  it('rescans a saved scan that opens only after the opening screen gave up on it, when the setting asks for it', async () => {
    const loaded = await prepare({}, undefined, { restored: true, rescanOnOpen: true });
    const { opening, restore, rescan, bootstrap, scan } = loaded;
    const { STEP_LIMIT_MS, SLOW_NOTE } = await import('./state/opening.svelte');
    let open: (() => void) | undefined;

    // A network folder that answers late: the saved scan opens 15 s in, after the step's limit.
    restore.mockImplementation(
      () =>
        new Promise<boolean>((resolve) => {
          open = () => {
            scan.state = { ...readyScan(), outdated: false };
            resolve(true);
          };
        }),
    );

    vi.useFakeTimers();

    try {
      const booting = bootstrap();

      await vi.advanceTimersByTimeAsync(STEP_LIMIT_MS + OPENING_MS);
      await booting;
      expect(opening.steps[1]?.note).toBe(SLOW_NOTE);
      expect(opening.steps[2]?.status).toBe('skipped');
      expect(rescan).not.toHaveBeenCalled();

      open?.();
      await vi.advanceTimersByTimeAsync(0);
      expect(rescan).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('lands when the disks cannot be read and no scan can be opened', async () => {
    const { opening, restore, bootstrap } = await prepare({});
    const { api } = await import('./api');

    vi.mocked(api.listVolumes).mockRejectedValueOnce(new Error('no disks'));
    restore.mockRejectedValue(new Error('unreadable'));
    vi.useFakeTimers();

    const booting = bootstrap();

    await vi.advanceTimersByTimeAsync(OPENING_MS);
    await booting;

    expect(opening.steps.map((step) => step.status)).toEqual(['done', 'done', 'skipped']);
    expect(opening.phase).toBe('landing');
  });

  it('waits for neither the update check nor the advisor to show its steps', async () => {
    let open!: () => void;
    const gate = new Promise<void>((resolve) => (open = resolve));
    const { opening, restore, bootstrap } = await prepare({}, 'windows', {
      restored: true,
      advisorGate: gate,
    });

    vi.useFakeTimers();

    const booting = bootstrap();

    // The advisor's status channel is held back, and the steps still go on, one after the other.
    await vi.advanceTimersByTimeAsync(3 * 1000);
    expect(restore).toHaveBeenCalledTimes(1);
    expect(opening.steps.map((step) => step.status)).toEqual(['done', 'done', 'done']);
    expect(state.calls).toEqual(['configure:false', 'watch']);

    // The landing waits a moment for the system to be known, and no longer.
    await vi.advanceTimersByTimeAsync(10_000);
    expect(opening.phase).toBe('landing');

    open();
    await vi.advanceTimersByTimeAsync(0);
    await booting;
  });

  it('sets the system from the advisor before it lands when the advisor is quick', async () => {
    const { opening, platform } = await boot({}, 'windows');

    expect(platform.os).toBe('windows');
    expect(opening.phase).toBe('landing');
  });

  it('is left out for a test hook that picks a view: the app starts as before, with the scan restored at once', async () => {
    const { opening, restore } = await boot({ view: 'settings' }, undefined, { restored: true });

    expect(opening.phase).toBe('done');
    expect(opening.steps.map((step) => step.status)).toEqual(['waiting', 'waiting', 'waiting']);
    expect(restore).toHaveBeenCalledTimes(1);
  });

  it('is left out for the static hook too', async () => {
    const { opening, restore } = await boot({ static: true }, undefined, { restored: true });

    expect(opening.phase).toBe('done');
    expect(restore).toHaveBeenCalledTimes(1);
  });
});
