import { api as defaultApi } from '../api';
import { describeError, type Api } from '../api/client';
import type { UpdateCheck, UpdateProgress } from '../api/types';
import { formatAgo } from '../format';
import { fmt } from './fmt.svelte';
import { settings, type UpdateSchedule } from './settings.svelte';
import { ui as defaultUi, type UiStore } from './ui.svelte';

export interface UpdateInfo {
  current: string;
  next: string;
  /** "14 MB", or empty when the release does not say. */
  size: string;
  /** What is new, one short line each. */
  notes: string[];
  /** "2 hours ago" */
  checked: string;
  /** This copy installs the update itself. Otherwise something else does (a package manager), and the release page is offered. */
  canInstall: boolean;
}

export type UpdatePhase = 'available' | 'downloading' | 'installing' | 'ready';

/** What the notice's buttons ask for: `open` is the release page, for an update this copy cannot install itself. */
export type UpdateAction = 'install' | 'open' | 'later' | 'restart';

/** What the last check found, for the Settings card. */
export type UpdateResult =
  { kind: 'upToDate' } | { kind: 'found'; version: string } | { kind: 'error'; message: string };

/** The key of the time of the last successful check, in unix milliseconds. */
export const UPDATE_CHECKED_KEY = 'platter.updateCheckedAt';

/** Wait this long after start before the first check, so it never competes with the window opening. */
export const FIRST_CHECK_DELAY_MS = 10_000;

/** How often the schedule looks at whether a check is due. */
export const DUE_CHECK_INTERVAL_MS = 30 * 60_000;

const HOUR_MS = 60 * 60_000;

/** The time between two automatic checks. `never` has none. */
const SCHEDULE_MS: Record<Exclude<UpdateSchedule, 'never'>, number> = {
  sixHours: 6 * HOUR_MS,
  daily: 24 * HOUR_MS,
  weekly: 7 * 24 * HOUR_MS,
};

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

/** localStorage can be missing or throw (private mode, quota). */
function defaultStorage(): StorageLike | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

export interface UpdaterDeps {
  api?: Api;
  ui?: Pick<UiStore, 'toast'>;
  storage?: StorageLike | null;
  schedule?: () => UpdateSchedule;
  /** Whether an update a check found is installed without a click (the Settings switch). On Windows installing only downloads and checks the setup, which runs on the restart click. */
  autoInstall?: () => boolean;
  /** The time in unix milliseconds. */
  now?: () => number;
}

/** State of the update checks, the sidebar update notice and the Updates card in Settings. */
export class UpdaterStore {
  /** The update to offer. Null until a check finds one, and after "Later". */
  info = $state<UpdateInfo | null>(null);
  phase = $state<UpdatePhase>('available');
  /** Download progress, 0 to 1. Not meaningful while `indeterminate`. */
  progress = $state(0);
  /** The download or the install has no measure of how far it is. */
  indeterminate = $state(false);
  /** A check is waiting for the server. */
  checking = $state(false);
  /** What the last check found, whoever asked for it. An automatic check that fails leaves it as it was. */
  result = $state<UpdateResult | null>(null);
  /** The running version, as the last check reported it. */
  /** This copy's version: the build's own until a check answers with the one the backend reports. */
  current = $state<string | null>(__APP_VERSION__);
  /** When the last successful check ran, in unix milliseconds: also one from an earlier session. Null when none is known. */
  checkedAt = $state<number | null>(null);
  /**
   * Whether this copy installs updates itself, as the last check that found one reported. Null until one was
   * found. False means this copy cannot (a package manager installs them) and the release page is offered.
   */
  canInstall = $state<boolean | null>(null);

  readonly #api: Api;
  readonly #ui: Pick<UiStore, 'toast'>;
  readonly #storage: StorageLike | null;
  readonly #schedule: () => UpdateSchedule;
  readonly #autoInstall: () => boolean;
  readonly #now: () => number;

  #timers: { first: ReturnType<typeof setTimeout>; due: ReturnType<typeof setInterval> } | null = null;

  constructor({
    api = defaultApi,
    ui = defaultUi,
    storage = defaultStorage(),
    schedule = () => settings.updateCheck,
    autoInstall = () => settings.autoInstallUpdates,
    now = Date.now,
  }: UpdaterDeps = {}) {
    this.#api = api;
    this.#ui = ui;
    this.#storage = storage;
    this.#schedule = schedule;
    this.#autoInstall = autoInstall;
    this.#now = now;
    this.checkedAt = this.#readCheckedAt();
  }

  /**
   * Asks the release server. An automatic check never shows a failure: it is tried again at the next due check.
   * A manual one puts the failure in `result`. A check is skipped while another runs or an update is installing.
   */
  async check({ manual = false }: { manual?: boolean } = {}): Promise<void> {
    if (this.checking || this.phase === 'downloading' || this.phase === 'installing') {
      return;
    }

    this.checking = true;

    try {
      this.#record(await this.#api.checkUpdate());
    } catch (reason) {
      if (manual) {
        this.result = { kind: 'error', message: describeError(reason) };
      }

      return;
    } finally {
      this.checking = false;
    }

    // "Install updates automatically": the download and install start by themselves. The restart stays the
    // user's click. A failure says nothing and is tried again at the next check.
    if (this.#autoInstall() && this.info?.canInstall && this.phase === 'available') {
      void this.install({ quiet: true });
    }
  }

  #record(answer: UpdateCheck): void {
    const now = this.#now();

    this.#writeCheckedAt(now);
    this.checkedAt = now;
    this.current = answer.current;

    const offer = answer.update;

    if (!offer) {
      this.result = { kind: 'upToDate' };

      if (this.phase === 'available') {
        this.info = null;
      }

      return;
    }

    this.result = { kind: 'found', version: offer.version };
    this.canInstall = offer.canInstall;

    // An installed update waits for the restart: a newer check does not take that notice away.
    if (this.phase !== 'available') {
      return;
    }

    this.info = {
      current: answer.current,
      next: offer.version,
      size: offer.sizeBytes === null ? '' : fmt.size(offer.sizeBytes),
      notes: offer.notes,
      checked: formatAgo(0),
      canInstall: offer.canInstall,
    };
  }

  /**
   * Downloads and installs the update the last check found. Needs `info.canInstall`. A `quiet` install (the
   * automatic one) shows no toast when it fails.
   */
  async install({ quiet = false }: { quiet?: boolean } = {}): Promise<void> {
    if (!this.info?.canInstall || this.phase !== 'available') {
      return;
    }

    this.phase = 'downloading';
    this.progress = 0;
    this.indeterminate = false;

    try {
      await this.#api.installUpdate((progress) => this.#progressed(progress));
      this.progress = 1;
      this.indeterminate = false;
      this.phase = 'ready';
    } catch (reason) {
      this.phase = 'available';

      if (!quiet) {
        this.#ui.toast('The update failed', describeError(reason), 'error');
      }
    }
  }

  #progressed(progress: UpdateProgress): void {
    if (progress.kind === 'installing') {
      this.phase = 'installing';
      this.indeterminate = true;

      return;
    }

    if (this.phase !== 'downloading') {
      return;
    }

    if (progress.total === null || progress.total <= 0) {
      this.indeterminate = true;

      return;
    }

    this.indeterminate = false;
    this.progress = Math.min(1, progress.received / progress.total);
  }

  /** Ends the app and starts it again from the installed update. */
  async restart(): Promise<void> {
    try {
      await this.#api.restartApp();
    } catch (reason) {
      this.#ui.toast('Platter could not restart', describeError(reason), 'error');
    }
  }

  /** Opens the release page, for an update this copy cannot install itself. */
  async openReleases(): Promise<void> {
    try {
      await this.#api.openExternal('releases');
    } catch (reason) {
      this.#ui.toast('The page could not be opened', describeError(reason), 'error');
    }
  }

  /** Hides the notice until the next check. */
  later(): void {
    if (this.phase !== 'available') {
      return;
    }

    this.info = null;
  }

  /**
   * Checks 10 seconds after start when a check is due, and then looks every 30 minutes whether one is due. Does
   * nothing when it already runs.
   */
  start(firstCheckDelayMs = FIRST_CHECK_DELAY_MS): void {
    if (this.#timers) {
      return;
    }

    this.#timers = {
      first: setTimeout(() => void this.checkIfDue(), firstCheckDelayMs),
      due: setInterval(() => {
        this.#refreshChecked();
        void this.checkIfDue();
      }, DUE_CHECK_INTERVAL_MS),
    };
  }

  stop(): void {
    if (!this.#timers) {
      return;
    }

    clearTimeout(this.#timers.first);
    clearInterval(this.#timers.due);
    this.#timers = null;
  }

  /** An automatic check when the setting asks for one and the last one is older than its interval. */
  async checkIfDue(): Promise<void> {
    const schedule = this.#schedule();

    if (schedule === 'never') {
      return;
    }

    const last = this.#readCheckedAt();
    // A last check in the future (the clock was set back) does not hold the next one off.
    const age = last === null ? Infinity : this.#now() - last;

    if (age >= 0 && age < SCHEDULE_MS[schedule]) {
      return;
    }

    await this.check({ manual: false });
  }

  /** "checked 2 hours ago" in the notice's tooltip goes stale while the notice stays on screen. */
  #refreshChecked(): void {
    const last = this.#readCheckedAt();

    if (this.info && last !== null) {
      this.info.checked = formatAgo(this.#now() - last);
    }
  }

  #readCheckedAt(): number | null {
    try {
      const saved = Number(this.#storage?.getItem(UPDATE_CHECKED_KEY));

      return Number.isFinite(saved) && saved > 0 ? saved : null;
    } catch {
      return null;
    }
  }

  #writeCheckedAt(time: number): void {
    try {
      this.#storage?.setItem(UPDATE_CHECKED_KEY, String(time));
    } catch {
      // Storage full or blocked: the next start checks again.
    }
  }
}

export const updater = new UpdaterStore();
