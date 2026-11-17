import { describe, expect, it } from 'vitest';
import { parseHooks } from './hooks';

const ALL_OFF = {
  view: null,
  mini: false,
  static: false,
  update: false,
  ready: false,
  mounts: false,
  root: null,
  overlays: false,
  ex: null,
  pick: null,
  mark: [],
  focus: null,
  dominant: false,
  warn: false,
  cleanup: false,
  advisor: null,
  gpu: null,
  gpuProblem: null,
  gpuLast: false,
  labels: false,
  ask: null,
  chat: null,
  rules: null,
  history: null,
};

describe('parseHooks', () => {
  it('is all off without a query', () => {
    expect(parseHooks('', false)).toEqual(ALL_OFF);
  });

  it('reads flags and values', () => {
    expect(parseHooks('?view=overview&static&mini&root=/home/demo', false)).toMatchObject({
      view: 'overview',
      static: true,
      mini: true,
      root: '/home/demo',
    });
  });

  it('reads the start screen hooks', () => {
    expect(parseHooks('?view=start&ready&mounts', false)).toMatchObject({
      view: 'start',
      ready: true,
      mounts: true,
    });
    expect(parseHooks('?view=start', false)).toMatchObject({ ready: false, mounts: false });
  });

  it('reads the scan warning hook, outside Tauri only', () => {
    expect(parseHooks('?view=overview&warn', false).warn).toBe(true);
    expect(parseHooks('?view=overview&warn', true).warn).toBe(false);
  });

  it('reads the dominant disk hook, outside Tauri only', () => {
    expect(parseHooks('?view=overview&dominant', false).dominant).toBe(true);
    expect(parseHooks('?view=overview&dominant', true).dominant).toBe(false);
  });

  it('reads the Cleanup view and its Scan step, outside Tauri only', () => {
    expect(parseHooks('?view=cleanup', false)).toMatchObject({ view: 'cleanup' });
    expect(parseHooks('?view=scanning&cleanup', false)).toMatchObject({ view: 'scanning', cleanup: true });
    expect(parseHooks('?view=scanning&cleanup', true)).toEqual(ALL_OFF);
  });

  it('ignores an unknown view', () => {
    expect(parseHooks('?view=nope', false).view).toBeNull();
  });

  it('reads the Explorer hooks', () => {
    expect(parseHooks('?view=explorer&ex=3d&pick=2&focus=17', false)).toMatchObject({
      view: 'explorer',
      ex: '3d',
      pick: 2,
      focus: 17,
    });
    expect(parseHooks('?ex=flat', false).ex).toBe('flat');
  });

  it('ignores Explorer hooks with bad values', () => {
    expect(parseHooks('?ex=cube&pick=-1&focus=abc&mark=x,-2', false)).toEqual(ALL_OFF);
  });

  it('reads the marks of the Explorer as a list of children', () => {
    expect(parseHooks('?mark=0', false).mark).toEqual([0]);
    expect(parseHooks('?mark=1,3,x,4', false).mark).toEqual([1, 3, 4]);
    expect(parseHooks('?mark=0,1', true).mark).toEqual([]);
  });

  it('ignores every hook inside the Tauri window', () => {
    const search = '?view=explorer&mini&static&update&overlays&root=/etc&ex=3d&pick=1&focus=5';

    expect(parseHooks(search, true)).toEqual(ALL_OFF);
    expect(parseHooks(search, false).view).toBe('explorer');
  });

  it('detects the Tauri window on its own', () => {
    expect(parseHooks('?view=settings').view).toBe('settings');

    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true });

    try {
      expect(parseHooks('?view=settings&static')).toEqual(ALL_OFF);
    } finally {
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    }
  });

  it('has no hook that starts or confirms a cleanup, whatever the query says', () => {
    const search = '?view=cleanup&clean=run&markclean=run&mark=0&clean=review&markclean=review';

    expect(parseHooks(search, false)).toEqual({ ...ALL_OFF, view: 'cleanup', mark: [0] });
    expect(Object.keys(parseHooks(search, false)).sort()).toEqual(Object.keys(ALL_OFF).sort());
  });

  it('reads the Advisor view and its mock model hooks, outside Tauri only', () => {
    expect(parseHooks('?view=advisor&advisor=loaded&labels', false)).toMatchObject({
      view: 'advisor',
      advisor: 'loaded',
      labels: true,
    });

    for (const state of ['none', 'downloading', 'disk', 'loaded']) {
      expect(parseHooks(`?advisor=${state}`, false).advisor).toBe(state);
    }

    expect(parseHooks('?advisor=huge', false).advisor).toBeNull();
    expect(parseHooks('?view=advisor&advisor=loaded&labels', true)).toEqual(ALL_OFF);
  });

  it('reads the graphics card of the mock model: a failing card or none, outside Tauri only', () => {
    expect(parseHooks('?view=advisor&advisor=loaded&gpu=fallback', false)).toMatchObject({ gpu: 'fallback' });
    expect(parseHooks('?view=settings&gpu=none', false).gpu).toBe('none');
    expect(parseHooks('?gpu=fast', false).gpu).toBeNull();
    expect(parseHooks('?gpu=fallback', true)).toEqual(ALL_OFF);
  });

  it('reads a graphics card problem that waits at start, and a last one in the log, outside Tauri only', () => {
    for (const what of ['crash', 'hang', 'error']) {
      expect(parseHooks(`?view=advisor&gpuProblem=${what}`, false).gpuProblem).toBe(what);
    }

    expect(parseHooks('?gpuProblem=melted', false).gpuProblem).toBeNull();
    expect(parseHooks('?gpuProblem', false).gpuProblem).toBeNull();
    expect(parseHooks('?view=settings&gpuLast=1', false)).toMatchObject({ gpuLast: true, gpuProblem: null });
    expect(parseHooks('?view=settings', false).gpuLast).toBe(false);
    expect(parseHooks('?gpuProblem=crash&gpuLast=1', true)).toEqual(ALL_OFF);
  });

  it('reads the question to ask on the Advisor view, outside Tauri only', () => {
    expect(parseHooks('?view=advisor&advisor=loaded&ask=biggest%20folders', false).ask).toBe(
      'biggest folders',
    );
    expect(parseHooks('?view=advisor&ask=%20%20', false).ask).toBeNull();
    expect(parseHooks('?view=advisor', false).ask).toBeNull();
    expect(parseHooks('?view=advisor&ask=biggest', true)).toEqual(ALL_OFF);
  });

  it('reads the number of questions to seed the Advisor conversation with, outside Tauri only', () => {
    expect(parseHooks('?view=advisor&advisor=loaded&chat=12', false).chat).toBe(12);
    expect(parseHooks('?view=advisor&chat=many', false).chat).toBeNull();
    expect(parseHooks('?view=advisor&chat=12', true)).toEqual(ALL_OFF);
  });

  it('reads the history view and the state of the history mock, outside Tauri only', () => {
    expect(parseHooks('?view=history', false).view).toBe('history');
    expect(parseHooks('?view=history&history=thin', false).history).toBe('thin');
    expect(parseHooks('?view=history&history=broken', false).history).toBe('broken');
    expect(parseHooks('?view=history&history=whatever', false).history).toBeNull();
    expect(parseHooks('?view=history', false).history).toBeNull();
    expect(parseHooks('?view=history&history=empty', true).history).toBeNull();
  });

  it('reads the hook that seeds the user rules, or breaks their file, outside Tauri only', () => {
    expect(parseHooks('?view=advisor&rules', false).rules).toBe('seed');
    expect(parseHooks('?view=advisor&rules=broken', false).rules).toBe('broken');
    expect(parseHooks('?view=advisor&rules=whatever', false).rules).toBe('seed');
    expect(parseHooks('?view=advisor', false).rules).toBeNull();
    expect(parseHooks('?view=advisor&rules=broken', true).rules).toBeNull();
  });
});
