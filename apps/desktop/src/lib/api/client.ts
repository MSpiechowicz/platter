import type {
  AdvisorAnswerDto,
  AdvisorLabelsDto,
  AdvisorOptionsDto,
  AdvisorStatusDto,
  ApiError,
  ApiErrorCode,
  ChildrenPage,
  CleanupEvent,
  CleanupId,
  CleanupPlanDto,
  ExternalLink,
  EntryMatchDto,
  FolderMatchDto,
  GpuProblemChoice,
  HistoryOverviewDto,
  HistoryRestoreDto,
  LodNode,
  NodeDto,
  NodeId,
  PathDto,
  PlanId,
  ScanAnswerDto,
  ScanEvent,
  ScanId,
  ScanOptions,
  SizeUnits,
  SortKey,
  SuggestionsDto,
  SummaryDto,
  UpdateCheck,
  UpdateProgress,
  UndoDto,
  UserRulesDto,
  UserVerdict,
  VolumeDto,
} from './types';

/** Id of the synthetic "N smaller items" node returned by `getLod`. It is never a real node. */
export const COLLAPSED_NODE_ID: NodeId = 0xffffffff;

/**
 * The only way the UI talks to the backend. `tauri.ts` implements it over IPC,
 * `mock.ts` implements it in memory so the app runs in a plain browser.
 */
export interface Api {
  listVolumes(): Promise<VolumeDto[]>;

  /** Opens the system folder picker. Resolves to `null` when the user cancels. */
  pickFolder(): Promise<string | null>;

  /**
   * Starts a scan. Resolves with the scan id as soon as the scan is running;
   * progress and the final `finished` / `cancelled` / `failed` event arrive on `onEvent`
   * (possibly before this promise resolves).
   */
  startScan(options: ScanOptions, onEvent: (event: ScanEvent) => void): Promise<ScanId>;
  cancelScan(scan: ScanId): Promise<void>;
  /** Frees the scan tree in the backend. */
  dropScan(scan: ScanId): Promise<void>;

  getNode(scan: ScanId, node: NodeId): Promise<NodeDto>;
  /**
   * One page of a folder's children. With a `query`, only the children whose name contains it (ignoring case) are
   * listed and `total` counts them; a blank query filters nothing, and one over 1024 bytes is `invalid`.
   */
  getChildren(
    scan: ScanId,
    node: NodeId,
    offset: number,
    limit: number,
    sort: SortKey,
    query?: string,
  ): Promise<ChildrenPage>;
  getLod(scan: ScanId, node: NodeId, maxNodes: number, maxDepth: number): Promise<LodNode>;
  getPath(scan: ScanId, node: NodeId): Promise<PathDto>;
  getSummary(scan: ScanId, node: NodeId): Promise<SummaryDto>;
  /**
   * What the cleanup rules would label in a finished scan. Read-only: nothing is changed. The `advisor` suggestion
   * (made from the local model's labels) is listed only while the advisor is switched on.
   */
  getSuggestions(scan: ScanId): Promise<SuggestionsDto>;

  /**
   * Plans the cleanup of the given suggestions (rule ids from `getSuggestions`) of a finished scan. Nothing is
   * changed. `invalid`: no ids, an unknown id, or an outdated scan (a cleanup already ran on it); for `advisor`
   * also the advisor switched off, or no labelled folder that can be cleaned. The labels of a scan are dropped when
   * the disk changes (a cleanup or an undo).
   */
  planCleanup(scan: ScanId, suggestionIds: string[]): Promise<CleanupPlanDto>;
  /**
   * Plans moving the items the user marked in the Explorer (node ids of a finished scan, 1 to 1000) to the trash:
   * one group `marked`, verdict `review`, action `trash`. Nothing is changed. Items the backend will not touch
   * (kept, protected, a link, the scan root, inside another marked item) come back in `refused`. `invalid`: no ids,
   * more than 1000, or an outdated scan; `notFound`: an id that is not in the scan. It is started, confirmed and
   * undone like any other plan.
   */
  planMarkedCleanup(scan: ScanId, nodes: NodeId[]): Promise<CleanupPlanDto>;
  /**
   * Starts a plan; a plan starts once. Resolves with the cleanup id as soon as it is running; `groupStarted`,
   * `targetDone`, `groupFinished` and a last `finished` (or `failed`) arrive on `onEvent`, possibly before this
   * promise resolves. Rejects with `cancelled` when the user declines the backend's own confirmation (asked when
   * the plan has something that cannot be undone), `notFound` for an unknown or used plan, `invalid` for an expired
   * plan, a cleanup already running or an outdated scan.
   */
  startCleanup(plan: PlanId, onEvent: (event: CleanupEvent) => void): Promise<CleanupId>;
  /** Stops a running cleanup after the target in progress. An unknown or ended id is not an error. */
  cancelCleanup(cleanup: CleanupId): Promise<void>;
  /** Puts back what a finished cleanup moved to the trash, once; `notFound` the second time. */
  undoCleanup(cleanup: CleanupId): Promise<UndoDto[]>;

  /**
   * The history of the disk's used space and of the cleanups, for the last `days` days (1 to 366, `invalid`
   * otherwise). The backend picks the volume. A history file that cannot be used is not an error: the answer has a
   * `problem` and nothing is recorded.
   */
  historyOverview(days: number): Promise<HistoryOverviewDto>;
  /**
   * Puts back, once, what one cleanup of the history moved to the trash. `notFound`: unknown cleanup. `invalid`
   * while a cleanup, another restore or an undo runs, and while the history file cannot be used. Items that could
   * not be put back come with an `error` and stay for another try.
   */
  restoreHistoryCleanup(cleanup: number): Promise<HistoryRestoreDto>;

  /** The local model's state, options and memory use. */
  advisorStatus(): Promise<AdvisorStatusDto>;
  /**
   * The one status channel of the advisor: the backend pushes the current status at once, then on every change
   * (options, load, unload, idle unload, download progress, labelling progress as `labelling`). A new subscription replaces the old one.
   */
  watchAdvisor(onEvent: (status: AdvisorStatusDto) => void): Promise<void>;
  /**
   * Tells the backend the advisor options (called at start, before anything is loaded, and when the setting
   * changes); until then the backend treats the model as switched off. Switching the model off unloads it. Options
   * the backend cannot read (an unknown `unloadAfter`, extra fields) are refused before the command runs, so the
   * error is not `invalid`; the mock answers `invalid`.
   */
  configureAdvisor(options: AdvisorOptionsDto): Promise<AdvisorStatusDto>;
  /**
   * Deletes the local question log (`advisor-questions.jsonl` in Platter's data folder). The web view never reads
   * the file. No log is not an error. `io`: a link or a folder is at the log's path, or it cannot be removed.
   */
  clearQuestionLog(): Promise<void>;
  /**
   * Answers the graphics card problem that `AdvisorStatusDto.gpuProblem` names, and answers with the status:
   * `useCpu` keeps the card blocked (the web view also saves `useGpu` off and configures), `tryAgain` lets the next
   * load try the card; a model already on the processor stays there. `invalid`: the id is not the pending problem
   * (already answered, or gone), or the choice is not one of the two.
   */
  answerGpuProblem(id: number, choice: GpuProblemChoice): Promise<AdvisorStatusDto>;
  /**
   * Deletes the graphics card problem log (`advisor-gpu.log` in Platter's data folder), so `lastGpuProblem` goes.
   * The web view never reads the file. No log is not an error. A problem that is still waiting for an answer stays
   * for this session. `io`: a link or a folder is at the log's path, or it cannot be removed.
   */
  clearGpuLog(): Promise<void>;
  /**
   * Starts the opt-in model download, or resumes one that was cancelled; progress arrives through `watchAdvisor`.
   * `invalid` unless the model is `notDownloaded`. The backend asks the user in a native dialog first; when it is
   * declined this rejects with `cancelled` and the model stays `notDownloaded`.
   */
  startModelDownload(): Promise<void>;
  /** Stops a running download and keeps what was received for a later resume. Nothing running is not an error. */
  cancelModelDownload(): Promise<void>;
  /**
   * Checks and loads the downloaded model. A loaded model answers with its status; `invalid` before the download,
   * while downloading or loading, and while the model is switched off. A failed check or load is not an error: the
   * answer is `onDisk` or `notDownloaded` with a `problem`.
   */
  loadModel(): Promise<AdvisorStatusDto>;
  /** Frees the model's memory. A model not in memory answers with its status; `invalid` while downloading or loading. */
  unloadModel(): Promise<AdvisorStatusDto>;
  /**
   * Has the model label the largest folders that no rule covers (at most 50, each of at least 256 MiB) in a finished
   * scan. Resolves as soon as the job is taken; progress arrives through `watchAdvisor` as `labelling`
   * (`done` of `total`), and the labels through `getAdvisorLabels`, which shows them when the run ends. A model on
   * disk is loaded first, and a failure after the job was taken (the model fails to load or verify) is not an error
   * of this call: the status shows it, `labelling` disappears and `problem` is set. A run that stops (cancelled,
   * time budget) or fails with no label keeps the labels of the earlier run; a run that ends normally with none
   * replaces them. A scan with no candidate gets an empty run at once. `notFound`:
   * unknown scan. `invalid`: the scan is still running or outdated (a cleanup ran on it), another model job runs
   * (download, load, labelling or a question), the model is switched off in the options, or no model file is on disk.
   */
  labelFolders(scan: ScanId): Promise<void>;
  /** Stops a running labelling after the folder in progress and keeps the labels made so far. Nothing running is not an error. */
  cancelLabelling(): Promise<void>;
  /**
   * The labels of a scan with the counts for "How a verdict is made": folders and bytes the rules matched, the model
   * labelled and the guard refused or left for the user. Also before any labelling (no labels, `candidates` set).
   * `notFound`: unknown scan.
   */
  getAdvisorLabels(scan: ScanId): Promise<AdvisorLabelsDto>;
  /**
   * What Platter says about one folder (a node id of a scan), from its rules alone: no model call, no model wording,
   * so it works with the model switched off, not downloaded, busy or labelling, and is instant. The answer comes in
   * this order: the guard (the scan root, a protected place, an unreadable folder: source `guard`), then a rule
   * covering the folder or one above it (`rule`; `user` for the user's own rule), then a label made earlier in this
   * scan (`model`: the verdict is the model's label, the text is Platter's), else `unruled` (verdict `review`).
   * `units` is how the size in `detail` is written (`'GB'` when left out). `notFound`: unknown or running scan, or
   * unknown node. `invalid`: not a folder, or the user's rules file is broken.
   */
  askAdvisor(scan: ScanId, node: NodeId, units?: SizeUnits): Promise<AdvisorAnswerDto>;
  /**
   * Answers a free question about a finished scan. The question is text only (never a path, never stored; 1 to 500
   * characters after trimming) and `units` is how sizes are written in the answer text. The backend works out what
   * is asked, finds the folders in the scan's tree and the rules; the model only reads the question (it never
   * sees the facts), and Platter writes the answer from the scan: a direct first sentence, then the rest of the facts.
   * The rows, sizes and verdicts are computed. A question that is not about the scan gets no rows and a fixed text. A scope the scan does not hold gives no rows and a note. A scan a
   * cleanup ran on answers with a note that it is outdated. The model is loaded first if needed. Errors, in order:
   * `invalid` for a blank or too long question; `notFound` for an unknown or running scan; `invalid` for a broken
   * rules file, while folders are being labelled or another model job runs, and when the model is switched off, not
   * on disk or unavailable.
   *
   * A question whose words name what it is about is answered at once, without the model. One that could mean more
   * than one thing (a name that fits two folders, a word that fits two tools) comes back as `clarify`: the prompt and
   * the choices, no rows. Asking the same question again with `pick` set to the `id` of a choice answers about that
   * choice; the web view sends only the text and that id, never a path. A `pick` that fits no choice (any more) is
   * `invalid`: "that choice is no longer available, so ask again".
   */
  askScan(scan: ScanId, question: string, units: SizeUnits, pick?: number): Promise<ScanAnswerDto>;
  /**
   * Looks for folders of a finished scan by name, for a folder picker; it only reads the scan's tree. An exact path
   * (`~/...` or `/...`) comes first, then folders whose name starts with the text, then folders whose name contains
   * it (case ignored), largest first within each group. Files are never listed, and the scan root matches only by
   * its exact path. `query` is at most 1024 bytes (`invalid` otherwise); a blank one (only spaces) finds nothing, and
   * any other is matched as typed, spaces included. `limit` is clamped to 1..20. `notFound`: unknown scan.
   */
  findFolders(scan: ScanId, query: string, limit: number): Promise<FolderMatchDto[]>;
  /**
   * Looks for folders and files of a finished scan by name, for the autocomplete of the Ask card. It only reads the
   * scan's tree. A query that is `~` or starts with `~/` or `/` is a path: the text up to its last `/` must be a folder of the
   * scan (a bare `~` is `~/`) and the result is that folder's children whose name fits the rest, all of them when the
   * rest is empty. Any other query is a name, looked for in the whole tree. Either way an exact name comes first
   * (case ignored), then names that start with the text, then names that contain it, largest first within each group.
   * Only folders and regular files are listed, never a link, and never the scan root. `query` is at most 1024 bytes
   * (`invalid` otherwise), a blank one finds nothing and `limit` is clamped to 1..20. `notFound`: unknown scan.
   */
  findEntries(scan: ScanId, query: string, limit: number): Promise<EntryMatchDto[]>;

  /**
   * The user's own rules, in the order they were made. Each of the four rule commands answers with the whole list.
   * While the rules file is broken the answer has no rules and a `problem`.
   */
  listUserRules(): Promise<UserRulesDto>;
  /**
   * Says `keep` (always keep) or `safe` (safe to clean) about a folder of a finished scan, named by its node id; the
   * backend works out the exact path itself. A folder that already has a rule gets the new verdict. `notFound`:
   * unknown scan or node (also a scan that has not finished). `invalid`: not a folder, a name that is not valid
   * text, `/`, a full list (256 rules), a broken rules file, and for `safe` also the home folder, the scan root and
   * a protected place (the text says which). Changing a rule drops the cleanup plans that were waiting.
   */
  addUserRule(scan: ScanId, node: NodeId, verdict: UserVerdict): Promise<UserRulesDto>;
  /** Removes a rule by its id. An id that is not there changes nothing. `invalid` while the rules file is broken. */
  removeUserRule(id: number): Promise<UserRulesDto>;
  /**
   * Moves a broken rules file aside and starts with no rules. `invalid` when the file is not broken. While it is
   * broken, every cleanup plan (`planCleanup`, `planMarkedCleanup`) is `invalid`; `getSuggestions` still works with
   * the built-in rules.
   */
  resetUserRules(): Promise<UserRulesDto>;

  openInFileManager(scan: ScanId, node: NodeId): Promise<void>;

  /**
   * Asks the release server whether a newer version exists, and keeps the offer for `installUpdate`. `update` is null
   * when this is the newest version. Rejects (`io`) when the server cannot be reached or its answer cannot be used.
   */
  checkUpdate(): Promise<UpdateCheck>;

  /**
   * Downloads the update the last `checkUpdate` found, checks its signature and replaces the AppImage. `onProgress`
   * gets the download, then `installing`. Resolves when it is installed. `invalid` when this copy is not an AppImage
   * (`canInstall` is false) or no check found an update.
   */
  installUpdate(onProgress: (progress: UpdateProgress) => void): Promise<void>;

  /** Ends the app and starts it again, from the installed update. */
  restartApp(): Promise<void>;

  /** Opens one of the fixed project pages in the system browser (`target=_blank` does nothing inside the Tauri window). */
  openExternal(link: ExternalLink): Promise<void>;
}

export class ApiRequestError extends Error implements ApiError {
  readonly code: ApiErrorCode;

  constructor(code: ApiErrorCode, message: string) {
    super(message);
    this.name = 'ApiRequestError';
    this.code = code;
  }
}

const API_ERROR_CODES: readonly string[] = ['notFound', 'cancelled', 'io', 'invalid'];

/** Tauri rejects with a plain `{ code, message }` object, the mock throws `ApiRequestError`. Both match. */
export function isApiError(value: unknown): value is ApiError {
  if (typeof value !== 'object' || value === null) {
    return false;
  }

  const candidate = value as Partial<Record<keyof ApiError, unknown>>;

  return (
    typeof candidate.code === 'string' &&
    API_ERROR_CODES.includes(candidate.code) &&
    typeof candidate.message === 'string'
  );
}

/** A message that is safe to show to the user, whatever was thrown. */
export function describeError(error: unknown): string {
  if (isApiError(error)) {
    return error.message;
  }

  if (error instanceof Error) {
    return error.message;
  }

  return typeof error === 'string' ? error : 'Something went wrong.';
}
