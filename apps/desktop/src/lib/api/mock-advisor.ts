import { ApiRequestError, type Api } from './client';
import { folderAnswer } from './mock-advisor-folder';
import { answerQuestion, readsWithoutModel, type FolderLookup } from './mock-scan-question';
import { ruleView, type MockLabel, type RuleView, type UserRuleInput } from './mock-suggestions';
import type { MockNode, MockTree } from './mock-tree';
import type {
  AdvisorAnswerDto,
  AdvisorLabelsDto,
  AdvisorModelDto,
  AdvisorOptionsDto,
  AdvisorStatusDto,
  OsName,
  EntryMatchDto,
  FolderMatchDto,
  GpuProblemChoice,
  GpuProblemDto,
  GpuProblemWhat,
  LabelDto,
  ModelState,
  ModelVerdict,
  ScanAnswerDto,
  ScanId,
  StepDto,
  SummaryDto,
  UnloadAfter,
} from './types';

/*
 * The mock's stand-in for the advisor commands of `src-tauri/src/commands/advisor.rs` (docs/architecture.md,
 * "Advisor"): the state machine of the model, and a fake labeller over the mock tree, never a model.
 *
 *   notDownloaded -> downloading -> onDisk -> loading -> loaded -> onDisk
 *
 * Labelling, Ask, free questions and folder search (`labelFolders`, `cancelLabelling`, `getAdvisorLabels`,
 * `askAdvisor`, `askScan`, `findFolders`, `findEntries`) follow the contract's validation and answer order.
 * `askAdvisor` is the rules alone (`mock-advisor-folder.ts`): no model, no wait, no refusal for the model's state,
 * labelling or another question.
 * Still different from the backend:
 * - The fake model is a few name heuristics (cache, build, tmp, download, `.next`, vm, ... say `review`, any other
 *   name says `keep`) with a canned one-sentence reason. It never says `safe`. Its answers do not depend on facts
 *   such as sizes, ages or the rules beside a folder.
 * - Labelling takes 300 ms per folder (the backend about 2.7 s) and sends the status with `labelling` after each
 *   one; there is no 5 minute budget, so `stopped` is only ever `cancelled`. A free question takes 800 ms. A model on
 *   disk is loaded first, as in the backend. The status shows `tokensPerSecond` (36.5) after the first generation,
 *   while the model is loaded.
 * - Candidates follow `label_candidates` (top down, at most 50 folders of at least 256 MiB that no rule covers,
 *   that hold no rule match, are not the scan root and are not refused by the protected places, which here are a
 *   hand-copied list; none without a known home or inside a kept folder). The mock's rules are the mock's
 *   (`mock-suggestions.ts`), and it has no guard that probes a disk: only the folders the tree calls unreadable
 *   are refused for that.
 * - The user's rules (`mock-user-rules.ts`) work as in the backend: a rule covering a folder, or one above it,
 *   answers with source `user` ("Your rule: always keep." / "Your rule: safe to clean."), a label whose folder has
 *   a rule of its own says `decided`, and the rules count in "How a verdict is made" and in the candidates. While
 *   the rules file is broken they are all left out (the host passes none), as in the backend.
 * - An answer from the model is not stored as a label. The labels of a run are stored when the run ends (not folder
 *   by folder, as in the backend), and a run replaces the earlier labels of the scan, with one exception: a run that
 *   stops (cancelled, switched off) with no label at all keeps the earlier ones. A scan with no candidate is
 *   replaced by an empty run at once, without loading the model. The labels are forgotten with the scan, and when
 *   a cleanup outdates it (`mock.ts`); a run that ends after that stores nothing. `modelMs` is the simulated time
 *   (300 ms per folder).
 * - `labelFolders` answers once the job is taken. A failure after that is only in the status: `labelling` goes away
 *   and `problem` is set (here only when the model is switched off while it loads). It is not an error of the call.
 * - `guard` in `getAdvisorLabels` counts the folders of at least 256 MiB inside a protected place and the
 *   unreadable folders seen while searching (the contract does not say exactly what "left for you" counts); `candidates` is worked out from the tree on every call, also before
 *   any run. `rules` counts every folder (and file) a rule matched.
 * - Switching the model off cancels a labelling or a question in progress; unloading is `invalid` while one runs.
 * - There is one model (`MODEL`, Qwen3.5-4B). Download and load act on it.
 * - `askScan` answers from `mock-scan-question.ts`: the words of the question first, a keyword reading standing in for
 *   the model's part, and the backend's template (Platter writes the answer from the scan, with a direct first
 *   sentence), never a model's text. It is refused in the backend's order (labelling, a busy model or another question,
 *   switched off, not on disk). A question the words settle (an intent word and a path, a personal folder or a tool of
 *   the rules) is answered at once, without the model and without a wait, as the backend does; any other takes the model
 *   for 800 ms (loaded first when needed), and a model switched off meanwhile fails it
 *   with "the model was stopped before it read the question". While it runs, `labelFolders` is refused with "the model is
 *   busy, so wait for it to finish" (`askAdvisor` is not: it uses no model). A `pick` takes one choice
 *   of the same question asked back, and a stale one is `invalid`.
 * - `findFolders` matches names in the mock's tree; an exact path is looked up as the rules see it (the demo disks
 *   `/real` and `/skew` start at `/`), and `~` is the home directory of the mock's rules. A blank query finds nothing;
 *   any other is matched as typed (lower-cased, not trimmed). The scan root matches only by its exact path.
 * - `findEntries` also lists regular files (never a link or a mount point, nor the scan root). A query that is `~` or
 *   starts with `~/` or `/` is a path: the children of the folder before its last `/` that fit the rest. Any other
 *   query is a name looked for in the whole tree.
 *
 * Still different from the backend (download and state machine):
 * - The download is simulated: 30 chunks, 100 ms apart (about 3 s). The backend asks the user in a native dialog
 *   first, and a declined dialog rejects with `cancelled` and leaves the model `notDownloaded`; the mock asks the
 *   `confirmDownload` hook (default: yes) once the state allows a download. Cancel keeps the bytes received so far, and a
 *   new download continues from them (the backend keeps a `.partial` file and uses `Range`). While a cancelled
 *   download has bytes, the status of `notDownloaded` carries `download` so that the UI can offer "Resume".
 * - Nothing is checked (no size, no SHA-256, no memory), so no download fails and no load is refused for those
 *   reasons. `problem` is set only when the model is switched off while it loads (as the backend: the load ends
 *   `onDisk` with a problem and the load call answers with that status).
 * - The options start as given (by default switched on); the backend starts switched off until the web view
 *   sends its stored options.
 * - The memory figures are fixed: 2 GiB in use of 32 GiB.
 * - The idle timer starts when the model is loaded. `windowClose` has no timer: the mock has no window to close.
 */

const GIB = 1024 * 1024 * 1024;

/** The one model on offer. */
const MODEL: AdvisorModelDto = {
  name: 'Qwen3.5-4B',
  format: 'GGUF · Q4_K_M',
  bytes: 2_740_937_888,
  host: 'huggingface.co',
};

const RAM_LOADED = 2 * GIB;
const RAM_TOTAL = 32 * GIB;

const DOWNLOAD_CHUNKS = 30;
const DOWNLOAD_STEP_MS = 100;
const LOAD_MS = 400;

/** The idle time of each option; `windowClose` is not a time. */
const IDLE_MS: Record<UnloadAfter, number | null> = {
  oneMinute: 60_000,
  fiveMinutes: 5 * 60_000,
  windowClose: null,
};

const MIB = 1024 * 1024;

/** The limits of `label_candidates`. */
const MIN_CANDIDATE_BYTES = 256 * MIB;
const MAX_CANDIDATES = 50;

/** Simulated time per labelled folder and per question, and the speed shown after the first generation. */
const LABEL_STEP_MS = 300;
const ASK_MS = 800;
const TOKENS_PER_SECOND = 36.5;

/** The limits of `find_folders`. */
const MAX_QUERY_BYTES = 1024;
const MAX_FOUND = 20;

/** The refusal while another job holds the model, as the backend's `busy()`. */
const BUSY = 'the model is busy, so wait for it to finish';

/** Whether a job is a free question: it holds the model, also for what the backend would answer without it. */
const holdsModel = (job: Job): boolean => job.kind === 'ask' && job.free === true;

/** The longest question `askScan` takes, in characters after trimming. */
const MAX_QUESTION_CHARS = 500;

const DEFAULT_OPTIONS: AdvisorOptionsDto = {
  enabled: true,
  unloadAfter: 'fiveMinutes',
  logQuestions: true,
  useGpu: true,
};

/** The card and the video memory of the mock's graphics card (the owner's machine). */
const GPU_NAME = 'AMD Radeon RX 7900 XTX (RADV NAVI31)';
const VIDEO_LOADED = 3.2 * 1000 ** 3;

/** The backend's problem line when the graphics card could not run the model. */
const GPU_FALLBACK_PROBLEM = 'The graphics card could not run the model, so it runs on the processor.';

/**
 * What the mock's build and graphics card do: `works` runs the model on the card; `fallback` has a card that fails,
 * so the model runs on the processor and says so; `none` is a build without graphics card support.
 */
export type MockGpu = 'works' | 'fallback' | 'none';

/** The mock's source of graphics card incidents: one that is there at start, pending or only in the log. */
export interface MockGpuIncident {
  what: GpuProblemWhat;
  /** Waiting for the user's answer (the card is blocked, so the model runs on the processor); else only logged. */
  pending: boolean;
}

/** What a URL hook can start the mock in. */
export type MockAdvisorState = 'none' | 'downloading' | 'failed' | 'disk' | 'loaded';

/** The share of the download a mock started as `downloading` has received. */
const HELD_SHARE = 0.4;

/** Timers the state machine uses; tests pass their own so that no real time passes. */
export interface MockTimers {
  set(callback: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

export interface MockAdvisorOptions {
  timers?: MockTimers;
  /** Where the machine starts (default `none`). */
  state?: MockAdvisorState;
  options?: AdvisorOptionsDto;
  /** The graphics card of the build (default `works`, and `none` on Windows, which has no card support). */
  gpu?: MockGpu;
  /** The system the app reports in its status (default `linux`). */
  os?: OsName;
  /** A graphics card incident the mock starts with (default none). */
  gpuIncident?: MockGpuIncident;
  /** Test hook: something that is not a file is at the log's path, so clearing the log fails with `io`. */
  questionLogBlocked?: boolean;
  /** Test hook: the answer of the download's confirmation dialog (default: confirmed). */
  confirmDownload?: () => boolean;
}

export type MockAdvisorApi = Pick<
  Api,
  | 'advisorStatus'
  | 'watchAdvisor'
  | 'configureAdvisor'
  | 'clearQuestionLog'
  | 'answerGpuProblem'
  | 'clearGpuLog'
  | 'startModelDownload'
  | 'cancelModelDownload'
  | 'loadModel'
  | 'unloadModel'
  | 'labelFolders'
  | 'cancelLabelling'
  | 'getAdvisorLabels'
  | 'askAdvisor'
  | 'askScan'
  | 'findFolders'
  | 'findEntries'
>;

/** The advisor mock: the commands, and what the rest of the mock needs to know about the options. */
export type MockAdvisor = MockAdvisorApi & {
  /** Whether the advisor is switched on in the options; the `advisor` suggestion exists only then. */
  advisorEnabled(): boolean;
};

/** The labels of the last labelling run of a scan. */
export interface LabelRun {
  labels: MockLabel[];
  stopped?: 'cancelled' | 'timeBudget';
  /** Simulated milliseconds the fake model ran. */
  ms: number;
}

/** The runs by scan; the mock removes a scan's run when the scan is dropped. */
export type MockLabelRuns = Map<ScanId, LabelRun>;

export interface MockAdvisorHost {
  /** The tree of a scan; rejects with `notFound` for an unknown scan. */
  treeOf(scan: ScanId): MockTree;
  pathOf(tree: MockTree, node: MockNode): string;
  /** The scan has not finished. */
  isRunning(scan: ScanId): boolean;
  /** Scans a cleanup ran on. */
  outdated: ReadonlySet<ScanId>;
  runs: MockLabelRuns;
  /** The summary of a folder (shares by category and by age). */
  summary(tree: MockTree, node: MockNode): SummaryDto;
  /** The user's rules now; none while their file is broken. */
  userRules(): readonly UserRuleInput[];
  /** Why the rules file cannot be used, if it cannot. */
  rulesProblem?(): string | undefined;
}

const realTimers: MockTimers = {
  set: (callback, ms) => setTimeout(callback, ms),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/** What the fake model says about a folder name: the first entry whose word the name has wins. */
const REVIEW_WORDS: { word: RegExp; reason: string }[] = [
  { word: /cache/, reason: 'Looks like a cache, which the program that made it builds again.' },
  {
    word: /node_modules/,
    reason: 'Looks like installed dependencies, which a package manager installs again.',
  },
  { word: /build|dist|\.next|target/, reason: 'Looks like build output, which a new build makes again.' },
  { word: /tmp|temp/, reason: 'Looks like temporary files that nothing needs after a while.' },
  { word: /download/, reason: 'Looks like downloaded files that can be fetched again.' },
  {
    word: /(^|[^a-z0-9])vms?([^a-z0-9]|$)|container/,
    reason: 'Looks like virtual machine or container storage that can be made again.',
  },
  { word: /(^|[^a-z0-9])logs?([^a-z0-9]|$)/, reason: 'Looks like old logs that nothing reads any more.' },
];

const KEEP_REASON = 'Looks like files someone made or cannot get back, so it is better left alone.';

/** The fake model: simple name heuristics, never `safe`, one sentence of reason. */
function judge(name: string): { verdict: ModelVerdict; reason: string } {
  const lower = name.toLowerCase();
  const hit = REVIEW_WORDS.find((entry) => entry.word.test(lower));

  return hit ? { verdict: 'review', reason: hit.reason } : { verdict: 'keep', reason: KEEP_REASON };
}

/** Places that may be neither a target nor inside one (`Protected` in `crates/platter-cleanup/src/protect.rs`, abridged). */
const SYSTEM_EXACT = [
  '/',
  '/usr',
  '/var',
  '/var/lib',
  '/home',
  '/etc',
  '/boot',
  '/root',
  '/bin',
  '/sbin',
  '/lib',
];
const SYSTEM_INSIDE = ['/etc', '/usr', '/boot', '/root', '/bin', '/sbin', '/lib'];
const HOME_INSIDE = [
  '.ssh',
  '.gnupg',
  '.password-store',
  '.pki',
  '.config',
  '.local/share/keyrings',
  '.local/share/kwalletd',
  '.mozilla',
  '.thunderbird',
  '.var',
  'snap',
  '.aws',
  '.kube',
  '.netrc',
  '.git-credentials',
  'Desktop',
  'Documents',
  'Music',
  'Pictures',
  'Videos',
];

export type Protection = 'inside' | 'exact' | 'holds';

/** The protected place a path is, lies inside or holds, and which of the three it is. */
export function protectedPlace(
  path: string,
  home: string | null,
): { kind: Protection; place: string } | null {
  const inside = [...SYSTEM_INSIDE, ...(home ? HOME_INSIDE.map((part) => `${home}/${part}`) : [])];
  const within = inside.find((place) => path === place || path.startsWith(`${place}/`));

  if (within !== undefined) {
    return { kind: 'inside', place: within };
  }

  const exact = home ? [...SYSTEM_EXACT, home] : SYSTEM_EXACT;

  if (exact.includes(path)) {
    return { kind: 'exact', place: path };
  }

  const held = exact.find((place) => place.startsWith(`${path}/`));

  return held === undefined ? null : { kind: 'holds', place: held };
}

/** Whether a path is a protected place, lies inside one, or holds one. */
export function protection(path: string, home: string | null): Protection | null {
  return protectedPlace(path, home)?.kind ?? null;
}

/** What the search for candidates found, and what the guard turned away on the way. */
interface Search {
  candidates: MockNode[];
  /** Folders of at least the minimum size inside a protected place (not entered), and unreadable folders. */
  refused: MockNode[];
}

/** `label_candidates`: the largest folders no rule covers, found top down; a folder that is taken is not entered. */
function searchCandidates(tree: MockTree, view: RuleView): Search {
  const found: Search = { candidates: [], refused: [] };

  if (view.insideKept || view.home === null) {
    return found;
  }

  const children = (node: MockNode): MockNode[] =>
    tree.nodes.slice(node.firstChild, node.firstChild + node.childCount);
  const bigDirs = (node: MockNode): MockNode[] =>
    children(node).filter((child) => child.kind === 'dir' && child.allocated >= MIN_CANDIDATE_BYTES);

  const stack: MockNode[] = [tree.nodes[0] as MockNode];

  for (let node = stack.pop(); node; node = stack.pop()) {
    if (node.unreadable || view.matched.has(node.id)) {
      continue;
    }

    const kind = node.parent < 0 ? 'exact' : protection(view.path(node), view.home);

    if (node.parent >= 0 && kind === null && !view.holding.has(node.id)) {
      found.candidates.push(node);
      continue;
    }

    if (kind === 'inside') {
      found.refused.push(node);
    }

    // Only the places that cannot be a target but hold what can be, and the folders above a rule's match, are entered.
    if (kind !== 'inside') {
      stack.push(...bigDirs(node));
      found.refused.push(...children(node).filter((child) => child.kind === 'dir' && child.unreadable));
    }
  }

  found.candidates.sort((a, b) => b.allocated - a.allocated || a.id - b.id);
  found.candidates.splice(MAX_CANDIDATES);
  found.refused.sort((a, b) => b.allocated - a.allocated || a.id - b.id);

  return found;
}

/** What the guard says about a folder, or `null` when it lets the question through. */
function guardReason(tree: MockTree, node: MockNode, view: RuleView): string | null {
  if (node.parent < 0) {
    return 'This is the folder that was scanned, so Platter never suggests cleaning it.';
  }

  if (view.insideKept) {
    return 'The scan starts inside a folder you keep, so nothing in it is suggested.';
  }

  if (node.unreadable) {
    return 'Platter could not read this folder, so it cannot tell what is in it.';
  }

  const found = protectedPlace(view.path(node), view.home);

  if (found === null) {
    return null;
  }

  if (found.kind === 'holds') {
    return `This folder holds a protected place (${found.place}), so it cannot be cleaned as a whole. Look inside it.`;
  }

  return `This is a protected place, or inside one (${found.place}). Platter does not clean it.`;
}

const encoder = new TextEncoder();

const utf8Length = (text: string): number => encoder.encode(text).length;

/** The folder an exact path (`~/...` or `/...`) names, if the tree has it, or with `files` a regular file too. */
function folderAtPath(tree: MockTree, view: RuleView, text: string, files = false): MockNode | null {
  let path = text;

  if (text === '~' || text.startsWith('~/')) {
    if (view.home === null) {
      return null;
    }

    path = `${view.home}${text.slice(1)}`;
  }

  if (!path.startsWith('/')) {
    return null;
  }

  const namesOf = (full: string): string[] =>
    full === '/' ? [] : full.replace(/\/$/, '').slice(1).split('/');
  const names = namesOf(path);
  const rootNames = namesOf(view.path(tree.nodes[0] as MockNode));

  if (names.some((name) => name === '' || name === '.' || name === '..')) {
    return null;
  }

  if (names.length < rootNames.length || rootNames.some((name, index) => names[index] !== name)) {
    return null;
  }

  let at: MockNode | undefined = tree.nodes[0];

  const inner = names.slice(rootNames.length);

  for (const [index, name] of inner.entries()) {
    const parent: MockNode = at as MockNode;
    const last = index === inner.length - 1;

    at = tree.nodes
      .slice(parent.firstChild, parent.firstChild + parent.childCount)
      .find(
        (child) => child.name === name && (child.kind === 'dir' || (files && last && child.kind === 'file')),
      );

    if (!at) {
      return null;
    }
  }

  return at ?? null;
}

/** `find_folders`: an exact path first, then names that start with the text, then names that contain it; largest first. */
function searchFolders(tree: MockTree, view: RuleView, query: string, wanted: number): MockNode[] {
  // As the backend: only the blank check trims, the text is matched as typed.
  if (query.trim() === '') {
    return [];
  }

  const needle = query.toLowerCase();
  const exact = folderAtPath(tree, view, query);
  // The scan root matches only by its exact path.
  const folders = tree.nodes.filter((node) => node.kind === 'dir' && node.parent >= 0 && node !== exact);
  const bySize = (a: MockNode, b: MockNode): number => b.allocated - a.allocated || a.id - b.id;
  const prefix = folders.filter((node) => node.name.toLowerCase().startsWith(needle)).sort(bySize);
  const inside = folders
    .filter((node) => !node.name.toLowerCase().startsWith(needle) && node.name.toLowerCase().includes(needle))
    .sort(bySize);

  return [...(exact ? [exact] : []), ...prefix, ...inside].slice(0, wanted);
}

/**
 * `find_entries`: folders and files, never a link, a mount point or the scan root. A path (`~`, `~/...` or `/...`)
 * lists the children of the folder before its last `/` whose name fits the rest, any other text looks in the whole tree. An
 * exact name (case ignored) comes first, then names that start with the text, then names that hold it, largest first.
 */
function searchEntries(tree: MockTree, view: RuleView, query: string, wanted: number): MockNode[] {
  if (query.trim() === '') {
    return [];
  }

  let pool: MockNode[];
  let rest = query;

  // A path is exactly `~`, or starts with `~/` or `/`: `~foo` is a name.
  if (query === '~' || query.startsWith('~/') || query.startsWith('/')) {
    // A bare `~` is `~/`.
    const path = query === '~' ? '~/' : query;
    const slash = path.lastIndexOf('/');
    const parent = folderAtPath(tree, view, path.slice(0, slash) || '/');

    if (!parent) {
      return [];
    }

    pool = tree.nodes.slice(parent.firstChild, parent.firstChild + parent.childCount);
    rest = path.slice(slash + 1);
  } else {
    pool = tree.nodes.filter((node) => node.parent >= 0);
  }

  const needle = rest.toLowerCase();
  const bySize = (a: MockNode, b: MockNode): number => b.allocated - a.allocated || a.id - b.id;
  const entries = pool.filter((node) => node.kind === 'dir' || node.kind === 'file');
  const nameOf = (node: MockNode): string => node.name.toLowerCase();
  const exact = entries.filter((node) => nameOf(node) === needle).sort(bySize);
  const prefix = entries
    .filter((node) => nameOf(node) !== needle && nameOf(node).startsWith(needle))
    .sort(bySize);
  const inside = entries
    .filter((node) => !nameOf(node).startsWith(needle) && nameOf(node).includes(needle))
    .sort(bySize);

  return [...exact, ...prefix, ...inside].slice(0, wanted);
}

const invalid = (message: string): never => {
  throw new ApiRequestError('invalid', message);
};

const fail = (code: 'notFound' | 'invalid', message: string): never => {
  throw new ApiRequestError(code, message);
};

/** What the user's rules say about exactly this folder (`keep` when they say both), as the `decided` of a label. */
function decidedOf(user: readonly UserRuleInput[], path: string): Pick<LabelDto, 'decided'> {
  const said = (verdict: UserRuleInput['verdict']): boolean =>
    user.some((rule) => rule.verdict === verdict && rule.folder === path);

  if (said('keep')) {
    return { decided: 'keep' };
  }

  return said('safe') ? { decided: 'safe' } : {};
}

function stepOf(folders: number, allocated: number, example?: StepDto['example']): StepDto {
  return example ? { folders, allocated, example } : { folders, allocated };
}

function validOptions(value: AdvisorOptionsDto): boolean {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof value.enabled === 'boolean' &&
    typeof value.logQuestions === 'boolean' &&
    // As the backend: `useGpu` may be left out (it is then on).
    (value.useGpu === undefined || typeof value.useGpu === 'boolean') &&
    Object.hasOwn(IDLE_MS, value.unloadAfter)
  );
}

/** The mock that the URL hooks reach: the last one created. */
let applyHookState: ((state: MockAdvisorState) => void) | null = null;

/**
 * Puts the advisor mock created last into a state, for URL hooks and screenshots. `downloading` stands still at
 * 40% until the download is cancelled or started again, so it can be looked at; `loaded` unloads after the
 * configured idle time as usual.
 */
export function setMockAdvisorState(state: MockAdvisorState): void {
  applyHookState?.(state);
}

type LabelJob = {
  kind: 'label';
  scan: ScanId;
  candidates: MockNode[];
  run: LabelRun;
  done: number;
  cancelled: boolean;
  timer: unknown;
};

/** A labelling run or a question the fake model is busy with. */
type Job =
  | LabelJob
  | {
      kind: 'ask';
      reject: (error: ApiRequestError) => void;
      timer: unknown;
      /** A free question (`askScan`): it holds the model, so labelling and a folder question are refused meanwhile. */
      free?: true;
      /** What a question that is stopped fails with, whatever stopped it (a free question has one reason). */
      stoppedAs?: ApiRequestError;
    };

export function createMockAdvisor(host: MockAdvisorHost, init: MockAdvisorOptions = {}): MockAdvisor {
  const timers = init.timers ?? realTimers;
  const confirmDownload = init.confirmDownload ?? (() => true);

  /** The lookups a question's scope is resolved with: the ones `findFolders` and `findEntries` use. */
  const lookupOf = (tree: MockTree): FolderLookup => {
    const view = ruleView(tree, host.userRules());

    return {
      exact: (path) => folderAtPath(tree, view, path),
      entry: (path) => folderAtPath(tree, view, path, true),
      search: (text, limit) => searchFolders(tree, view, text, limit),
      searchEntries: (text, limit) => searchEntries(tree, view, text, limit),
    };
  };

  /** As the backend: while the rules file is broken nothing is labelled or asked, so a kept folder never reaches the model. */
  const rulesUsable = (): void => {
    const problem = host.rulesProblem?.();

    if (problem !== undefined) {
      invalid(
        `Your rules file cannot be used, so Platter makes no cleanup plan: ${problem} Fix the file, or start over in the Advisor view.`,
      );
    }
  };
  /** As the backend: after a declined dialog, a new download request waits this long. */
  const DECLINE_PAUSE_MS = 2_000;
  let declinePause: unknown = null;

  let state: ModelState = 'notDownloaded';
  let options: AdvisorOptionsDto = { ...(init.options ?? DEFAULT_OPTIONS) };
  const os = init.os ?? 'linux';
  const gpu = init.gpu ?? (os === 'windows' ? 'none' : 'works');
  /** The unanswered incident. */
  let pendingGpu: GpuProblemDto | undefined;
  /** The card is blocked from the incident until "try again": the model runs on the processor. */
  let gpuBlocked = false;
  /** The last incident of the log, answered or not; clearing the log removes it. */
  let lastGpu: GpuProblemDto | undefined;
  /** A model that stays on the processor after "try again" until it is unloaded; the card is tried at the next load. */
  let heldOnCpu = false;
  let lastIncidentId = 0;

  /** As the backend: the id is the Unix time in ms, bumped when it equals the one before. */
  const incident = (what: GpuProblemWhat): GpuProblemDto => {
    lastIncidentId = Math.max(Date.now(), lastIncidentId + 1);

    return { id: lastIncidentId, at: lastIncidentId, card: GPU_NAME, what };
  };

  if (init.gpuIncident) {
    lastGpu = incident(init.gpuIncident.what);
    pendingGpu = init.gpuIncident.pending ? lastGpu : undefined;
    gpuBlocked = pendingGpu !== undefined;
  }

  /** Whether the option asks for the card, the build has it and nothing blocks it. */
  const wantsCard = (): boolean => options.useGpu && gpu === 'works' && !gpuBlocked;
  /** Whether a loaded model sits on the graphics card. */
  const onCard = (): boolean => wantsCard() && !heldOnCpu;
  /** Bytes of the partial download. */
  let received = 0;
  let watcher: ((status: AdvisorStatusDto) => void) | null = null;

  let downloadTimer: unknown = null;
  let loadTimer: unknown = null;
  let idleTimer: unknown = null;
  let pendingLoad: ((error: ApiRequestError) => void) | null = null;
  let finishLoad: ((status: AdvisorStatusDto) => void) | null = null;
  let problem: string | null = null;
  let job: Job | null = null;
  let tokensPerSecond: number | null = null;

  const status = (): AdvisorStatusDto => {
    const dto: AdvisorStatusDto = {
      state,
      model: { ...MODEL },
      options: { ...options },
      ramTotal: RAM_TOTAL,
      gpuSupported: gpu !== 'none',
      os,
    };

    // A loaded model that had to leave the failing card says so; switching the card off clears the line.
    const fallback = state === 'loaded' && options.useGpu && (gpu === 'fallback' || gpuBlocked || heldOnCpu);

    if (problem !== null) {
      dto.problem = problem;
    } else if (fallback) {
      dto.problem = GPU_FALLBACK_PROBLEM;
    }

    if (state === 'downloading' || (state === 'notDownloaded' && received > 0)) {
      dto.download = { received, total: MODEL.bytes };
    }

    if (state === 'loaded') {
      dto.ramBytes = RAM_LOADED;

      if (onCard()) {
        dto.device = { kind: 'gpu', name: GPU_NAME };
        dto.videoBytes = VIDEO_LOADED;
      } else {
        dto.device = { kind: 'cpu' };
      }

      if (tokensPerSecond !== null) {
        dto.tokensPerSecond = tokensPerSecond;
      }
    }

    if (job?.kind === 'label') {
      dto.labelling = { scan: job.scan, done: job.done, total: job.candidates.length };
    }

    if (pendingGpu) {
      dto.gpuProblem = { ...pendingGpu };
    }

    if (lastGpu) {
      dto.lastGpuProblem = { ...lastGpu };
    }

    return dto;
  };

  const push = (): void => {
    watcher?.(status());
  };

  const stopTimer = (handle: unknown): null => {
    if (handle !== null) {
      timers.clear(handle);
    }

    return null;
  };

  /**
   * Stores the run of a labelling job that has ended, as the backend does: it replaces the labels of the scan. A run
   * that stopped without a single label keeps the earlier ones, and a dropped or outdated scan takes none.
   */
  const storeRun = (ended: LabelJob): void => {
    try {
      host.treeOf(ended.scan);
    } catch {
      return;
    }

    if (host.outdated.has(ended.scan)) {
      return;
    }

    if (ended.run.labels.length === 0 && ended.run.stopped !== undefined) {
      return;
    }

    host.runs.set(ended.scan, ended.run);
  };

  /** Ends the job without a word to its caller beyond what `abort` says; a label run keeps what it has. */
  const dropJob = (abort: ApiRequestError | null): void => {
    const dropped = job;

    job = null;

    if (!dropped) {
      return;
    }

    dropped.timer = stopTimer(dropped.timer);

    if (dropped.kind === 'label') {
      dropped.run.stopped = 'cancelled';
      storeRun(dropped);
    } else if (dropped.stoppedAs) {
      dropped.reject(dropped.stoppedAs);
    } else if (abort) {
      dropped.reject(abort);
    }
  };

  const stopAll = (): void => {
    downloadTimer = stopTimer(downloadTimer);
    loadTimer = stopTimer(loadTimer);
    idleTimer = stopTimer(idleTimer);
    dropJob(new ApiRequestError('cancelled', 'The question was cancelled.'));

    pendingLoad?.(new ApiRequestError('cancelled', 'The load was cancelled.'));
    pendingLoad = null;
    finishLoad = null;
  };

  const unload = (): void => {
    idleTimer = stopTimer(idleTimer);
    state = 'onDisk';
    tokensPerSecond = null;
    heldOnCpu = false;
    push();
  };

  const startIdleTimer = (): void => {
    idleTimer = stopTimer(idleTimer);

    const idleMs = IDLE_MS[options.unloadAfter];

    if (idleMs !== null) {
      idleTimer = timers.set(() => {
        idleTimer = null;
        unload();
      }, idleMs);
    }
  };

  /** A job is over: the model idles again, and the status shows that nothing is running. */
  const endJob = (ended: Job): void => {
    if (job !== ended) {
      return;
    }

    job = null;

    if (state === 'loaded') {
      startIdleTimer();
    }

    push();
  };

  /**
   * Gives the job the model: a model on disk is loaded first (as the backend's worker does), then `next` runs. The
   * idle timer does not run while a job does.
   */
  const withModel = (started: Job, next: () => void): void => {
    job = started;
    idleTimer = stopTimer(idleTimer);

    if (state === 'loaded') {
      // A running labelling shows in the status at once.
      if (started.kind === 'label') {
        push();
      }

      next();
      return;
    }

    state = 'loading';
    problem = null;
    push();
    loadTimer = timers.set(() => {
      loadTimer = null;
      state = 'loaded';
      push();

      if (job === started) {
        next();
      }
    }, LOAD_MS);
  };

  const downloadStep = (): void => {
    received = Math.min(MODEL.bytes, received + Math.ceil(MODEL.bytes / DOWNLOAD_CHUNKS));

    if (received >= MODEL.bytes) {
      downloadTimer = null;
      received = 0;
      state = 'onDisk';
      push();

      return;
    }

    push();
    downloadTimer = timers.set(downloadStep, DOWNLOAD_STEP_MS);
  };

  const seed = (target: MockAdvisorState): void => {
    stopAll();
    received = 0;
    problem = null;

    if (target === 'none') {
      state = 'notDownloaded';
    } else if (target === 'downloading') {
      state = 'downloading';
      received = Math.floor(MODEL.bytes * HELD_SHARE);
    } else if (target === 'failed') {
      // A download that stopped: the partial file stays, the backend's text says why.
      state = 'notDownloaded';
      received = Math.floor(MODEL.bytes * HELD_SHARE);
      problem =
        'The download stopped: the connection dropped. Nothing was lost. It continues where it stopped.';
    } else if (target === 'disk') {
      state = 'onDisk';
    } else {
      state = 'loaded';
      startIdleTimer();
    }

    push();
  };

  applyHookState = seed;
  seed(init.state ?? 'none');

  return {
    advisorEnabled: () => options.enabled,

    async advisorStatus() {
      return status();
    },

    async watchAdvisor(onEvent) {
      // As the backend: the new watcher gets the status at once, so nothing between reading and watching is lost.
      watcher = onEvent;
      push();
    },

    async clearQuestionLog() {
      // As the backend: no log is fine; a link or a folder at the log's path is an `io` error.
      if (init.questionLogBlocked) {
        throw new ApiRequestError('io', 'The question log could not be removed.');
      }
    },

    async answerGpuProblem(id, choice: GpuProblemChoice) {
      // As the backend: a closed choice, and only the problem that is waiting.
      if (choice !== 'useCpu' && choice !== 'tryAgain') {
        return invalid('The choice is not valid.');
      }

      if (pendingGpu === undefined || pendingGpu.id !== id) {
        return invalid('That graphics card problem is not waiting for an answer.');
      }

      pendingGpu = undefined;

      if (choice === 'tryAgain') {
        // The card is tried at the next load; a model already on the processor stays there.
        heldOnCpu = state === 'loaded';
        gpuBlocked = false;
      }

      push();

      return status();
    },

    async clearGpuLog() {
      // As the backend: no log is fine; a problem that waits for an answer stays for this session.
      lastGpu = undefined;
      push();
    },

    async configureAdvisor(next) {
      if (!validOptions(next)) {
        return invalid('The advisor options are not valid.');
      }

      const changed =
        next.enabled !== options.enabled ||
        next.unloadAfter !== options.unloadAfter ||
        next.logQuestions !== options.logQuestions ||
        (next.useGpu ?? true) !== options.useGpu;
      const wasOnCard = onCard();

      options = {
        enabled: next.enabled,
        unloadAfter: next.unloadAfter,
        logQuestions: next.logQuestions,
        useGpu: next.useGpu ?? true,
      };

      // As the backend: with the card switched off, a problem that waits for an answer is answered `useCpu`.
      const answersPending = !options.useGpu && pendingGpu !== undefined;

      if (answersPending) {
        pendingGpu = undefined;
      }

      // Switching the model off ends a labelling or a question in progress.
      if (!options.enabled) {
        dropJob(new ApiRequestError('invalid', 'The local model is switched off in the settings.'));
      }

      // As the backend: a change of the options alone is pushed too.
      if (changed || answersPending) {
        push();
      }

      if (!options.enabled && state === 'loading') {
        // As the backend: the load ends on disk with a problem, and the load call answers with that status.
        loadTimer = stopTimer(loadTimer);
        const finish = finishLoad;
        pendingLoad = null;
        finishLoad = null;
        problem = 'The local model is switched off in the settings.';
        unload();
        finish?.(status());
      } else if (!options.enabled && state === 'loaded') {
        unload();
      } else if (state === 'loaded' && wasOnCard !== wantsCard()) {
        // As the backend: a model that is not where the options put it is unloaded; the next load chooses again.
        dropJob(new ApiRequestError('cancelled', 'The model was unloaded to change where it runs.'));
        unload();
      } else if (state === 'loaded' && changed && job === null) {
        startIdleTimer();
      }

      return status();
    },

    async startModelDownload() {
      if (state !== 'notDownloaded') {
        return invalid(`The model cannot be downloaded while it is ${state}.`);
      }
      if (declinePause !== null) {
        return invalid('The download was just declined, so wait a moment.');
      }

      // The backend's native dialog: declined, nothing starts and the model stays as it was, and for a moment
      // a new request is refused without asking (the backend also allows one dialog at a time).
      if (!confirmDownload()) {
        declinePause = timers.set(() => {
          declinePause = null;
        }, DECLINE_PAUSE_MS);
        throw new ApiRequestError('cancelled', 'The download was not confirmed.');
      }

      state = 'downloading';
      problem = null;
      push();
      downloadTimer = timers.set(downloadStep, DOWNLOAD_STEP_MS);
    },

    async cancelModelDownload() {
      if (state !== 'downloading') {
        return;
      }

      downloadTimer = stopTimer(downloadTimer);
      state = 'notDownloaded';
      push();
    },

    loadModel() {
      // As the backend: loading a loaded model answers with its status.
      if (state === 'loaded') {
        return Promise.resolve(status());
      }
      if (state === 'onDisk' && !options.enabled) {
        return Promise.reject(
          new ApiRequestError('invalid', 'The local model is switched off in the settings.'),
        );
      }
      if (state !== 'onDisk') {
        return Promise.reject(
          new ApiRequestError('invalid', `The model cannot be loaded while it is ${state}.`),
        );
      }

      state = 'loading';
      problem = null;
      push();

      return new Promise<AdvisorStatusDto>((resolve, reject) => {
        pendingLoad = reject;
        finishLoad = resolve;
        loadTimer = timers.set(() => {
          loadTimer = null;
          pendingLoad = null;
          finishLoad = null;
          state = 'loaded';
          startIdleTimer();
          push();
          resolve(status());
        }, LOAD_MS);
      });
    },

    async unloadModel() {
      // As the backend: unloading a model that is not in memory answers with its status.
      if (state === 'onDisk' || state === 'notDownloaded') {
        return status();
      }
      if (state !== 'loaded') {
        return invalid(`The model cannot be unloaded while it is ${state}.`);
      }
      if (job !== null) {
        return invalid('The model cannot be unloaded while it is busy.');
      }

      unload();

      return status();
    },

    async labelFolders(scan) {
      const tree = host.treeOf(scan);

      rulesUsable();
      if (host.isRunning(scan)) {
        return invalid('The scan is still running.');
      }
      if (host.outdated.has(scan)) {
        return invalid('The disk changed since this scan, so rescan first.');
      }
      if (job !== null) {
        return invalid(holdsModel(job) ? BUSY : 'The model is busy with another job.');
      }
      if (!options.enabled) {
        return invalid('The local model is switched off in the settings.');
      }
      if (state !== 'onDisk' && state !== 'loaded') {
        return invalid(
          state === 'notDownloaded'
            ? 'The model is not on disk yet.'
            : `Folders cannot be labelled while the model is ${state}.`,
        );
      }

      const run: LabelRun = { labels: [], ms: 0 };
      const candidates = searchCandidates(tree, ruleView(tree, host.userRules())).candidates;

      // As the backend: with nothing to ask about the run is empty and replaces the earlier one, and no model is loaded.
      if (candidates.length === 0) {
        host.runs.set(scan, run);

        return;
      }

      const started: LabelJob = {
        kind: 'label',
        scan,
        candidates,
        run,
        done: 0,
        cancelled: false,
        timer: null,
      };

      const labelNext = (): void => {
        if (job !== started) {
          return;
        }

        started.timer = null;

        try {
          host.treeOf(scan);
        } catch {
          // The scan was dropped: its labels went with it.
          endJob(started);
          return;
        }

        const node = started.candidates[started.done];

        if (node) {
          run.labels.push({ node: node.id, ...judge(node.name) });
          run.ms += LABEL_STEP_MS;
          started.done++;
          tokensPerSecond = TOKENS_PER_SECOND;
        }

        if (started.done < started.candidates.length) {
          push();
          started.timer = timers.set(labelNext, LABEL_STEP_MS);

          return;
        }

        storeRun(started);
        endJob(started);
      };

      withModel(started, () => {
        if (started.cancelled) {
          run.stopped = 'cancelled';
          storeRun(started);
          endJob(started);

          return;
        }

        started.timer = timers.set(labelNext, LABEL_STEP_MS);
      });
    },

    async cancelLabelling() {
      if (job?.kind !== 'label') {
        return;
      }

      // While the model is still loading the job ends when it has loaded.
      if (job.timer === null) {
        job.cancelled = true;

        return;
      }

      const stopped = job;

      stopped.timer = stopTimer(stopped.timer);
      stopped.run.stopped = 'cancelled';
      storeRun(stopped);
      endJob(stopped);
    },

    async getAdvisorLabels(scan): Promise<AdvisorLabelsDto> {
      const tree = host.treeOf(scan);
      const user = host.userRules();
      const view = ruleView(tree, user);
      const run = host.runs.get(scan);
      const { candidates, refused } = searchCandidates(tree, view);

      const labels = (run?.labels ?? [])
        .map((label): LabelDto | null => {
          const node = tree.nodes[label.node];

          return node
            ? {
                node: node.id,
                path: host.pathOf(tree, node),
                allocated: node.allocated,
                verdict: label.verdict,
                reason: label.reason,
                ...decidedOf(user, view.path(node)),
              }
            : null;
        })
        .filter((label): label is LabelDto => label !== null)
        .sort((a, b) => b.allocated - a.allocated || a.node - b.node);

      return {
        scan,
        labelled: labels.length,
        candidates: candidates.length,
        labels,
        modelMs: run?.ms ?? 0,
        ...(run?.stopped ? { stopped: run.stopped } : {}),
        rules: view.summary(),
        model: stepOf(
          labels.length,
          labels.reduce((sum, label) => sum + label.allocated, 0),
          labels[0] && { path: labels[0].path, verdict: labels[0].verdict },
        ),
        guard: stepOf(
          refused.length,
          refused.reduce((sum, node) => sum + node.allocated, 0),
          refused[0] && { path: host.pathOf(tree, refused[0]), verdict: 'keep' },
        ),
      };
    },

    async askAdvisor(scan, node, units = 'GB'): Promise<AdvisorAnswerDto> {
      const tree = host.treeOf(scan);
      const target = tree.nodes[node];

      if (host.isRunning(scan)) {
        return fail('notFound', `No such scan: ${scan}`);
      }
      if (!target) {
        return fail('notFound', `No such node: ${node}`);
      }
      if (target.kind !== 'dir') {
        return invalid('Only a folder can be asked about.');
      }
      rulesUsable();

      // The rules alone answer: no model, so nothing waits and nothing is refused for the model's sake.
      const view = ruleView(tree, host.userRules());

      return folderAnswer({
        tree,
        node: target,
        path: host.pathOf(tree, target),
        view,
        guard: guardReason(tree, target, view),
        labels: host.runs.get(scan)?.labels ?? [],
        units,
        summary: (folder) => host.summary(tree, folder),
      });
    },

    async askScan(scan, question, units, pick): Promise<ScanAnswerDto> {
      const text = question.trim();

      if (text === '') {
        return invalid('the question is empty');
      }
      if (Array.from(text).length > MAX_QUESTION_CHARS) {
        return invalid(`the question is longer than ${MAX_QUESTION_CHARS} characters`);
      }

      host.treeOf(scan);

      if (host.isRunning(scan)) {
        return fail('notFound', `No such scan: ${scan}`);
      }
      rulesUsable();

      // As the backend's `check_can_ask`: labelling, then a busy model (a load, a download, another job or question),
      // then the model switched off, then not on disk.
      if (job?.kind === 'label') {
        return invalid('folders are being labelled, so ask again when that is done');
      }
      if (job !== null || state === 'downloading' || state === 'loading') {
        return invalid(BUSY);
      }
      if (!options.enabled) {
        return invalid('the local model is switched off in the settings');
      }
      if (state !== 'onDisk' && state !== 'loaded') {
        return invalid('the model is not on disk yet');
      }

      /** The answer from the scan as it is now: the scan may have been dropped meanwhile, which is a `notFound`. */
      const answerNow = (): ScanAnswerDto => {
        const current = host.treeOf(scan);
        const view = ruleView(current, host.userRules());

        return answerQuestion(
          {
            scan,
            tree: current,
            user: host.userRules(),
            labels: host.runs.get(scan)?.labels ?? [],
            outdated: host.outdated.has(scan),
            units,
            lookup: lookupOf(current),
            guard: (node) => guardReason(current, node, view),
            summary: (node) => host.summary(current, node),
          },
          text,
          pick,
        );
      };

      // The words settle what it asks: no model reads it, so nothing waits.
      if (readsWithoutModel(text)) {
        return answerNow();
      }

      return new Promise<ScanAnswerDto>((resolve, reject) => {
        const started: Job = {
          kind: 'ask',
          reject,
          timer: null,
          free: true,
          stoppedAs: new ApiRequestError('invalid', 'the model was stopped before it read the question'),
        };

        withModel(started, () => {
          started.timer = timers.set(() => {
            started.timer = null;
            tokensPerSecond = TOKENS_PER_SECOND;
            endJob(started);

            try {
              resolve(answerNow());
            } catch (error) {
              reject(error);
            }
          }, ASK_MS);
        });
      });
    },

    async findFolders(scan, query, limit): Promise<FolderMatchDto[]> {
      const tree = host.treeOf(scan);

      if (utf8Length(query) > MAX_QUERY_BYTES) {
        return invalid(`The search text must not be longer than ${MAX_QUERY_BYTES} bytes.`);
      }

      const wanted = Math.min(MAX_FOUND, Math.max(1, Math.trunc(Number.isFinite(limit) ? limit : 1)));

      return searchFolders(tree, ruleView(tree, host.userRules()), query, wanted).map((node) => ({
        node: node.id,
        path: host.pathOf(tree, node),
        allocated: node.allocated,
      }));
    },

    async findEntries(scan, query, limit): Promise<EntryMatchDto[]> {
      const tree = host.treeOf(scan);

      if (utf8Length(query) > MAX_QUERY_BYTES) {
        return invalid(`The search text must not be longer than ${MAX_QUERY_BYTES} bytes.`);
      }

      const wanted = Math.min(MAX_FOUND, Math.max(1, Math.trunc(Number.isFinite(limit) ? limit : 1)));

      return searchEntries(tree, ruleView(tree, host.userRules()), query, wanted).map((node) => ({
        node: node.id,
        path: host.pathOf(tree, node),
        allocated: node.allocated,
        kind: node.kind === 'file' ? 'file' : 'dir',
      }));
    },
  };
}
