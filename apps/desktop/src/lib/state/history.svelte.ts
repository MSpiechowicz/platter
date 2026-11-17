import { api as defaultApi } from '../api';
import { describeError, isApiError, type Api } from '../api/client';
import type { HistoryOverviewDto, UndoDto } from '../api/types';
import { ui as defaultUi, type UiStore } from './ui.svelte';

/** How many days back the History view looks. */
export const HISTORY_DAYS = 90;

/** What went wrong with one Restore: a text, and the items that could not be put back. */
export interface RestoreFailure {
  message: string;
  items: UndoDto[];
}

/**
 * What the History view shows: the overview the backend answered with, and the Restore of a cleanup. It reads the
 * overview each time the view opens (`load`), and keeps the last one on screen while it does.
 */
export class HistoryStore {
  data = $state<HistoryOverviewDto | undefined>(undefined);
  /** Why the last read of the overview failed. An older overview may still be on screen. */
  error = $state<string | undefined>(undefined);
  loading = $state(false);
  /** The cleanup being restored. Only one Restore runs at a time: the backend refuses a second. */
  restoring = $state<number | undefined>(undefined);
  /** What went wrong with the last Restore of a cleanup, by cleanup id; a new try or a success clears it. */
  failures = $state<Record<number, RestoreFailure>>({});

  readonly #api: Api;
  readonly #ui: Pick<UiStore, 'toast'>;

  constructor(api: Api = defaultApi, ui: Pick<UiStore, 'toast'> = defaultUi) {
    this.#api = api;
    this.#ui = ui;
  }

  /**
   * Reads the overview. A call while one is reading does not send a second request: it waits for that one and then
   * reads once more, so the answer reflects everything done before the call (the backend refuses a third read).
   */
  load(days = HISTORY_DAYS): Promise<void> {
    if (this.#busy) {
      this.#again = days;

      return this.#reading;
    }

    this.#busy = true;
    this.#reading = this.#read(days);

    return this.#reading;
  }

  /** Set while `#read` loops. Cleared in the same step the loop ends, so no call can join a read that is over. */
  #busy = false;
  #reading: Promise<void> = Promise.resolve();
  #again: number | undefined;

  async #read(days: number): Promise<void> {
    this.loading = true;

    try {
      for (let next: number | undefined = days; next !== undefined; next = this.#takeAgain()) {
        try {
          this.data = await this.#api.historyOverview(next);
          this.error = undefined;
        } catch (reason) {
          this.error = describeError(reason);
        }
      }
    } finally {
      this.#busy = false;
      this.loading = false;
    }
  }

  #takeAgain(): number | undefined {
    const days = this.#again;

    this.#again = undefined;

    return days;
  }

  /** Puts back what a cleanup moved to the trash, then reads the overview again so the row shows the new status. */
  async restore(cleanup: number): Promise<void> {
    if (this.restoring !== undefined) {
      return;
    }

    this.restoring = cleanup;
    this.#forget(cleanup);

    try {
      const answer = await this.#api.restoreHistoryCleanup(cleanup);
      const failed = answer.items.filter((item) => !item.restored);

      if (failed.length > 0) {
        this.failures = {
          ...this.failures,
          [cleanup]: {
            message: `${failed.length} of ${answer.items.length} items could not be put back.`,
            items: failed,
          },
        };
      }

      if (answer.restored > 0) {
        const noun = answer.restored === 1 ? 'item' : 'items';
        this.#ui.toast(`Restored ${answer.restored} ${noun}`, 'They are back where they were.');
      }
    } catch (reason) {
      // The user said no in the backend's own dialog: nothing happened, and nothing is said about it.
      if (!isApiError(reason) || reason.code !== 'cancelled') {
        this.failures = { ...this.failures, [cleanup]: { message: describeError(reason), items: [] } };
      }
    }

    this.restoring = undefined;
    await this.load(this.data?.days);
  }

  #forget(cleanup: number): void {
    const { [cleanup]: _gone, ...rest } = this.failures;

    this.failures = rest;
  }
}

export const history = new HistoryStore();
