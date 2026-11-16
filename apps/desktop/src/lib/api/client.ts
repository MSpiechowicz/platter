import type {
  ApiError,
  ApiErrorCode,
  ChildrenPage,
  CleanupEvent,
  CleanupId,
  CleanupPlanDto,
  ExternalLink,
  LodNode,
  NodeDto,
  NodeId,
  PathDto,
  PlanId,
  ScanEvent,
  ScanId,
  ScanOptions,
  SortKey,
  SuggestionsDto,
  SummaryDto,
  UndoDto,
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
  /** What the cleanup rules would label in a finished scan. Read-only: nothing is changed. */
  getSuggestions(scan: ScanId): Promise<SuggestionsDto>;

  /**
   * Plans the cleanup of the given suggestions (rule ids from `getSuggestions`) of a finished scan. Nothing is
   * changed. `invalid`: no ids, an unknown id, or an outdated scan (a cleanup already ran on it).
   */
  planCleanup(scan: ScanId, suggestionIds: string[]): Promise<CleanupPlanDto>;
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

  openInFileManager(scan: ScanId, node: NodeId): Promise<void>;

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
