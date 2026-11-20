import type { ReadyScan, RunningScan, ScanState } from './scan.svelte';
import type { Page } from './ui.svelte';

/** Why Overview, Explorer, Cleanup, Advisor and History cannot be opened yet. */
export const SCAN_FIRST_HINT = 'Scan a disk or folder first';

/** What the main area shows. The scan the screen is about travels with it. */
export type Screen =
  | { id: 'start' }
  | { id: 'progress'; scan: RunningScan }
  | { id: 'overview'; scan: ReadyScan }
  | { id: 'explorer'; scan: ReadyScan }
  /** The Cleanup view also shows its first step, the scan itself, so it takes a running scan too. */
  | { id: 'cleanup'; scan: ReadyScan | RunningScan }
  /** The Advisor view is about a scan (owner decision 2026-11-16): it opens once one is ready. The model download
   *  is reachable without a scan through the first-launch prompt and Settings. */
  | { id: 'advisor'; scan: ReadyScan }
  /** The History view waits for a scan like the other pages (owner decision 2026-11-17). */
  | { id: 'history' }
  | { id: 'settings' };

/**
 * The screen for a sidebar item and the state of the scan.
 *
 * - Settings is always itself: it needs no scan, and a running one does not hide it.
 * - A running scan always shows its progress, whichever item was chosen, except in Cleanup, whose first step is the
 *   scan: it shows that step.
 * - "Scan" shows the start screen, even when a result is ready (the result stays until a new scan starts).
 * - Overview, Explorer, Cleanup, Advisor and History show their page once a scan is ready. Before that they fall back
 *   to the start screen.
 */
export function resolveScreen(state: ScanState, page: Page): Screen {
  if (page === 'settings') {
    return { id: 'settings' };
  }

  if (state.status === 'scanning') {
    return page === 'cleanup' ? { id: 'cleanup', scan: state } : { id: 'progress', scan: state };
  }

  if (state.status === 'ready' && page === 'overview') {
    return { id: 'overview', scan: state };
  }

  if (state.status === 'ready' && page === 'explorer') {
    return { id: 'explorer', scan: state };
  }

  if (state.status === 'ready' && page === 'cleanup') {
    return { id: 'cleanup', scan: state };
  }

  if (state.status === 'ready' && page === 'advisor') {
    return { id: 'advisor', scan: state };
  }

  if (state.status === 'ready' && page === 'history') {
    return { id: 'history' };
  }

  return { id: 'start' };
}

/** The sidebar item that is highlighted: the start and progress screens both belong to "Scan". */
export function highlightedPage(screen: Screen): Page {
  switch (screen.id) {
    case 'start':
    case 'progress':
      return 'scan';
    default:
      return screen.id;
  }
}

/** Overview, Explorer, Cleanup, Advisor and History wait for a scan, like each other. */
export function isPageAvailable(page: Page, status: ScanState['status']): boolean {
  if (
    page === 'overview' ||
    page === 'explorer' ||
    page === 'cleanup' ||
    page === 'advisor' ||
    page === 'history'
  ) {
    return status === 'ready';
  }

  return true;
}

/**
 * A finished scan takes the user to its Overview, unless they are in Settings, History or the Advisor view (they are
 * not waiting for the scan), or in Cleanup, where the banner's Rescan is meant to come back to the review of the new
 * scan.
 */
export function pageAfterScan(page: Page): Page {
  return page === 'settings' || page === 'history' || page === 'cleanup' || page === 'advisor'
    ? page
    : 'overview';
}
