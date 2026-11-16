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
});
