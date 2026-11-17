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

async function boot(hooks: Record<string, unknown>, os?: 'windows') {
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

    if (os) {
      advisor.status = { os } as typeof advisor.status;
    }
  });

  const startScan = vi.spyOn(scan, 'start').mockResolvedValue();

  await bootstrap();

  return { advisor, settings, check, start, platform, startScan };
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
