import type { ReadyScan, RunningScan, ScanState } from './scan.svelte';
import type { Page } from './ui.svelte';

/** Why Overview and Explorer cannot be opened yet. */
export const SCAN_FIRST_HINT = 'Scan a disk or folder first';

/** What the main area shows. The scan the screen is about travels with it. */
export type Screen =
  | { id: 'start' }
  | { id: 'progress'; scan: RunningScan }
  | { id: 'overview'; scan: ReadyScan }
  | { id: 'explorer'; scan: ReadyScan }
  | { id: 'settings' };

/**
 * The screen for a sidebar item and the state of the scan.
 *
 * - Settings is always itself.
 * - A running scan always shows its progress, whichever item was chosen.
 * - "Scan" shows the start screen, even when a result is ready (the result stays until a new scan starts).
 * - Overview and Explorer show their page once a scan is ready; before that they fall back to the start screen.
 */
export function resolveScreen(state: ScanState, page: Page): Screen {
  if (page === 'settings') {
    return { id: 'settings' };
  }

  if (state.status === 'scanning') {
    return { id: 'progress', scan: state };
  }

  if (state.status === 'ready' && page === 'overview') {
    return { id: 'overview', scan: state };
  }

  if (state.status === 'ready' && page === 'explorer') {
    return { id: 'explorer', scan: state };
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

/** Overview and Explorer have nothing to show until a scan is ready. */
export function isPageAvailable(page: Page, status: ScanState['status']): boolean {
  if (page === 'overview' || page === 'explorer') {
    return status === 'ready';
  }

  return true;
}

/** A finished scan takes the user to its Overview, unless they are in Settings. */
export function pageAfterScan(page: Page): Page {
  return page === 'settings' ? 'settings' : 'overview';
}
