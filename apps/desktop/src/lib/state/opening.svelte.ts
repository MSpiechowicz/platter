import { formatAgo } from '../format';
import { hooks } from '../hooks';
import { LOADING_MIN_VISIBLE_MS } from './delayed.svelte';
import { fmt } from './fmt.svelte';
import { motion } from './motion.svelte';

/** Every step stays running at least this long (the project rule for a spinner that has appeared), so it can be read. */
export const STEP_MIN_MS = LOADING_MIN_VISIBLE_MS;

/** The logo draws itself once over this long, and the landing waits until it is complete. */
export const LOGO_DRAW_MS = 2000;

/** A beat after the last step, so the last tick is seen before the screen leaves. */
export const LAST_BEAT_MS = 500;

/** The logo flies to the menu, and the menu slides in, over this long. */
export const LAND_MS = 700;
export const LAND_EASING = 'cubic-bezier(0.65, 0, 0.35, 1)';

/** The opening screen fades out over this long while the logo flies. */
export const FADE_MS = 300;

/** The landing waits for the background start-up (the system is known from it) for at most this long. */
export const READY_GRACE_MS = 1500;

/**
 * How long a step may run before the app opens without it. A saved scan of a network folder whose server is gone can
 * hang for good, and the opening screen must not stay over an app it makes unusable. The step's work goes on: a scan
 * that opens later still takes the app to its Overview, as a finished scan does.
 */
export const STEP_LIMIT_MS = 10_000;

/** What a step says when it ran out of time. */
export const SLOW_NOTE = 'taking long, opening anyway';

export type StepId = 'disks' | 'last' | 'current';
export type StepStatus = 'waiting' | 'running' | 'done' | 'skipped';

/** `opening`: only the opening screen. `landing`: the logo flies and the app slides in. `done`: the app as usual. */
export type OpeningPhase = 'opening' | 'landing' | 'done';

export interface OpeningStep {
  id: StepId;
  label: string;
  status: StepStatus;
  /** What the step found, shown when it is done. */
  note?: string;
  /** The note is a warning (amber). */
  warn: boolean;
}

/** The scan kept from the last run, as far as the opening screen tells. */
export interface SavedScanSummary {
  /** What the scan holds on the disk, in bytes. */
  bytes: number;
  /** When it was scanned, in unix seconds. */
  scannedAt: number;
  /** A cleanup ran since: the scan no longer describes the disk. */
  outdated: boolean;
}

export interface OpeningTasks {
  /** Loads the disks. Resolves to how many there are. */
  disks: () => Promise<number>;
  /** Opens the scan kept from the last run. Resolves to what it holds, or null when there is none. */
  lastScan: () => Promise<SavedScanSummary | null>;
  /** Starts a new scan of the opened one ("Rescan when Platter opens"). Left out when the user did not ask for it. */
  rescan?: () => Promise<void>;
  /** Start-up work that goes on in the background. The landing waits for it, but only for `READY_GRACE_MS`. */
  ready?: Promise<unknown>;
}

interface StepNote {
  text: string;
  warn?: boolean;
}

const LABELS: Record<StepId, string> = {
  disks: 'Checking your disks',
  last: 'Looking for your last scan',
  current: 'Checking it is still current',
};

function freshSteps(): OpeningStep[] {
  return (Object.keys(LABELS) as StepId[]).map((id) => ({
    id,
    label: LABELS[id],
    status: 'waiting',
    warn: false,
  }));
}

/** Test hooks and screenshots go straight to the app, as they did before there was an opening screen. */
function initialPhase(): OpeningPhase {
  return hooks.view !== null || hooks.static ? 'done' : 'opening';
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Resolves when `promise` settles (either way) or after `ms`, whichever comes first. */
function settledWithin(promise: Promise<unknown> | undefined, ms: number): Promise<void> {
  if (!promise) {
    return Promise.resolve();
  }

  return Promise.race([promise.then(noop, noop), sleep(ms)]);
}

function noop(): void {}

function diskNote(count: number): StepNote {
  return { text: `${count} ${count === 1 ? 'disk' : 'disks'}` };
}

function scanNote(saved: SavedScanSummary): StepNote {
  return { text: `${fmt.size(saved.bytes)} · ${formatAgo(Date.now() - saved.scannedAt * 1000)}` };
}

/**
 * What the app is doing before its first screen: three steps (disks, the last scan, whether it is current) with what
 * each found, and the phase that takes the opening screen to the app. `bootstrap` drives it with `run`.
 */
class OpeningState {
  phase = $state<OpeningPhase>(initialPhase());
  steps = $state<OpeningStep[]>(freshSteps());

  /** Straight to the app, without the opening screen. */
  skip(): void {
    this.phase = 'done';
  }

  /** The opening screen again, with its steps waiting (for tests). */
  reset(phase: OpeningPhase = initialPhase()): void {
    this.phase = phase;
    this.steps = freshSteps();
  }

  /** The logo has landed on the menu: the opening screen goes, and the menu shows its own logo. */
  finish(): void {
    this.phase = 'done';
  }

  /**
   * Runs the steps in order, each at least `STEP_MIN_MS` on screen, then waits a beat and starts the landing. It
   * resolves when the landing starts. A step that fails counts as done, so the app always opens: with no saved scan
   * the start screen is what lands.
   */
  async run(tasks: OpeningTasks): Promise<void> {
    const startedAt = Date.now();

    await this.#step('disks', tasks.disks, 0, diskNote);

    const saved = await this.#step('last', tasks.lastScan, null, (found) =>
      found ? scanNote(found) : { text: 'none kept' },
    );

    if (saved) {
      await this.#step(
        'current',
        async () => (tasks.rescan ? await tasks.rescan() : undefined),
        undefined,
        () => {
          if (tasks.rescan) {
            return { text: 'rescanning' };
          }

          return saved.outdated
            ? { text: 'outdated, rescan to refresh', warn: true }
            : { text: 'up to date' };
        },
      );
    } else {
      this.#find('current').status = 'skipped';
    }

    await sleep(LAST_BEAT_MS);

    // The first page needs the system's name (Recycle Bin or trash), which comes with the background start-up. It is
    // awaited with or without the landing, but only so long.
    await settledWithin(tasks.ready, READY_GRACE_MS);

    if (motion.reduced) {
      this.phase = 'done';
      return;
    }

    // The logo is drawn once, and it must be whole before it flies.
    const drawing = startedAt + LOGO_DRAW_MS - Date.now();

    if (drawing > 0) {
      await sleep(drawing);
    }

    this.phase = 'landing';
  }

  #find(id: StepId): OpeningStep {
    const step = this.steps.find((candidate) => candidate.id === id);

    if (!step) {
      throw new Error(`No opening step ${id}`);
    }

    return step;
  }

  /** Runs `work` as a step: running while it works and for at least `STEP_MIN_MS`, then done with its note. */
  async #step<T>(
    id: StepId,
    work: () => Promise<T>,
    fallback: T,
    describe: (value: T) => StepNote,
  ): Promise<T> {
    const step = this.#find(id);
    const began = Date.now();
    let value = fallback;
    let note: StepNote | undefined;

    step.status = 'running';

    const outOfTime = Symbol('out of time');
    let limit: ReturnType<typeof setTimeout> | undefined;

    try {
      const work$ = work();
      // A late failure, after the step gave up on it, is nobody's to hear.
      work$.catch(() => undefined);

      const result = await Promise.race([
        work$,
        new Promise<typeof outOfTime>((resolve) => {
          limit = setTimeout(() => resolve(outOfTime), STEP_LIMIT_MS);
        }),
      ]);

      if (result === outOfTime) {
        note = { text: SLOW_NOTE, warn: true };
      } else {
        value = result;
        note = describe(value);
      }
    } catch {
      // A step that fails is done without a note: the app opens with what it has.
    } finally {
      clearTimeout(limit);
    }

    const left = began + STEP_MIN_MS - Date.now();

    if (left > 0) {
      await sleep(left);
    }

    step.note = note?.text;
    step.warn = note?.warn ?? false;
    step.status = 'done';

    return value;
  }
}

export const opening = new OpeningState();
