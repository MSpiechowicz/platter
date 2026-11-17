/**
 * URL switches for testing and screenshots, e.g. `/?view=overview&static&mini`.
 * They never change what the app does for a user who does not type them, and they are ignored
 * inside the Tauri window, where nobody can type a URL anyway.
 */
export type HookView = 'start' | 'scanning' | 'overview' | 'explorer' | 'cleanup' | 'advisor' | 'settings';

/** Where the advisor mock starts: not downloaded, a download held at 40%, a download that failed at 40%, on disk, or loaded into RAM. */
export type HookAdvisor = 'none' | 'downloading' | 'failed' | 'disk' | 'loaded';

/**
 * The graphics card of the advisor mock: `fallback` has a card that cannot run the model, so the loaded model runs on
 * the processor and says so; `none` is a build without graphics card support (no switch in Settings). Without the
 * hook the card works.
 */
export type HookGpu = 'fallback' | 'none';

/** What went wrong on the graphics card in the `gpuProblem` hook: see `GpuProblemWhat`. */
export type HookGpuProblem = 'crash' | 'hang' | 'error';

export interface TestHooks {
  /**
   * Jump to a view; `overview`, `scanning`, `explorer` and `cleanup` start a scan on the mock first, and so does
   * `advisor` with `labels` or `root` (without them it opens with no scan).
   */
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
  /** Explorer: mark these children (0 is the biggest, `mark=0,2`) once the folder is shown, as a click would. */
  mark: number[];
  /** The mock disk keeps nearly everything in one folder, and nearly all of that in one folder inside it. */
  dominant: boolean;
  /** The mock scan finishes with a warning (the mount table could not be read). */
  warn: boolean;
  /** With `view=scanning`: show the Cleanup view's Scan step instead of the plain progress screen. */
  cleanup: boolean;
  /** The state the advisor mock starts in; the model is also switched on in the settings (not saved). */
  advisor: HookAdvisor | null;
  /** The graphics card of the advisor mock (`gpu=fallback`, `gpu=none`); with `advisor=loaded` it shows where the model runs. */
  gpu: HookGpu | null;
  /**
   * An incident on the graphics card that is waiting for an answer at start (`gpuProblem=crash`, `hang` or `error`):
   * the model is loaded and runs on the processor, and the dialog is open. It is also the last problem in Settings.
   */
  gpuProblem: HookGpuProblem | null;
  /** The graphics card problem log has a last problem (a crash) that was answered: the row in Settings, no dialog (`gpuLast=1`). */
  gpuLast: boolean;
  /** With `view=advisor`: scan, and let the mock model label the folders before the page opens. */
  labels: boolean;
  /**
   * With `view=advisor` and a scan: ask this free question once the page is ready (`ask=biggest folders`). On the demo
   * disk `ask=can I delete the src folder?` is asked back (four projects have a `src`), to look at the choices.
   */
  ask: string | null;
  /**
   * With `view=advisor` and a scan: ask this many questions, one after another and each answered, before the page
   * opens (`chat=12`), to look at a long conversation.
   */
  chat: number | null;
  /**
   * The mock starts with rules of the user's own (`rules`, six demo rules), or with a rules file that cannot be used
   * (`rules=broken`).
   */
  rules: 'seed' | 'broken' | null;
}

const VIEWS: readonly string[] = [
  'start',
  'scanning',
  'overview',
  'explorer',
  'cleanup',
  'advisor',
  'settings',
];

const ADVISOR_STATES: readonly string[] = ['none', 'downloading', 'failed', 'disk', 'loaded'];

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

/** `0,2` as numbers; anything that is not a whole number is left out. */
function wholeNumbers(value: string | null): number[] {
  return (value ?? '')
    .split(',')
    .map((part) => wholeNumber(part))
    .filter((part): part is number => part !== null);
}

export function parseHooks(search: string, insideTauri: boolean = isInsideTauri()): TestHooks {
  if (insideTauri) {
    return { ...NO_HOOKS };
  }

  const params = new URLSearchParams(search);
  const view = params.get('view');
  const ex = params.get('ex');
  const advisor = params.get('advisor');
  const gpu = params.get('gpu');
  const gpuProblem = params.get('gpuProblem');

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
    mark: wholeNumbers(params.get('mark')),
    focus: wholeNumber(params.get('focus')),
    dominant: params.has('dominant'),
    warn: params.has('warn'),
    cleanup: params.has('cleanup'),
    advisor: advisor !== null && ADVISOR_STATES.includes(advisor) ? (advisor as HookAdvisor) : null,
    gpu: gpu === 'fallback' || gpu === 'none' ? gpu : null,
    gpuProblem: gpuProblem === 'crash' || gpuProblem === 'hang' || gpuProblem === 'error' ? gpuProblem : null,
    gpuLast: params.has('gpuLast'),
    labels: params.has('labels'),
    ask: params.get('ask')?.trim() || null,
    chat: wholeNumber(params.get('chat')),
    rules: params.get('rules') === 'broken' ? 'broken' : params.has('rules') ? 'seed' : null,
  };
}

export const hooks: TestHooks = parseHooks(typeof location === 'undefined' ? '' : location.search);
