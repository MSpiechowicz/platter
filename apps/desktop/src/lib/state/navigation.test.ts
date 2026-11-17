import { describe, expect, it } from 'vitest';
import type { ReadyScan, RunningScan, ScanState } from './scan.svelte';
import { highlightedPage, isPageAvailable, pageAfterScan, resolveScreen, type Screen } from './navigation';
import type { Page } from './ui.svelte';

const options = { root: '/', oneFileSystem: true };

const running: RunningScan = {
  status: 'scanning',
  options,
  progress: { files: 1, dirs: 1, bytes: 1, errors: 0, currentPath: '/' },
  cancelling: false,
};

const ready: ReadyScan = {
  status: 'ready',
  options,
  scanId: 1,
  rootId: 0,
  totals: { allocated: 1, apparent: 1, files: 1, dirs: 1, errors: 0 },
  elapsedMs: 10,
  warnings: [],
  outdated: false,
};

const states: Record<string, ScanState> = {
  idle: { status: 'idle' },
  scanning: running,
  ready,
  cancelled: { status: 'cancelled', options },
  failed: { status: 'failed', options, message: 'boom' },
};

const screenIds = (page: Page): Record<string, Screen['id']> =>
  Object.fromEntries(Object.entries(states).map(([name, state]) => [name, resolveScreen(state, page).id]));

describe('resolveScreen', () => {
  it('shows the start screen for "Scan" unless a scan is running', () => {
    expect(screenIds('scan')).toEqual({
      idle: 'start',
      scanning: 'progress',
      ready: 'start',
      cancelled: 'start',
      failed: 'start',
    });
  });

  it('shows Overview and Explorer only for a ready scan, progress while scanning, else the start screen', () => {
    for (const page of ['overview', 'explorer'] as const) {
      expect(screenIds(page)).toEqual({
        idle: 'start',
        scanning: 'progress',
        ready: page,
        cancelled: 'start',
        failed: 'start',
      });
    }
  });

  it('shows Cleanup for a ready scan, and its first step, the scan, for a running one', () => {
    expect(screenIds('cleanup')).toEqual({
      idle: 'start',
      scanning: 'cleanup',
      ready: 'cleanup',
      cancelled: 'start',
      failed: 'start',
    });
  });

  it('shows Settings in every state, even while a scan is running', () => {
    expect(Object.values(screenIds('settings'))).toEqual(Array(5).fill('settings'));
  });

  it('shows History only for a ready scan, progress while scanning, else the start screen', () => {
    expect(screenIds('history')).toEqual({
      idle: 'start',
      scanning: 'progress',
      ready: 'history',
      cancelled: 'start',
      failed: 'start',
    });
  });

  it('shows the Advisor view only for a ready scan, progress while scanning, else the start screen', () => {
    expect(screenIds('advisor')).toEqual({
      idle: 'start',
      scanning: 'progress',
      ready: 'advisor',
      cancelled: 'start',
      failed: 'start',
    });
    expect(resolveScreen(ready, 'advisor')).toEqual({ id: 'advisor', scan: ready });
  });

  it('hands the scan to the screens that are about it', () => {
    expect(resolveScreen(running, 'scan')).toEqual({ id: 'progress', scan: running });
    expect(resolveScreen(ready, 'explorer')).toEqual({ id: 'explorer', scan: ready });
    expect(resolveScreen(running, 'cleanup')).toEqual({ id: 'cleanup', scan: running });
    expect(resolveScreen(ready, 'cleanup')).toEqual({ id: 'cleanup', scan: ready });
  });
});

describe('highlightedPage', () => {
  it('highlights "Scan" on the start and progress screens, the page itself otherwise', () => {
    expect(highlightedPage(resolveScreen(states.idle as ScanState, 'overview'))).toBe('scan');
    expect(highlightedPage(resolveScreen(running, 'explorer'))).toBe('scan');
    expect(highlightedPage(resolveScreen(ready, 'scan'))).toBe('scan');
    expect(highlightedPage(resolveScreen(ready, 'overview'))).toBe('overview');
    expect(highlightedPage(resolveScreen(ready, 'explorer'))).toBe('explorer');
    expect(highlightedPage(resolveScreen(ready, 'cleanup'))).toBe('cleanup');
    expect(highlightedPage(resolveScreen(running, 'cleanup'))).toBe('cleanup');
    expect(highlightedPage(resolveScreen(running, 'settings'))).toBe('settings');
    expect(highlightedPage(resolveScreen(running, 'advisor'))).toBe('scan');
    expect(highlightedPage(resolveScreen(ready, 'advisor'))).toBe('advisor');
    expect(highlightedPage(resolveScreen(states.idle as ScanState, 'history'))).toBe('scan');
    expect(highlightedPage(resolveScreen(ready, 'history'))).toBe('history');
  });
});

describe('isPageAvailable', () => {
  it('opens Overview, Explorer, Cleanup, Advisor and History only for a ready scan', () => {
    for (const [name, state] of Object.entries(states)) {
      expect(isPageAvailable('overview', state.status), name).toBe(name === 'ready');
      expect(isPageAvailable('explorer', state.status), name).toBe(name === 'ready');
      expect(isPageAvailable('cleanup', state.status), name).toBe(name === 'ready');
      expect(isPageAvailable('advisor', state.status), name).toBe(name === 'ready');
      expect(isPageAvailable('history', state.status), name).toBe(name === 'ready');
    }
  });

  it('always opens Scan and Settings', () => {
    for (const state of Object.values(states)) {
      expect(isPageAvailable('scan', state.status)).toBe(true);
      expect(isPageAvailable('settings', state.status)).toBe(true);
    }
  });
});

describe('pageAfterScan', () => {
  it('goes to Overview, but leaves Settings, History, Advisor and Cleanup alone', () => {
    expect(pageAfterScan('scan')).toBe('overview');
    expect(pageAfterScan('overview')).toBe('overview');
    expect(pageAfterScan('explorer')).toBe('overview');
    expect(pageAfterScan('settings')).toBe('settings');
    expect(pageAfterScan('cleanup')).toBe('cleanup');
    expect(pageAfterScan('advisor')).toBe('advisor');
    expect(pageAfterScan('history')).toBe('history');
  });
});
