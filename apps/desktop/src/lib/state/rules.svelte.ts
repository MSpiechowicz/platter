import { api as defaultApi } from '../api';
import { describeError, type Api } from '../api/client';
import type { NodeId, ScanId, UserRuleDto, UserRulesDto, UserVerdict } from '../api/types';
import { advisor } from './advisor.svelte';

/**
 * The user's own rules ("always keep this folder", "this folder is safe to clean") as the interface sees them: the
 * list the backend holds, and the four commands that change it. The web view names a folder only by its node id in a
 * finished scan; the backend works out the path, and keeps the rules in its own file. Every command answers with the
 * whole list, and a successful change says so through `onChange`, as the suggestions and the labels' `decided` are
 * different from then on.
 *
 * A command that is refused or fails does not throw: it returns the plain text of the reason, to be shown next to
 * the control that was pressed (`undefined` means it worked).
 */
export class RulesStore {
  /** The rules, in the order they were made. Empty while the rules file is broken. */
  rules = $state<UserRuleDto[]>([]);
  /** Why the rules file cannot be used. While it is set, nothing can be added or removed. */
  problem = $state<string | undefined>(undefined);
  /** The list has been read at least once (an empty list is then really empty). */
  loaded = $state(false);
  /** Why the list could not be read: plain text. */
  loadFailure = $state<string | undefined>(undefined);

  readonly #api: Api;
  readonly #onChange: () => void;
  /** The id of the latest command; the answer of an older one never replaces the list of a newer one. */
  #requests = 0;
  #applied = 0;

  constructor(api: Api = defaultApi, onChange: () => void = () => advisor.rulesChanged()) {
    this.#api = api;
    this.#onChange = onChange;
  }

  /** The file is broken: the list is not shown and nothing can be added. */
  get broken(): boolean {
    return this.problem !== undefined;
  }

  /** Reads the list. The rules file is read again by the backend every time, so a hand edit shows up. */
  async load(): Promise<void> {
    const request = ++this.#requests;

    try {
      this.#apply(await this.#api.listUserRules(), request);
      this.loadFailure = undefined;
    } catch (error) {
      this.loadFailure = describeError(error);
    }
  }

  /** Says `keep` or `safe` about a folder of a finished scan. Returns the reason when it was refused. */
  async add(scan: ScanId, node: NodeId, verdict: UserVerdict): Promise<string | undefined> {
    return this.#change(() => this.#api.addUserRule(scan, node, verdict));
  }

  async remove(id: number): Promise<string | undefined> {
    return this.#change(() => this.#api.removeUserRule(id));
  }

  /** Moves a broken rules file aside and starts with no rules. */
  async reset(): Promise<string | undefined> {
    return this.#change(() => this.#api.resetUserRules());
  }

  async #change(run: () => Promise<UserRulesDto>): Promise<string | undefined> {
    const request = ++this.#requests;

    try {
      this.#apply(await run(), request);
    } catch (error) {
      return describeError(error);
    }

    this.#onChange();

    return undefined;
  }

  #apply(list: UserRulesDto, request: number): void {
    this.loaded = true;

    if (request < this.#applied) {
      return;
    }

    this.#applied = request;
    this.rules = list.rules;
    this.problem = list.problem;
  }
}

export const rules = new RulesStore();
