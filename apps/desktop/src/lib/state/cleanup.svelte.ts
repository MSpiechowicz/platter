import { SvelteSet } from 'svelte/reactivity';
import { api as defaultApi } from '../api';
import { describeError, isApiError, type Api } from '../api/client';
import type {
  CleanupEvent,
  CleanupId,
  CleanupPlanDto,
  OutcomeKind,
  PlanGroupDto,
  ScanId,
  SuggestionDto,
  SuggestionsDto,
} from '../api/types';
import { scan as defaultScan, type ScanStore } from './scan.svelte';

/*
 * The cleanup flow of the Overview, with no knowledge of the interface: which suggestions are selected, planning,
 * the review step, running with progress, the result, undo.
 *
 * The backend is the safety gate: it shows its own native confirmation for every plan. The review step here is a
 * summary the interface shows, never a gate. This store only sends rule ids of the user's own selection, starts a
 * cleanup only when a method is called (the interface calls them from a click), and never retries by itself.
 */

/** What the segmented control shows: the selection is exactly the safe ones, all of them, none, or something else. */
export type SelectionChoice = 'safe' | 'all' | 'none' | 'custom';

/** A target that was not cleaned, as the backend described it. */
export interface Problem {
  path: string;
  detail: string;
}

export interface UndoState {
  restored: number;
  failures: { path: string; error: string }[];
  /** Why the undo itself failed; the entries are still there to try again unless it was `notFound`. */
  error?: string;
}

export interface CleanupResult {
  /** `finished` carries counts; `failed` only a message. */
  kind: 'finished' | 'failed';
  scanId: ScanId;
  cleanupId: CleanupId;
  cleaned: number;
  withProblems: number;
  /** Free space after minus before; absent when it could not be measured. */
  freed: number | undefined;
  cancelled: boolean;
  toolsNotRun: string[];
  /** How many targets were moved to the trash. */
  trashed: number;
  message: string | undefined;
  /** The skipped or failed targets, as many as were kept (see `MAX_PROBLEMS`). */
  problems: Problem[];
  /** Undo is offered now: the backend can put something back and it has not been put back yet. */
  undoable: boolean;
  undo: UndoState | undefined;
}

export interface Progress {
  /** Title of the group being cleaned; undefined until the first one starts. */
  group: string | undefined;
  done: number;
  total: number;
}

export type CleanupPhase =
  | { kind: 'idle' }
  | { kind: 'planning' }
  /** The plan is shown to the user in the review dialog. Nothing runs until `confirmReview`. */
  | { kind: 'review'; plan: CleanupPlanDto }
  | { kind: 'running'; progress: Progress; cleanupId: CleanupId | undefined; cancelling: boolean }
  | { kind: 'done'; result: CleanupResult };

export type Action = 'cancel' | 'undo' | 'rescan';

const MAX_PROBLEMS = 20;
const BAD_OUTCOMES: readonly OutcomeKind[] = ['skipped', 'failed'];

export const NOT_CLEANED = 'Not cleaned.';

/** The scan store as the flow needs it. */
export type ScanHost = Pick<ScanStore, 'state' | 'markOutdated' | 'rescan'>;

interface Run {
  plan: CleanupPlanDto;
  cancelRequested: boolean;
  cleanupId: CleanupId | undefined;
  trashed: number;
  problems: Problem[];
  /** Per rule: targets that went well and targets that did not. */
  tally: Map<string, { ok: number; bad: number }>;
}

export class CleanupStore {
  /** The scan the suggestions belong to. */
  scanId = $state<ScanId | undefined>(undefined);
  suggestions = $state<SuggestionDto[]>([]);
  phase = $state<CleanupPhase>({ kind: 'idle' });
  /** One plain sentence about why nothing ran ("Not cleaned.", why a plan could not be made). */
  notice = $state<string | undefined>(undefined);
  /** The last plan's refusals and rules that matched nothing, to be shown as text. */
  plan = $state<CleanupPlanDto | undefined>(undefined);
  /** Bytes freed by every cleanup of this session, from the backend's `freed`; 0 before any. */
  freedTotal = $state(0);

  /** Rule ids the user ticked. */
  readonly selected = new SvelteSet<string>();
  /** Rules a cleanup finished without problems: their rows are done. */
  readonly cleaned = new SvelteSet<string>();

  readonly #api: Api;
  readonly #scan: ScanHost;
  #data: SuggestionsDto | undefined;
  #run: Run | undefined;
  /** Bumped when an answer that is still on its way no longer matters. */
  #epoch = 0;

  /** Rows the user may select: not kept, not already cleaned. */
  readonly cleanable = $derived(
    this.suggestions.filter((item) => item.verdict !== 'keep' && !this.cleaned.has(item.id)),
  );
  readonly safeItems = $derived(this.cleanable.filter((item) => item.verdict === 'safe'));
  readonly selectedItems = $derived(this.cleanable.filter((item) => this.selected.has(item.id)));
  readonly reviewSelected = $derived(this.selectedItems.filter((item) => item.verdict === 'review'));
  readonly selectedBytes = $derived(sum(this.selectedItems));
  readonly reviewBytes = $derived(sum(this.reviewSelected));
  readonly safeBytes = $derived(sum(this.safeItems));

  readonly choice: SelectionChoice = $derived.by(() => {
    if (this.selectedItems.length === 0) {
      return 'none';
    }

    const onlySafe =
      this.safeItems.length > 0 &&
      this.selectedItems.length === this.safeItems.length &&
      this.safeItems.every((item) => this.selected.has(item.id));

    if (onlySafe) {
      return 'safe';
    }

    return this.selectedItems.length === this.cleanable.length ? 'all' : 'custom';
  });

  constructor(api: Api = defaultApi, scan: ScanHost = defaultScan) {
    this.#api = api;
    this.#scan = scan;
  }

  /** A cleanup ran on this scan: it no longer describes the disk. */
  get outdated(): boolean {
    const state = this.#scan.state;

    return state.status === 'ready' && state.scanId === this.scanId && state.outdated;
  }

  /** Planning, the review step or a run is going on: a click starts nothing, the selection stays as it is. */
  get busy(): boolean {
    return this.phase.kind !== 'idle' && this.phase.kind !== 'done';
  }

  get canCleanSafe(): boolean {
    return !this.busy && !this.outdated && this.scanId !== undefined && this.safeItems.length > 0;
  }

  get canCleanSelected(): boolean {
    return !this.busy && !this.outdated && this.scanId !== undefined && this.selectedItems.length > 0;
  }

  /** The plan the review dialog shows, while it is open. */
  get reviewing(): CleanupPlanDto | undefined {
    return this.phase.kind === 'review' ? this.phase.plan : undefined;
  }

  /** What the footer offers to click while a result or a run is shown. */
  get actions(): Action[] {
    const phase = this.phase;

    if (phase.kind === 'running') {
      // While the backend's own dialog is open, that dialog is where to say no: no second Cancel here.
      const acting = phase.cleanupId !== undefined || phase.progress.group !== undefined;
      return phase.cancelling || !acting ? [] : ['cancel'];
    }

    if (phase.kind === 'done') {
      return phase.result.undoable ? ['undo', 'rescan'] : ['rescan'];
    }

    return this.outdated ? ['rescan'] : [];
  }

  isSelected(id: string): boolean {
    return this.selected.has(id);
  }

  /**
   * The suggestions of a scan arrived (or arrived again). Safe rows are ticked, review rows are not; a new scan
   * forgets the result of the last cleanup. A run in progress is left alone.
   */
  load(scanId: ScanId, data: SuggestionsDto | undefined): void {
    const newScan = scanId !== this.scanId;

    if (!newScan && data === this.#data) {
      return;
    }

    this.scanId = scanId;
    this.#data = data;
    this.suggestions = data?.suggestions ?? [];

    if (newScan) {
      this.cleaned.clear();

      if (this.phase.kind !== 'running') {
        // A plan or an answer for another scan means nothing here.
        this.#epoch++;
        this.phase = { kind: 'idle' };
        this.notice = undefined;
        this.plan = undefined;
      }
    }

    this.selected.clear();

    for (const item of this.safeItems) {
      this.selected.add(item.id);
    }
  }

  toggle(id: string): void {
    if (this.busy || this.outdated || !this.cleanable.some((item) => item.id === id)) {
      return;
    }

    if (this.selected.has(id)) {
      this.selected.delete(id);
    } else {
      this.selected.add(id);
    }
  }

  /** "All" is every row that can be cleaned: kept rows are never part of it. */
  select(choice: 'safe' | 'all' | 'none'): void {
    if (this.busy || this.outdated) {
      return;
    }

    this.selected.clear();

    const chosen = choice === 'safe' ? this.safeItems : choice === 'all' ? this.cleanable : [];

    for (const item of chosen) {
      this.selected.add(item.id);
    }
  }

  /** Clean every safe item. The selection becomes exactly those rows, so the ids sent are the ones shown ticked. */
  async cleanSafe(): Promise<void> {
    if (!this.canCleanSafe) {
      return;
    }

    this.select('safe');
    await this.#begin(this.safeItems.map((item) => item.id));
  }

  async cleanSelected(): Promise<void> {
    if (!this.canCleanSelected) {
      return;
    }

    await this.#begin(this.selectedItems.map((item) => item.id));
  }

  /** "Clean <total>" in the review dialog. */
  async confirmReview(): Promise<void> {
    const phase = this.phase;

    if (phase.kind === 'review') {
      await this.#start(phase.plan);
    }
  }

  /** Cancel, Escape or a click outside the review dialog. The plan is dropped; the backend lets it expire. */
  dismissReview(): void {
    if (this.phase.kind === 'review') {
      this.#epoch++;
      this.phase = { kind: 'idle' };
      this.notice = NOT_CLEANED;
    }
  }

  /** Stops a running cleanup after the target in progress. */
  async cancel(): Promise<void> {
    const phase = this.phase;
    const run = this.#run;

    if (phase.kind !== 'running' || phase.cancelling || !run) {
      return;
    }

    this.phase = { ...phase, cancelling: true };

    if (phase.cleanupId === undefined) {
      // The id is not known yet; the request is sent as soon as it is.
      run.cancelRequested = true;
      return;
    }

    await this.#cancelRun(phase.cleanupId);
  }

  /** Puts back what the last cleanup moved to the trash. Failures stay in the result as text. */
  async undo(): Promise<void> {
    const phase = this.phase;

    if (phase.kind !== 'done' || !phase.result.undoable) {
      return;
    }

    const result = phase.result;

    // Taken off at once, so a second click cannot start a second undo.
    this.phase = { kind: 'done', result: { ...result, undoable: false } };

    try {
      const entries = await this.#api.undoCleanup(result.cleanupId);
      const failures = entries
        .filter((entry) => !entry.restored)
        .map((entry) => ({ path: entry.path, error: entry.error ?? 'could not be put back' }));

      // Entries that failed stay in the backend for another try.
      this.#after(result, {
        undo: { restored: entries.length - failures.length, failures },
        undoable: failures.length > 0,
      });
      this.#scan.markOutdated(result.scanId);
    } catch (error) {
      const gone = isApiError(error) && error.code === 'notFound';

      this.#after(result, {
        undo: { restored: 0, failures: [], error: describeError(error) },
        undoable: !gone,
      });
    }
  }

  async rescan(): Promise<void> {
    await this.#scan.rescan();
  }

  #after(result: CleanupResult, change: Partial<CleanupResult>): void {
    // The result on screen may have been replaced meanwhile (a new scan).
    if (this.phase.kind === 'done' && this.phase.result.cleanupId === result.cleanupId) {
      this.phase = { kind: 'done', result: { ...this.phase.result, ...change } };
    }
  }

  async #begin(ruleIds: string[]): Promise<void> {
    const scanId = this.scanId;

    if (scanId === undefined || ruleIds.length === 0) {
      return;
    }

    const epoch = ++this.#epoch;

    this.notice = undefined;
    this.plan = undefined;
    this.phase = { kind: 'planning' };

    let plan: CleanupPlanDto;

    try {
      plan = await this.#api.planCleanup(scanId, ruleIds);
    } catch (error) {
      if (epoch === this.#epoch) {
        this.phase = { kind: 'idle' };
        this.notice = `Could not plan the cleanup: ${describeError(error)}`;
      }

      return;
    }

    if (epoch !== this.#epoch) {
      return;
    }

    this.plan = plan;

    if (plan.groups.length === 0) {
      this.phase = { kind: 'idle' };
      this.notice = 'Nothing from this selection can be cleaned.';
      return;
    }

    if (plan.groups.some((group) => group.verdict === 'review')) {
      this.phase = { kind: 'review', plan };
      return;
    }

    await this.#start(plan);
  }

  async #start(plan: CleanupPlanDto): Promise<void> {
    const run: Run = {
      plan,
      cancelRequested: false,
      cleanupId: undefined,
      trashed: 0,
      problems: [],
      // Bookkeeping of one run, never rendered.
      // eslint-disable-next-line svelte/prefer-svelte-reactivity
      tally: new Map(),
    };

    this.#run = run;
    this.notice = undefined;
    this.phase = {
      kind: 'running',
      progress: {
        group: undefined,
        done: 0,
        total: plan.groups.reduce((sum, group) => sum + group.items, 0),
      },
      cleanupId: undefined,
      cancelling: false,
    };

    try {
      const id = await this.#api.startCleanup(plan.planId, (event) => this.#onEvent(run, event));

      // The last event may have arrived before the id did.
      if (this.#run !== run) {
        return;
      }

      run.cleanupId = id;

      if (this.phase.kind === 'running') {
        this.phase = { ...this.phase, cleanupId: id };
      }

      if (run.cancelRequested) {
        await this.#cancelRun(id);
      }
    } catch (error) {
      if (this.#run === run) {
        this.#run = undefined;
        this.phase = { kind: 'idle' };
        this.notice = notStarted(error);
      }
    }
  }

  async #cancelRun(id: CleanupId): Promise<void> {
    try {
      await this.#api.cancelCleanup(id);
    } catch {
      // The run has ended already, or the backend refused: its own events say how it went.
    }
  }

  #onEvent(run: Run, event: CleanupEvent): void {
    if (this.#run !== run) {
      return;
    }

    switch (event.type) {
      case 'groupStarted':
        this.#progress({ group: titleOf(run.plan.groups, event.ruleId) });
        break;

      case 'targetDone': {
        const tally = run.tally.get(event.ruleId) ?? { ok: 0, bad: 0 };
        const bad = BAD_OUTCOMES.includes(event.outcome.kind) || event.outcome.problems > 0;

        if (bad) {
          tally.bad++;
        } else {
          tally.ok++;
        }

        run.tally.set(event.ruleId, tally);

        if (event.outcome.kind === 'trashed') {
          run.trashed++;
        }

        if (BAD_OUTCOMES.includes(event.outcome.kind) && run.problems.length < MAX_PROBLEMS) {
          run.problems.push({ path: event.path, detail: event.outcome.detail ?? event.outcome.kind });
        }

        this.#progress({ done: this.phase.kind === 'running' ? this.phase.progress.done + 1 : 0 });
        break;
      }

      case 'groupFinished':
        break;

      case 'finished':
        this.#end(run, {
          kind: 'finished',
          scanId: event.outdatedScan,
          cleanupId: event.cleanupId,
          cleaned: event.cleaned,
          withProblems: event.withProblems,
          freed: event.freed,
          cancelled: event.cancelled,
          toolsNotRun: event.toolsNotRun,
          message: undefined,
          undoable: event.canUndo,
        });
        break;

      case 'failed':
        this.#end(run, {
          kind: 'failed',
          scanId: event.outdatedScan,
          cleanupId: event.cleanupId,
          cleaned: 0,
          withProblems: 0,
          freed: undefined,
          cancelled: false,
          toolsNotRun: [],
          message: event.message,
          undoable: event.canUndo,
        });
        break;
    }
  }

  #progress(change: Partial<Progress>): void {
    if (this.phase.kind === 'running') {
      this.phase = { ...this.phase, progress: { ...this.phase.progress, ...change } };
    }
  }

  /** Whatever the end was, the scan no longer describes the disk. */
  #end(run: Run, result: Omit<CleanupResult, 'trashed' | 'problems' | 'undo'>): void {
    this.#run = undefined;
    this.#scan.markOutdated(result.scanId);

    if (result.freed !== undefined) {
      this.freedTotal += result.freed;
    }

    for (const [ruleId, tally] of run.tally) {
      if (tally.ok > 0 && tally.bad === 0) {
        this.cleaned.add(ruleId);
        this.selected.delete(ruleId);
      }
    }

    this.phase = {
      kind: 'done',
      result: { ...result, trashed: run.trashed, problems: run.problems, undo: undefined },
    };
  }
}

const sum = (items: SuggestionDto[]): number => items.reduce((total, item) => total + item.allocated, 0);

const titleOf = (groups: PlanGroupDto[], ruleId: string): string =>
  groups.find((group) => group.ruleId === ruleId)?.title ?? ruleId;

/** Why `startCleanup` said no. Nothing is retried: the next attempt is the user's next click. */
function notStarted(error: unknown): string {
  if (isApiError(error)) {
    switch (error.code) {
      case 'cancelled':
        return NOT_CLEANED;
      case 'notFound':
        return 'This plan is gone: it was used or it expired. Click again to make a new one.';
      case 'invalid':
        return `Not started: ${error.message}`;
    }
  }

  return `Could not start the cleanup: ${describeError(error)}`;
}

export const cleanup = new CleanupStore();
