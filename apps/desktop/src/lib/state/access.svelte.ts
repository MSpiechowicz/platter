import { api as defaultApi } from '../api';
import { describeError, type Api } from '../api/client';
import type { DiskAccess } from '../api/types';
import { ui as defaultUi, type UiStore } from './ui.svelte';

/**
 * Whether macOS lets Platter into every folder (Full Disk Access), for the notice that asks for it. The app is signed
 * ad hoc, so macOS takes every update for a new app and the access has to be given again.
 *
 * The check runs at start and each time the window comes back into focus: a user who switched the access on in
 * System Settings comes back to a page without the notice. "Not now" hides it until the next start.
 */
export class DiskAccessStore {
  /** What the last check said. `unknown` until it answers, and when it could not tell: no notice either way. */
  status = $state<DiskAccess>('unknown');

  /** The user chose "Not now". Not saved: the notice is back at the next start. */
  dismissed = $state(false);

  readonly #api: Api;
  readonly #ui: Pick<UiStore, 'toast'>;
  /** How many checks were started: only the newest one's answer counts. */
  #asked = 0;
  #stop: (() => void) | undefined;

  constructor({ api = defaultApi, ui = defaultUi }: { api?: Api; ui?: Pick<UiStore, 'toast'> } = {}) {
    this.#api = api;
    this.#ui = ui;
  }

  /** The notice is on screen. */
  get shown(): boolean {
    return this.status === 'missing' && !this.dismissed;
  }

  /** Asks again. A failed check keeps what the last one said, and an answer that a newer check overtook is dropped. */
  async check(): Promise<void> {
    const asked = ++this.#asked;

    try {
      const status = await this.#api.diskAccess();

      if (asked === this.#asked) {
        this.status = status;
      }
    } catch {
      // Nothing to tell the user: without an answer there is no notice to change.
    }
  }

  /**
   * Checks now, and again each time the window gets the focus back or is shown again. Returns the function that
   * stops it. Starting again stops the earlier start first, so the checks never pile up.
   */
  start(target: Pick<Window, 'addEventListener' | 'removeEventListener'> = window): () => void {
    this.#stop?.();

    const recheck = (): void => void this.check();
    const shown = (): void => {
      if (typeof document === 'undefined' || document.visibilityState === 'visible') {
        recheck();
      }
    };

    void this.check();
    target.addEventListener('focus', recheck);
    target.addEventListener('visibilitychange', shown);

    const stop = (): void => {
      target.removeEventListener('focus', recheck);
      target.removeEventListener('visibilitychange', shown);

      if (this.#stop === stop) {
        this.#stop = undefined;
      }
    };

    this.#stop = stop;

    return stop;
  }

  /** "Not now": hides the notice until the next start. */
  later(): void {
    this.dismissed = true;
  }

  /** Opens the Full Disk Access list of System Settings. */
  async openSettings(): Promise<void> {
    try {
      await this.#api.openDiskAccessSettings();
    } catch (reason) {
      this.#ui.toast('System Settings could not be opened', describeError(reason), 'error');
    }
  }
}

export const diskAccess = new DiskAccessStore();
