import { api as defaultApi } from '../api';
import { describeError, isApiError, type Api } from '../api/client';
import type {
  AdvisorAnswerDto,
  AdvisorLabelsDto,
  AdvisorStatusDto,
  ChoiceDto,
  EntryMatchDto,
  FolderMatchDto,
  GpuProblemChoice,
  GpuProblemDto,
  NodeId,
  ScanAnswerDto,
  ScanId,
  UnloadAfter,
} from '../api/types';
import { settings as defaultSettings, type SettingsStore } from './settings.svelte';

/**
 * The local model as the interface sees it: its status (pushed by the backend), the labels of the scan, the
 * questions asked (about one folder, or free ones about the scan), and the commands. It keeps no path and no URL: the backend owns the download and shows the one
 * native confirmation; this store only asks for it and reports what came back.
 */

/** The settings the advisor reads and writes. */
export type AdvisorSettings = Pick<
  SettingsStore,
  'advisorEnabled' | 'advisorUnloadAfter' | 'advisorLogQuestions' | 'advisorUseGpu' | 'units' | 'set'
>;

interface ExchangeBase {
  id: number;
  scan: ScanId;
  /** Why there is no answer: plain text. */
  error?: string;
}

/** "Can I delete this folder?" asked from a row of an answer. */
export interface FolderExchange extends ExchangeBase {
  kind: 'folder';
  node: NodeId;
  /** The path of the folder asked about. */
  path: string;
  /** Undefined while the advisor is working on it. */
  answer?: AdvisorAnswerDto;
}

/** A free question about the scan, as the user typed it. */
export interface ScanExchange extends ExchangeBase {
  kind: 'scan';
  question: string;
  /**
   * The choice of an earlier answer that asked back (`answer.clarify`) that this exchange takes: the same question is
   * asked again with the choice's `id`, and the choice's label is what the user is shown to have said.
   */
  pick?: Pick<ChoiceDto, 'id' | 'label'>;
  /** Undefined while the advisor is working on it. */
  answer?: ScanAnswerDto;
}

/** One question and its answer, in the order they were asked. */
export type Exchange = FolderExchange | ScanExchange;

/** The most results the folder picker asks for. */
export const FIND_LIMIT = 8;
/** The most questions kept on screen. */
const MAX_EXCHANGES = 30;

export class AdvisorStore {
  /** Undefined until the first answer of the backend arrives. */
  status = $state<AdvisorStatusDto | undefined>(undefined);
  /** The labels of the scan last asked for; use `labelsOf` to be sure they belong to the scan on screen. */
  labels = $state<AdvisorLabelsDto | undefined>(undefined);
  /**
   * Counts every change that makes the suggestions different (new labels, the model switched on or off): the
   * Overview and the Cleanup view fetch them again when it moves.
   */
  suggestionsVersion = $state(0);
  exchanges = $state<Exchange[]>([]);
  /** Plain text about the last download, load or unload that was refused or failed. */
  failure = $state<string | undefined>(undefined);
  /** Plain text about the last labelling that was refused or failed. */
  labelFailure = $state<string | undefined>(undefined);
  /** A labelling just ended and its labels are being fetched. */
  labelsPending = $state(false);
  /** The latest labels request is still on its way. */
  labelsLoading = $state(false);
  /** The download dialog of the backend is open (the request has not answered yet). */
  asking = $state(false);
  /** A load or an unload is on its way. */
  switching = $state(false);
  /** The graphics card problem the user closed the dialog on without answering; it is not shown again this session. */
  #dismissedGpuProblem = $state<number | undefined>(undefined);
  /** An answer to a graphics card problem is on its way. */
  #answering = false;

  readonly #api: Api;
  readonly #settings: AdvisorSettings;
  #nextExchange = 1;
  /** The id of the latest labels request: the answer of an older one no longer matters. */
  #labelsRequest = 0;
  /** Counts the statuses pushed by the backend: a command's answer older than a push is not applied. */
  #pushes = 0;

  constructor(api: Api = defaultApi, settings: AdvisorSettings = defaultSettings) {
    this.#api = api;
    this.#settings = settings;
  }

  get loaded(): boolean {
    return this.status?.state === 'loaded';
  }

  /** The graphics card problem to ask the user about: waiting for an answer, and not closed unanswered this session. */
  get gpuProblem(): GpuProblemDto | undefined {
    const problem = this.status?.gpuProblem;

    return problem?.id === this.#dismissedGpuProblem ? undefined : problem;
  }

  /** The last graphics card problem in the log, answered or not. */
  get lastGpuProblem(): GpuProblemDto | undefined {
    return this.status?.lastGpuProblem;
  }

  /** Some question is on its way, of either kind. */
  get waiting(): boolean {
    return this.exchanges.some((exchange) => !exchange.answer && !exchange.error);
  }

  /**
   * A free question is on its way. The model answers one at a time, so no other free question is sent meanwhile, about
   * any scan. A question about one folder uses no model: it neither waits for this nor holds it up.
   */
  get scanWaiting(): boolean {
    return this.exchanges.some((exchange) => exchange.kind === 'scan' && !exchange.answer && !exchange.error);
  }

  /** The model is switched on in the settings. */
  get enabled(): boolean {
    return this.#settings.advisorEnabled;
  }

  /** The labels, when they are those of this scan. */
  labelsOf(scan: ScanId): AdvisorLabelsDto | undefined {
    return this.labels?.scan === scan ? this.labels : undefined;
  }

  /** The questions asked about this scan. */
  exchangesOf(scan: ScanId): Exchange[] {
    return this.exchanges.filter((exchange) => exchange.scan === scan);
  }

  /**
   * Takes the questions and answers of this scan off the screen (those of other scans stay). Nothing is stored, so
   * there is nothing else to undo. Not while a question is on its way: its answer would have nowhere to go.
   */
  clearExchanges(scan: ScanId): void {
    if (this.waiting) {
      return;
    }

    this.exchanges = this.exchanges.filter((exchange) => exchange.scan !== scan);
  }

  /** Subscribes to the backend's status channel; the backend pushes the current status at once. */
  async watch(): Promise<void> {
    try {
      await this.#api.watchAdvisor((status) => {
        this.#pushes++;
        this.#apply(status);
      });
    } catch {
      await this.refresh();
    }
  }

  /** Asks for the status once, for when the channel is not there yet. */
  async refresh(): Promise<void> {
    const since = this.#pushes;

    try {
      this.#applyAnswer(await this.#api.advisorStatus(), since);
    } catch {
      // The status stays as it was; the next push replaces it.
    }
  }

  /**
   * Sends the options of the settings to the backend. Called at start and whenever one changes. A refusal is in
   * `failure`.
   */
  async configure(): Promise<void> {
    const since = this.#pushes;

    this.failure = undefined;

    try {
      const status = await this.#api.configureAdvisor({
        enabled: this.#settings.advisorEnabled,
        unloadAfter: this.#settings.advisorUnloadAfter,
        logQuestions: this.#settings.advisorLogQuestions,
        useGpu: this.#settings.advisorUseGpu,
      });

      this.failure = undefined;
      this.#applyAnswer(status, since);
    } catch (error) {
      this.failure = describeError(error);
    }
  }

  async setEnabled(enabled: boolean): Promise<void> {
    this.#settings.set('advisorEnabled', enabled);
    await this.configure();
  }

  async setUnloadAfter(unloadAfter: UnloadAfter): Promise<void> {
    this.#settings.set('advisorUnloadAfter', unloadAfter);
    await this.configure();
  }

  async setLogQuestions(logQuestions: boolean): Promise<void> {
    this.#settings.set('advisorLogQuestions', logQuestions);
    await this.configure();
  }

  async setUseGpu(useGpu: boolean): Promise<void> {
    this.#settings.set('advisorUseGpu', useGpu);
    await this.configure();
  }

  /** Deletes the local question log. Whether it worked is the answer; a refusal is in `failure`. */
  async clearQuestionLog(): Promise<boolean> {
    this.failure = undefined;

    try {
      await this.#api.clearQuestionLog();

      return true;
    } catch (error) {
      this.failure = describeError(error);

      return false;
    }
  }

  /**
   * Answers the graphics card problem on screen. `useCpu` also saves "Use the graphics card" off and configures, so
   * the card stays off from now on; `tryAgain` only answers (the card is tried at the next load). A refusal is in
   * `failure`, and the dialog does not come back for that problem.
   */
  async answerGpuProblem(choice: GpuProblemChoice): Promise<void> {
    const problem = this.gpuProblem;

    if (!problem || this.#answering) {
      return;
    }

    const since = this.#pushes;

    this.#answering = true;
    this.failure = undefined;

    try {
      this.#applyAnswer(await this.#api.answerGpuProblem(problem.id, choice), since);
    } catch (error) {
      this.#dismissedGpuProblem = problem.id;
      this.failure = describeError(error);
      await this.refresh();

      return;
    } finally {
      this.#answering = false;
    }

    if (choice === 'useCpu') {
      await this.setUseGpu(false);
    }
  }

  /** The user closed the dialog without answering: the problem stays unanswered and is not shown again this session. */
  dismissGpuProblem(): void {
    const problem = this.status?.gpuProblem;

    if (problem) {
      this.#dismissedGpuProblem = problem.id;
    }
  }

  /** Deletes the graphics card problem log. Whether it worked is the answer; a refusal is in `failure`. */
  async clearGpuLog(): Promise<boolean> {
    this.failure = undefined;

    try {
      await this.#api.clearGpuLog();
    } catch (error) {
      this.failure = describeError(error);

      return false;
    }

    // The backend does not have to push for this: ask, so the last problem goes from the screen.
    await this.refresh();

    return true;
  }

  /**
   * Asks the backend to download the model (or continue a cancelled download). The backend asks the user in its
   * own dialog; a "no" is quiet. Progress arrives through the status.
   */
  async download(): Promise<void> {
    if (this.asking || this.status?.state !== 'notDownloaded') {
      return;
    }

    this.asking = true;
    this.failure = undefined;

    try {
      await this.#api.startModelDownload();
    } catch (error) {
      // The user said no: nothing happened, and nothing is said about it.
      if (!isApiError(error) || error.code !== 'cancelled') {
        this.failure = describeError(error);
      }
    } finally {
      this.asking = false;
    }
  }

  async cancelDownload(): Promise<void> {
    this.failure = undefined;

    try {
      await this.#api.cancelModelDownload();
    } catch (error) {
      this.failure = describeError(error);
    }
  }

  async load(): Promise<void> {
    await this.#switch(() => this.#api.loadModel());
  }

  async unload(): Promise<void> {
    await this.#switch(() => this.#api.unloadModel());
  }

  /** Has the model label the folders no rule covers. Progress arrives through the status. */
  async label(scan: ScanId): Promise<void> {
    this.labelFailure = undefined;

    try {
      await this.#api.labelFolders(scan);
    } catch (error) {
      this.labelFailure = describeError(error);
    }
  }

  async cancelLabelling(): Promise<void> {
    try {
      await this.#api.cancelLabelling();
    } catch (error) {
      this.labelFailure = describeError(error);
    }
  }

  /**
   * Fetches the labels of a scan. Only the latest request stores its result (the labels, or their absence when it
   * failed) and settles the pending state; the answer of an older one is dropped. Returns whether it stored.
   */
  async loadLabels(scan: ScanId): Promise<boolean> {
    const request = ++this.#labelsRequest;

    this.labelsLoading = true;

    let labels: AdvisorLabelsDto | undefined;

    try {
      labels = await this.#api.getAdvisorLabels(scan);
    } catch {
      labels = undefined;
    }

    if (request !== this.#labelsRequest) {
      return false;
    }

    if (labels) {
      this.labels = labels;
    } else if (this.labels?.scan === scan) {
      this.labels = undefined;
    }

    this.labelsLoading = false;

    // The labels a finished labelling left are here: the suggestions change with them.
    if (this.labelsPending) {
      this.labelsPending = false;
      this.suggestionsVersion++;
    }

    return true;
  }

  /**
   * The user's rules changed: the suggestions are different (the Overview and the Cleanup view fetch them again), and
   * so is what the labels say was `decided`, so the labels of the scan on screen are read again.
   */
  rulesChanged(): void {
    this.suggestionsVersion++;

    if (this.labels) {
      void this.loadLabels(this.labels.scan);
    }
  }

  /**
   * Asks about one folder. The question shows at once; the answer, or the reason there is none, joins it. Platter
   * answers from its rules alone, so this is never held back by a free question, labelling or the model's state. The
   * size in the answer is written in the units of the settings.
   */
  async ask(scan: ScanId, node: NodeId, path: string): Promise<void> {
    await this.#ask({ kind: 'folder', id: this.#nextExchange++, scan, node, path });
  }

  /**
   * Asks a free question about the scan. It shows at once; the answer (at once when the words settle it, else about
   * 8 s), or the reason there is none, joins it. The sizes come back written in the units of the settings. A question
   * that was asked back is asked again with `pick`, the choice the user took: a new exchange of the same question,
   * that only sends the choice's id (never a path).
   */
  async askScan(scan: ScanId, question: string, pick?: ChoiceDto): Promise<void> {
    const text = question.trim();

    if (text === '' || this.scanWaiting) {
      return;
    }

    await this.#ask({
      kind: 'scan',
      id: this.#nextExchange++,
      scan,
      question: text,
      ...(pick === undefined ? {} : { pick: { id: pick.id, label: pick.label } }),
    });
  }

  /** Asks the same question again after an error, unless it is a free question and another one is on its way. */
  async retry(id: number): Promise<void> {
    const failed = this.exchanges.find((exchange) => exchange.id === id);

    if (!failed?.error || (failed.kind === 'scan' && this.scanWaiting)) {
      return;
    }

    this.exchanges = this.exchanges.map((exchange) =>
      exchange.id === id ? { ...exchange, error: undefined } : exchange,
    );

    await this.#resolve(failed);
  }

  async #ask(exchange: Exchange): Promise<void> {
    this.exchanges = [...this.exchanges, exchange].slice(-MAX_EXCHANGES);

    await this.#resolve(exchange);
  }

  async #resolve(exchange: Exchange): Promise<void> {
    let result: { answer: AdvisorAnswerDto | ScanAnswerDto } | { error: string };

    try {
      result = { answer: await this.#send(exchange) };
    } catch (error) {
      result = { error: describeError(error) };
    }

    this.exchanges = this.exchanges.map((entry) =>
      entry.id === exchange.id ? ({ ...entry, ...result } as Exchange) : entry,
    );
  }

  /** The command of an exchange: a question about a folder, or a free one (with the choice it takes, if any). */
  #send(exchange: Exchange): Promise<AdvisorAnswerDto | ScanAnswerDto> {
    const units = this.#settings.units;

    if (exchange.kind === 'folder') {
      return this.#api.askAdvisor(exchange.scan, exchange.node, units);
    }

    const { scan, question, pick } = exchange;

    return pick === undefined
      ? this.#api.askScan(scan, question, units)
      : this.#api.askScan(scan, question, units, pick.id);
  }

  /** Folders of a scan whose name or path matches the text. Rejects as the API does. */
  async find(scan: ScanId, query: string): Promise<FolderMatchDto[]> {
    return this.#api.findFolders(scan, query, FIND_LIMIT);
  }

  /** Folders and files of a scan for the word being typed in the question box. Rejects as the API does. */
  async findEntries(scan: ScanId, query: string): Promise<EntryMatchDto[]> {
    return this.#api.findEntries(scan, query, FIND_LIMIT);
  }

  async #switch(run: () => Promise<AdvisorStatusDto>): Promise<void> {
    if (this.switching) {
      return;
    }

    const since = this.#pushes;

    this.switching = true;
    this.failure = undefined;

    try {
      this.#applyAnswer(await run(), since);
    } catch (error) {
      this.failure = describeError(error);
    } finally {
      this.switching = false;
    }
  }

  /** The answer of a command: dropped when the backend pushed a status after the command began, as that is newer. */
  #applyAnswer(next: AdvisorStatusDto, pushesAtStart: number): void {
    if (this.#pushes === pushesAtStart) {
      this.#apply(next);
    }
  }

  /** Takes a status, whichever way it came: what changes from the one before is derived here, once. */
  #apply(next: AdvisorStatusDto): void {
    const previous = this.status;

    this.status = next;

    if (!previous) {
      return;
    }

    if (previous.options.enabled !== next.options.enabled) {
      this.suggestionsVersion++;
    }

    // A labelling that ended: its labels are fetched, and the suggestions change with them.
    if (previous.labelling && !next.labelling) {
      this.labelsPending = true;

      void this.loadLabels(previous.labelling.scan);
    }

    // A finished download switches the model on; it was off because there was nothing to use.
    const downloaded = next.state === 'onDisk' || next.state === 'loading' || next.state === 'loaded';

    if (previous.state === 'downloading' && downloaded) {
      if (!this.#settings.advisorEnabled) {
        void this.setEnabled(true);
      } else if (!next.options.enabled) {
        void this.configure();
      }
    }
  }
}

export const advisor = new AdvisorStore();
