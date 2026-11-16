import type {
  ApiError,
  ApiErrorCode,
  ChildrenPage,
  ExternalLink,
  LodNode,
  NodeDto,
  NodeId,
  PathDto,
  ScanEvent,
  ScanId,
  ScanOptions,
  SortKey,
  SuggestionsDto,
  SummaryDto,
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
  getChildren(
    scan: ScanId,
    node: NodeId,
    offset: number,
    limit: number,
    sort: SortKey,
  ): Promise<ChildrenPage>;
  getLod(scan: ScanId, node: NodeId, maxNodes: number, maxDepth: number): Promise<LodNode>;
  getPath(scan: ScanId, node: NodeId): Promise<PathDto>;
  getSummary(scan: ScanId, node: NodeId): Promise<SummaryDto>;
  /** What the cleanup rules would label in a finished scan. Read-only: nothing is changed. */
  getSuggestions(scan: ScanId): Promise<SuggestionsDto>;

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
