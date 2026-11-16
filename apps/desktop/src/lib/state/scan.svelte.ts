import { api as defaultApi } from '../api';
import { describeError, isApiError, type Api } from '../api/client';
import type { NodeId, ScanEvent, ScanId, ScanOptions, Totals, VolumeDto } from '../api/types';

export interface ScanProgress {
  files: number;
  dirs: number;
  bytes: number;
  errors: number;
  currentPath: string;
}

export type ScanState =
  | { status: 'idle' }
  | { status: 'scanning'; options: ScanOptions; progress: ScanProgress; cancelling: boolean }
  | {
      status: 'ready';
      options: ScanOptions;
      scanId: ScanId;
      rootId: NodeId;
      totals: Totals;
      elapsedMs: number;
    }
  | { status: 'cancelled'; options: ScanOptions }
  | { status: 'failed'; options: ScanOptions; message: string };

export type RunningScan = Extract<ScanState, { status: 'scanning' }>;
export type ReadyScan = Extract<ScanState, { status: 'ready' }>;

export type LoadStatus = 'idle' | 'loading' | 'ready' | 'error';

const NO_PROGRESS: ScanProgress = { files: 0, dirs: 0, bytes: 0, errors: 0, currentPath: '' };

/** The volume that is exactly this root, or the one whose mount point contains it (the deepest wins). */
export function findVolume(
  volumes: VolumeDto[],
  root: string,
): { volume: VolumeDto; exact: boolean } | undefined {
  const exact = volumes.find((volume) => volume.mountPoint === root);

  if (exact) {
    return { volume: exact, exact: true };
  }

  const containing = volumes
    .filter((volume) => volume.mountPoint === '/' || root.startsWith(`${volume.mountPoint}/`))
    .sort((a, b) => b.mountPoint.length - a.mountPoint.length)[0];

  return containing ? { volume: containing, exact: false } : undefined;
}

/**
 * The scan lifecycle: idle, scanning (with progress), then ready, cancelled or failed.
 * Only one scan runs at a time; starting a new one frees the previous tree in the backend.
 */
export class ScanStore {
  volumes = $state<VolumeDto[]>([]);
  volumesStatus = $state<LoadStatus>('idle');
  state = $state<ScanState>({ status: 'idle' });

  readonly #api: Api;
  #generation = 0;
  #scanId: ScanId | null = null;
  #cancelRequested = false;

  constructor(api: Api = defaultApi) {
    this.#api = api;
  }

  /** The options of the current or last scan, if there is one. */
  get options(): ScanOptions | null {
    return this.state.status === 'idle' ? null : this.state.options;
  }

  /** The volume the scan root belongs to, and whether the root is that volume itself. */
  get volumeInfo(): { volume: VolumeDto; exact: boolean } | undefined {
    const options = this.options;

    return options ? findVolume(this.volumes, options.root) : undefined;
  }

  async loadVolumes(): Promise<void> {
    this.volumesStatus = 'loading';

    try {
      this.volumes = await this.#api.listVolumes();
      this.volumesStatus = 'ready';
    } catch {
      this.volumesStatus = 'error';
    }
  }

  async start(options: ScanOptions): Promise<void> {
    if (this.state.status === 'scanning') {
      return;
    }

    this.#release();

    const generation = ++this.#generation;

    this.#cancelRequested = false;
    this.state = { status: 'scanning', options, progress: NO_PROGRESS, cancelling: false };

    try {
      const id = await this.#api.startScan(options, (event) => this.#onEvent(generation, options, event));

      if (generation !== this.#generation) {
        this.#drop(id);
        return;
      }

      // Events may already have moved the state on while the id was on its way.
      const status = this.#status();

      if (status === 'scanning') {
        this.#scanId = id;

        if (this.#cancelRequested) {
          await this.#cancelScan(id);
        }
      } else if (status !== 'ready') {
        // The scan ended (cancelled or failed) before its id was known.
        this.#drop(id);
      }
    } catch (error) {
      if (generation === this.#generation) {
        this.state = { status: 'failed', options, message: describeError(error) };
      }
    }
  }

  /** Asks the backend to stop. The state turns `cancelled` when the backend confirms. */
  async cancel(): Promise<void> {
    if (this.state.status !== 'scanning' || this.state.cancelling) {
      return;
    }

    this.state = { ...this.state, cancelling: true };
    this.#cancelRequested = true;

    if (this.#scanId !== null) {
      const generation = this.#generation;

      try {
        await this.#cancelScan(this.#scanId);
      } catch (error) {
        // Only a scan that is still running can fail here: a state the scan has already ended in stays.
        if (generation === this.#generation && this.state.status === 'scanning') {
          this.state = { status: 'failed', options: this.state.options, message: describeError(error) };
        }
      }
    }
  }

  /** Asks the backend to stop one scan. "No such scan" means it has already ended, which is what was wanted. */
  async #cancelScan(id: ScanId): Promise<void> {
    try {
      await this.#api.cancelScan(id);
    } catch (error) {
      if (!isApiError(error) || error.code !== 'notFound') {
        throw error;
      }
    }
  }

  /** Scans the same root again with the same options. */
  async rescan(): Promise<void> {
    const options = this.options;

    if (options) {
      await this.start(options);
    }
  }

  /** Back to the start screen, freeing the scan. */
  reset(): void {
    if (this.state.status === 'scanning') {
      return;
    }

    this.#release();
    this.#generation++;
    this.state = { status: 'idle' };
  }

  #status(): ScanState['status'] {
    return this.state.status;
  }

  #onEvent(generation: number, options: ScanOptions, event: ScanEvent): void {
    if (generation !== this.#generation) {
      return;
    }

    switch (event.type) {
      case 'progress':
        if (this.state.status === 'scanning') {
          const { type: _type, ...progress } = event;
          this.state = { ...this.state, progress };
        }
        break;

      case 'finished':
        this.#scanId = event.scanId;
        this.state = {
          status: 'ready',
          options,
          scanId: event.scanId,
          rootId: event.rootId,
          totals: event.totals,
          elapsedMs: event.elapsedMs,
        };
        break;

      case 'cancelled':
        this.#release();
        this.state = { status: 'cancelled', options };
        break;

      case 'failed':
        this.#release();
        this.state = { status: 'failed', options, message: event.message };
        break;
    }
  }

  /** Frees the backend tree of the current scan, if any. */
  #release(): void {
    if (this.#scanId !== null) {
      this.#drop(this.#scanId);
      this.#scanId = null;
    }
  }

  #drop(id: ScanId): void {
    this.#api.dropScan(id).catch(() => {
      // Already gone; nothing to free.
    });
  }
}

export const scan = new ScanStore();
