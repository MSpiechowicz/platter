import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import { api as defaultApi } from '../api';
import { describeError, isApiError, type Api } from '../api/client';
import type {
  CleanupOptions,
  CleanupEvent,
  CleanupId,
  CleanupPlanDto,
  NodeId,
  OutcomeKind,
  PlanGroupDto,
  ScanId,
  SizeUnits,
  SuggestionDto,
  SuggestionsDto,
  Verdict,
} from '../api/types';
import { scan as defaultScan, type ScanStore } from './scan.svelte';
import { settings } from './settings.svelte';
import { visibleText } from '../features/overview/display';
import { ui as defaultUi, type UiStore } from './ui.svelte';

/*
 * The cleanup flow, with no knowledge of the interface: which suggestions are selected (Overview, Cleanup view) or
 * which items are marked (Explorer), planning, the review step, running with progress, the result, undo. Marked
 * items and suggestions share the one flow: a marked plan is just another plan.
 *
 * The backend is the safety gate: it shows its own native confirmation for every plan. The review step here is a
 * summary the interface shows, never a gate. This store only sends rule ids and node ids of the user's own
 * selection and marks (never a path), starts a cleanup only when a method is called (the interface calls them from
 * a click), and never retries by itself.
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

/** Where a rule of the running (or last) cleanup stands: not reached yet, finished cleanly, or finished with problems. */
export type GroupState = 'queued' | 'done' | 'problem';

/** The rule id the backend gives the group of a marked plan. */
export const MARKED_RULE = 'marked';
/** The most items one marked plan may name (the backend refuses more). */
export const MAX_MARKS = 1000;

const MAX_PROBLEMS = 20;
const BAD_OUTCOMES: readonly OutcomeKind[] = ['skipped', 'failed'];

export const NOT_CLEANED = 'Not cleaned.';

/** The scan store as the flow needs it. */
export type ScanHost = Pick<ScanStore, 'state' | 'markOutdated' | 'rescan'>;

/** An item the user marked in the Explorer. Only its id is ever sent; the rest is for totals and for nesting. */
export interface Mark {
  id: NodeId;
  /** The bytes it holds, from the scan. */
  allocated: number;
  /** The folders above it, the scan root first. A mark with none is the scan root, which cannot be marked. */
  ancestors: readonly NodeId[];
}

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
  /** The suggestions of `scanId` have arrived. Until then an empty list means "not known", not "none". */
  suggestionsKnown = $state(false);
  phase = $state<CleanupPhase>({ kind: 'idle' });
  /** One plain sentence about why nothing ran ("Not cleaned.", why a plan could not be made). */
  notice = $state<string | undefined>(undefined);
  /** The last plan's refusals and rules that matched nothing, to be shown as text. */
  plan = $state<CleanupPlanDto | undefined>(undefined);
  /** Bytes freed by every cleanup of this session, from the backend's `freed`; 0 before any. */
  freedTotal = $state(0);
  /** The hand-marked group of the running or last cleanup (`ruleId: "marked"`), when its plan had one. */
  markedRun = $state<PlanGroupDto | undefined>(undefined);

  /** Rule ids the user ticked. */
  readonly selected = new SvelteSet<string>();
  /** What the user marked in the Explorer, by node id. They belong to `scanId` and go with it. */
  readonly marks = new SvelteMap<NodeId, Mark>();
  /** Rules a cleanup finished without problems: their rows are done. */
  readonly cleaned = new SvelteSet<string>();
  /**
   * The rules of the running or last cleanup, each with the state of its group as the backend's events report it.
   * It is what a view shows while a cleanup runs, whatever the selection says by then.
   */
  readonly groups = new SvelteMap<string, GroupState>();

  readonly #api: Api;
  readonly #scan: ScanHost;
  readonly #ui: Pick<UiStore, 'toast'>;
  #data: SuggestionsDto | undefined;
  #run: Run | undefined;
  #stamp = 0;
  #units: SizeUnits = settings.units;
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

  /** The nodes a `keep` suggestion lists: what the Explorer shows as kept. */
  readonly keptNodes: ReadonlySet<NodeId> = $derived(nodesOf(this.suggestions, 'keep'));
  /** The nodes a `safe` suggestion lists: what the Explorer shows as safe to clean. */
  readonly safeNodes: ReadonlySet<NodeId> = $derived(nodesOf(this.suggestions, 'safe'));

  /** The marks that are not inside another mark: what a cleanup acts on, and what the total counts. */
  readonly outermostMarks: Mark[] = $derived(
    [...this.marks.values()].filter((mark) => !mark.ancestors.some((above) => this.marks.has(above))),
  );
  readonly markedBytes = $derived(this.outermostMarks.reduce((total, mark) => total + mark.allocated, 0));

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

  constructor(api: Api = defaultApi, scan: ScanHost = defaultScan, ui: Pick<UiStore, 'toast'> = defaultUi) {
    this.#api = api;
    this.#scan = scan;
    this.#ui = ui;
  }

  /** A cleanup that could not be planned or started says so in a toast: the view keeps showing the selection. */
  #fail(title: string, description: string): void {
    this.notice = undefined;
    // What the backend said may hold paths: hidden characters are shown, not acted on.
    this.#ui.toast(title, visibleText(description), 'error');
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

  /** Marks can change: nothing runs, the scan still describes the disk, and there is room for one more. */
  get canMark(): boolean {
    return !this.busy && !this.outdated && this.scanId !== undefined;
  }

  get canCleanMarked(): boolean {
    return this.canMark && this.outermostMarks.length > 0;
  }

  /** The most items are marked: another one cannot be added until one is unmarked. */
  get markFull(): boolean {
    return this.marks.size >= MAX_MARKS;
  }

  /**
   * The group of what the user marked by hand in the plan under review, or in the cleanup that runs or ran. The
   * suggestions know nothing of it, so the Cleanup view draws it from here.
   */
  get markedGroup(): PlanGroupDto | undefined {
    const plan = this.reviewing;

    return plan ? markedGroupOf(plan) : this.markedRun;
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

  isMarked(id: NodeId): boolean {
    return this.marks.has(id);
  }

  /**
   * Whether a keep suggestion covers an item: it lists the item, or a folder above it (`ancestors`, the scan root
   * first). Conservative: a keep suggestion lists at most its 20 largest matches, so an item it covers but does not
   * list is not found here, and the backend refuses it when it is planned. A folder that merely holds something kept
   * is not covered either.
   */
  isKept(id: NodeId, ancestors: readonly NodeId[]): boolean {
    return this.keptNodes.has(id) || ancestors.some((above) => this.keptNodes.has(above));
  }

  /**
   * The Explorer shows this scan: marks of another scan are forgotten, and so is what an earlier look at the
   * suggestions said ("Not cleaned.") when nothing is running: the footer is about the marks.
   */
  adopt(scanId: ScanId): void {
    if (scanId !== this.scanId) {
      this.load(scanId, undefined);
    } else if (this.phase.kind === 'idle') {
      this.notice = undefined;
      this.plan = undefined;
    }
  }

  /** Makes sure the suggestions of the scan are here (the Explorer needs the kept and the safe nodes). */
  async ensureSuggestions(scanId: ScanId): Promise<void> {
    if (this.scanId === scanId && this.#data !== undefined) {
      return;
    }

    let data: SuggestionsDto;

    try {
      data = await this.#api.getSuggestions(scanId, settings.units);
    } catch {
      // Nothing is shown as kept or safe; the backend still refuses what it must.
      return;
    }

    // The scan may have been replaced, or the Overview may have loaded them meanwhile.
    if (this.scanId === scanId && this.#data === undefined) {
      this.load(scanId, data);
    }
  }

  /** Marks an item for cleanup. Does nothing while a cleanup runs, once the scan is outdated, for a kept item or the scan root. */
  mark(mark: Mark): void {
    const known = this.marks.has(mark.id);

    if (!this.canMark || mark.ancestors.length === 0 || this.isKept(mark.id, mark.ancestors)) {
      return;
    }

    if (!known && this.markFull) {
      return;
    }

    this.marks.set(mark.id, { ...mark, ancestors: [...mark.ancestors] });
    this.#marksChanged();
  }

  unmark(id: NodeId): void {
    if (this.canMark && this.marks.delete(id)) {
      this.#marksChanged();
    }
  }

  /** What was said about the last plan no longer fits the marks. */
  #marksChanged(): void {
    if (this.phase.kind === 'idle') {
      this.notice = undefined;
      this.plan = undefined;
    }
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

    // The same scan with other suggestions (the advisor's labels or on/off changed): the rows the user ticked stay
    // ticked while they still exist, and a safe row that is new is ticked like the others.
    const refetched = !newScan && this.#data !== undefined && data !== undefined;
    const known = this.suggestions.map((item) => item.id);

    this.scanId = scanId;
    this.#data = data;
    this.suggestions = data?.suggestions ?? [];
    this.suggestionsKnown = data !== undefined;

    if (newScan) {
      this.cleaned.clear();
      this.groups.clear();
      this.markedRun = undefined;
      this.marks.clear();

      if (this.phase.kind !== 'running') {
        // A plan or an answer for another scan means nothing here.
        this.#epoch++;
        this.phase = { kind: 'idle' };
        this.notice = undefined;
        this.plan = undefined;
      }
    }

    // The same scan again while a cleanup runs (a page that was left and opened again): the selection is the
    // user's, and the cleanup's own groups say what is going on.
    if (!newScan && this.phase.kind === 'running') {
      return;
    }

    if (refetched) {
      for (const id of [...this.selected]) {
        if (!this.cleanable.some((item) => item.id === id)) {
          this.selected.delete(id);
        }
      }

      for (const item of this.safeItems) {
        if (!known.includes(item.id)) {
          this.selected.add(item.id);
        }
      }

      return;
    }

    this.selected.clear();

    for (const item of this.safeItems) {
      this.selected.add(item.id);
    }
  }

  /**
   * The number the pages watch to fetch the suggestions again: the advisor's version, held still while a cleanup
   * runs (a list that changes under a plan would confuse it) and let go when it ends.
   */
  stampFor(version: number): number {
    if (!this.busy) {
      this.#stamp = version;
    }

    return this.#stamp;
  }

  /**
   * The units the pages fetch the suggestions in, so a change of them in the settings fetches them again: held still
   * while a cleanup runs, like `stampFor`, and let go when it ends.
   */
  unitsFor(units: SizeUnits): SizeUnits {
    if (!this.busy) {
      this.#units = units;
    }

    return this.#units;
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
    const ids = this.safeItems.map((item) => item.id);

    await this.#begin((scan) => this.#api.planCleanup(scan, ids, this.#options()));
  }

  async cleanSelected(): Promise<void> {
    if (!this.canCleanSelected) {
      return;
    }

    const ids = this.selectedItems.map((item) => item.id);

    await this.#begin((scan) => this.#api.planCleanup(scan, ids, this.#options()));
  }

  /**
   * Clean what is marked. Only the outermost marks are sent: one inside another is part of the outer one, and the
   * backend would leave it out anyway.
   */
  async cleanMarked(): Promise<void> {
    if (!this.canCleanMarked) {
      return;
    }

    const nodes = this.outermostMarks.map((mark) => mark.id);

    await this.#begin((scan) => this.#api.planMarkedCleanup(scan, nodes, this.#options()));
  }

  /** The Cleanup settings, fixed into the plan the backend makes now. */
  #options(): CleanupOptions {
    return { keepPackageVersions: settings.keepPackageVersions, askForAdmin: settings.askForAdmin };
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
      // The user said no in the backend's own dialog: nothing happened, nothing is said, and Undo stays offered.
      if (isApiError(error) && error.code === 'cancelled') {
        this.#after(result, { undoable: true });

        return;
      }

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

  async #begin(planner: (scan: ScanId) => Promise<CleanupPlanDto>): Promise<void> {
    const scanId = this.scanId;

    if (scanId === undefined) {
      return;
    }

    const epoch = ++this.#epoch;

    this.notice = undefined;
    this.plan = undefined;
    this.phase = { kind: 'planning' };

    let plan: CleanupPlanDto;

    try {
      plan = await planner(scanId);
    } catch (error) {
      if (epoch === this.#epoch) {
        this.phase = { kind: 'idle' };
        this.#fail('Could not plan the cleanup', describeError(error));
      }

      return;
    }

    if (epoch !== this.#epoch) {
      return;
    }

    this.plan = plan;

    if (plan.groups.length === 0) {
      this.phase = { kind: 'idle' };
      this.#fail('Nothing could be cleaned', refusedSummary(plan));
      return;
    }

    if (plan.refused.length + plan.refusedMore > 0) {
      // The rest is cleaned: the toast only says what stays, the footer keeps to the run.
      this.#ui.toast('Part of the selection is left out', visibleText(refusedSummary(plan)), 'info');
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
    this.groups.clear();
    this.markedRun = markedGroupOf(plan);

    for (const group of plan.groups) {
      this.groups.set(group.ruleId, 'queued');
    }

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
        const why = notStarted(error);

        if (why === NOT_CLEANED) {
          this.notice = why;
        } else {
          this.#fail('The cleanup did not start', why);
        }
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
        this.#progress({ group: progressTitle(run.plan.groups, event.ruleId) });
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

      case 'groupFinished': {
        const tally = run.tally.get(event.ruleId);

        this.groups.set(event.ruleId, tally && tally.ok > 0 && tally.bad === 0 ? 'done' : 'problem');
        break;
      }

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

    // A cancelled run never reached the groups still queued: they are not part of what was cleaned.
    for (const [ruleId, state] of [...this.groups]) {
      if (state === 'queued') {
        this.groups.delete(ruleId);
      }
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

const markedGroupOf = (plan: CleanupPlanDto): PlanGroupDto | undefined =>
  plan.groups.find((group) => group.ruleId === MARKED_RULE);

const nodesOf = (items: SuggestionDto[], verdict: Verdict): Set<NodeId> =>
  // A derived value, rebuilt when the suggestions change: not a reactive set.
  new Set(items.filter((item) => item.verdict === verdict).flatMap((item) => item.nodes));

const sum = (items: SuggestionDto[]): number => items.reduce((total, item) => total + item.allocated, 0);

const titleOf = (groups: PlanGroupDto[], ruleId: string): string =>
  groups.find((group) => group.ruleId === ruleId)?.title ?? ruleId;

const asAdministrator = (group: PlanGroupDto): boolean => group.action === 'runTool' && group.needsElevation;

/**
 * What the progress line names while a group runs. The backend runs every system tool that needs the administrator in
 * one go, when the first of them is reached: that group's line names them all, as they all run then.
 */
export function progressTitle(groups: PlanGroupDto[], ruleId: string): string {
  const elevated = groups.filter(asAdministrator);
  const first = elevated[0];

  if (elevated.length > 1 && first?.ruleId === ruleId) {
    return elevated.map((group) => group.title).join(', ');
  }

  return titleOf(groups, ruleId);
}

/** Why a plan holds nothing to clean, in a sentence or two: how many items were refused and the most common reason. */
export function refusedSummary(plan: CleanupPlanDto): string {
  const refused = plan.refused.length + plan.refusedMore;

  if (refused === 0) {
    return 'The selected rules found nothing on the disk.';
  }

  // A local tally, never rendered.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const counts = new Map<string, number>();

  for (const item of plan.refused) {
    counts.set(item.reason, (counts.get(item.reason) ?? 0) + 1);
  }

  const [reason] = [...counts.entries()].sort((a, b) => b[1] - a[1])[0];
  const items = refused === 1 ? '1 item was' : `${refused.toLocaleString('en-US')} items were`;
  const most = counts.size === 1 ? 'Why' : 'Most often';

  // A reason may be a sentence of its own already ("… turned off in Settings.").
  return `${items} left out. ${most}: ${reason.replace(/\.$/, '')}.`;
}

/**
 * Why `startCleanup` said no, under the toast title "The cleanup did not start" (or `NOT_CLEANED` when the
 * user declined). Nothing is retried: the next attempt is the user's next click.
 */
function notStarted(error: unknown): string {
  if (isApiError(error)) {
    switch (error.code) {
      case 'cancelled':
        return NOT_CLEANED;
      case 'notFound':
        return 'The plan was used or it expired. Click again to make a new one.';
      case 'invalid':
        return capitalized(error.message);
    }
  }

  return capitalized(describeError(error));
}

function capitalized(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

export const cleanup = new CleanupStore();
