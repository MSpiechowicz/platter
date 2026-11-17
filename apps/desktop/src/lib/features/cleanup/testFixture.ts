import { waitFor } from '@testing-library/svelte';
import { expect, vi } from 'vitest';
import type { Api } from '../../api/client';
import { createMockApi } from '../../api/mock';
import type {
  CleanupEvent,
  CleanupPlanDto,
  PlanGroupDto,
  SuggestionDto,
  SuggestionsDto,
  Verdict,
} from '../../api/types';
import { CleanupStore } from '../../state/cleanup.svelte';
import { ScanStore } from '../../state/scan.svelte';

/** Helpers for the tests of the Cleanup view's parts: a small, known set of suggestions and a scripted backend. */

export const GB = 1_000_000_000;

export function suggestion(id: string, verdict: Verdict, gb: number, nodes: number[] = [11]): SuggestionDto {
  return {
    id,
    title: `Title of ${id}`,
    path: `~/${id}`,
    verdict,
    reason: `Reason of ${id}.`,
    allocated: gb * GB,
    items: nodes.length,
    nodes,
  };
}

export const DATA: SuggestionsDto = {
  suggestions: [
    suggestion('snapshots', 'review', 268),
    suggestion('steam', 'keep', 142),
    suggestion('target', 'safe', 88),
    suggestion('trash', 'safe', 17),
    suggestion('thin', 'safe', 0.4),
  ],
  totals: { safe: 105.4 * GB, review: 268 * GB, keep: 142 * GB },
};

export const group = (id: string, verdict: Verdict, gb: number): PlanGroupDto => ({
  ruleId: id,
  title: `Title of ${id}`,
  verdict,
  action: 'delete',
  items: 1,
  allocated: gb * GB,
  needsElevation: false,
  samplePaths: [`/home/demo/${id}`],
});

/** The group a plan of hand-marked items has: one rule id, `marked`, whatever was marked. */
export const markedGroup = (gb: number, items = 2): PlanGroupDto => ({
  ruleId: 'marked',
  title: 'Marked in the Explorer',
  verdict: 'review',
  action: 'trash',
  items,
  allocated: gb * GB,
  needsElevation: false,
  samplePaths: ['/home/demo/Downloads/big.iso'],
});

export const planOf = (groups: PlanGroupDto[]): CleanupPlanDto => ({
  planId: 5,
  scanId: 1,
  options: { keepPackageVersions: 2, askForAdmin: true },
  groups,
  refused: [],
  refusedMore: 0,
  notFound: [],
  allocated: groups.reduce((sum, item) => sum + item.allocated, 0),
  needsConfirmation: true,
});

export interface Fixture {
  scan: ScanStore;
  store: CleanupStore;
  scanId: number;
  /** Sends an event to the run the store started. */
  send: (event: CleanupEvent) => void;
}

/**
 * A store over a finished mock scan, loaded with `DATA`, whose backend plans what it is asked and holds the run.
 * `change` may replace parts of that backend.
 */
export async function fixture(change: (api: Api) => Api = (api) => api): Promise<Fixture> {
  let send: (event: CleanupEvent) => void = () => {};
  const mock = createMockApi({ durationMs: 0 });
  const api: Api = change({
    ...mock,
    planCleanup: vi.fn<Api['planCleanup']>(async (_scan, ids) =>
      planOf(
        ids.map((id) => group(id, DATA.suggestions.find((item) => item.id === id)?.verdict ?? 'safe', 1)),
      ),
    ),
    planMarkedCleanup: vi.fn<Api['planMarkedCleanup']>(async () => planOf([markedGroup(40)])),
    startCleanup: vi.fn<Api['startCleanup']>(async (_plan, onEvent) => {
      send = onEvent;

      return 77;
    }),
  });
  const scan = new ScanStore(api);

  await scan.start({ root: '/', oneFileSystem: true });
  await waitFor(() => expect(scan.state.status).toBe('ready'));

  if (scan.state.status !== 'ready') {
    throw new Error('expected a finished scan');
  }

  const store = new CleanupStore(api, scan);

  store.load(scan.state.scanId, DATA);

  return { scan, store, scanId: scan.state.scanId, send: (event) => send(event) };
}

export const removed = { kind: 'removed', problems: 0 } as const;

/** The events of one group that went well. */
export function cleanGroup(send: (event: CleanupEvent) => void, ruleId: string): void {
  send({ type: 'groupStarted', ruleId, targets: 1 });
  send({ type: 'targetDone', ruleId, path: `/home/demo/${ruleId}`, allocated: 1, outcome: removed });
  send({ type: 'groupFinished', ruleId });
}

export function finish(
  send: (event: CleanupEvent) => void,
  scanId: number,
  extra: Partial<Extract<CleanupEvent, { type: 'finished' }>> = {},
): void {
  send({
    type: 'finished',
    cleanupId: 77,
    outdatedScan: scanId,
    cleaned: 2,
    withProblems: 0,
    freed: 105 * GB,
    cancelled: false,
    toolsNotRun: [],
    canUndo: false,
    ...extra,
  });
}

/**
 * jsdom has no layout: the chart's box is told it is 704 x 440, and a resize observer that never fires is enough.
 * Returns what undoes it.
 */
export function stubChartBox(): () => void {
  const proto = HTMLElement.prototype as unknown as Record<string, unknown>;
  const observer = globalThis.ResizeObserver;
  const size = (value: number) => ({
    configurable: true,
    get(this: HTMLElement) {
      return this.classList.contains('flow') ? value : 0;
    },
  });

  Object.defineProperty(HTMLElement.prototype, 'clientWidth', size(704));
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', size(440));
  globalThis.ResizeObserver ??= class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  };

  return () => {
    delete proto.clientWidth;
    delete proto.clientHeight;
    globalThis.ResizeObserver = observer;
  };
}
