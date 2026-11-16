/**
 * URL switches for testing and screenshots, e.g. `/?view=overview&static&mini`.
 * They never change what the app does for a user who does not type them, and they are ignored
 * inside the Tauri window, where nobody can type a URL anyway.
 */
export type HookView = 'start' | 'scanning' | 'overview' | 'explorer' | 'cleanup' | 'settings';

export interface TestHooks {
  /** Jump to a view; `overview`, `scanning`, `explorer` and `cleanup` start a scan on the mock first. */
  view: HookView | null;
  /** Start with the sidebar collapsed (not saved). */
  mini: boolean;
  /** No animations or transitions, and the mock scan finishes at once. */
  static: boolean;
  /** Show the sidebar update notice with demo data. */
  update: boolean;
  /** With `view=start`: finish a mock scan first, so the start screen is shown with a result ready. */
  ready: boolean;
  /** A mock volume list where one filesystem is mounted several times (btrfs subvolumes). */
  mounts: boolean;
  /** Scan this path instead of the first volume. */
  root: string | null;
  /** Open the update tooltip and show a toast, to look at them. */
  overlays: boolean;
  /** Explorer: open in this view (not saved). */
  ex: 'flat' | '3d' | null;
  /** Explorer: select the n-th child (0 is the biggest) once the folder is shown. */
  pick: number | null;
  /** Explorer: open on this node id. */
  focus: number | null;
  /** The mock disk keeps nearly everything in one folder, and nearly all of that in one folder inside it. */
  dominant: boolean;
  /** The mock scan finishes with a warning (the mount table could not be read). */
  warn: boolean;
  /**
   * Overview and Cleanup, for screenshots: `review` selects every suggestion and opens the review dialog; `run`
   * cleans the safe ones on the mock. Only the mock can ever run it (the hooks are off inside the Tauri window).
   */
  clean: 'review' | 'run' | null;
  /** With `view=scanning`: show the Cleanup view's Scan step instead of the plain progress screen. */
  cleanup: boolean;
}

const VIEWS: readonly string[] = ['start', 'scanning', 'overview', 'explorer', 'cleanup', 'settings'];

const NO_HOOKS: TestHooks = {
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
  focus: null,
  dominant: false,
  warn: false,
  clean: null,
  cleanup: false,
};

function isInsideTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

function wholeNumber(value: string | null): number | null {
  if (value === null || !/^\d+$/.test(value)) {
    return null;
  }

  return Number(value);
}

export function parseHooks(search: string, insideTauri: boolean = isInsideTauri()): TestHooks {
  if (insideTauri) {
    return { ...NO_HOOKS };
  }

  const params = new URLSearchParams(search);
  const view = params.get('view');
  const ex = params.get('ex');
  const clean = params.get('clean');

  return {
    view: view !== null && VIEWS.includes(view) ? (view as HookView) : null,
    mini: params.has('mini'),
    static: params.has('static'),
    update: params.has('update'),
    ready: params.has('ready'),
    mounts: params.has('mounts'),
    root: params.get('root'),
    overlays: params.has('overlays'),
    ex: ex === 'flat' || ex === '3d' ? ex : null,
    pick: wholeNumber(params.get('pick')),
    focus: wholeNumber(params.get('focus')),
    dominant: params.has('dominant'),
    warn: params.has('warn'),
    clean: clean === 'review' || clean === 'run' ? clean : null,
    cleanup: params.has('cleanup'),
  };
}

export const hooks: TestHooks = parseHooks(typeof location === 'undefined' ? '' : location.search);
