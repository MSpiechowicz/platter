import { describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type Api } from '../api/client';
import { createMockApi } from '../api/mock';
import type { CleanupEvent, CleanupPlanDto, PlanGroupDto, SuggestionsDto } from '../api/types';
import { CleanupStore, NOT_CLEANED, progressTitle, refusedSummary } from './cleanup.svelte';
import { ScanStore } from './scan.svelte';
import { settings } from './settings.svelte';

const ROOT = { root: '/', oneFileSystem: true };

interface Setup {
  api: Api;
  scan: ScanStore;
  store: CleanupStore;
  /** The toasts the store showed, as `[title, description, kind]`. */
  toast: ReturnType<typeof vi.fn>;
  data: SuggestionsDto;
  scanId: number;
}

/** A finished mock scan, its suggestions loaded into a fresh store. `wrap` lets a test change what the API does. */
async function setup(wrap: (api: Api) => Api = (api) => api): Promise<Setup> {
  const mock = createMockApi({ durationMs: 0, now: () => 1_750_000_000 });
  const api = wrap(mock);
  const scan = new ScanStore(api);

  await scan.start(ROOT);
  await vi.waitFor(() => expect(scan.state.status).toBe('ready'));

  if (scan.state.status !== 'ready') {
    throw new Error('expected a finished scan');
  }

  const scanId = scan.state.scanId;
  const data = await api.getSuggestions(scanId);
  const toast = vi.fn();
  const store = new CleanupStore(api, scan, { toast });

  store.load(scanId, data);

  return { api, scan, store, data, scanId, toast };
}

const ids = (data: SuggestionsDto, verdict: string): string[] =>
  data.suggestions.filter((item) => item.verdict === verdict).map((item) => item.id);

/** A review rule the mock plan acts on (deleting build folders), and one it moves to the trash. */
const REVIEW_RULE = 'rust-target-recent';
const TRASH_RULE = 'disk-images';

const sorted = (values: Iterable<string>): string[] => [...values].sort();

const done = (store: CleanupStore) => (): void => {
  expect(store.phase.kind).toBe('done');
};

describe('known suggestions', () => {
  it('are known once they are loaded, and not known for a new scan until its own arrive', async () => {
    const { store, scanId } = await setup();

    expect(store.suggestionsKnown).toBe(true);

    store.adopt(scanId + 1);

    expect(store.suggestionsKnown).toBe(false);
    expect(store.suggestions).toEqual([]);
  });
});

describe('selection', () => {
  it('ticks the safe rows when the suggestions arrive, and no review or kept row', async () => {
    const { store, data } = await setup();

    expect(sorted(store.selected)).toEqual(sorted(ids(data, 'safe')));
    expect(store.choice).toBe('safe');
    expect(store.selectedBytes).toBe(store.safeBytes);
    expect(store.reviewSelected).toEqual([]);
  });

  it('selects safe, all and none; all is every row that can be cleaned and never a kept row', async () => {
    const { store, data } = await setup();

    store.select('all');
    expect(sorted(store.selected)).toEqual(sorted([...ids(data, 'safe'), ...ids(data, 'review')]));
    expect(store.choice).toBe('all');
    expect(ids(data, 'keep').some((id) => store.isSelected(id))).toBe(false);

    store.select('none');
    expect(store.selected.size).toBe(0);
    expect(store.choice).toBe('none');
    expect(store.canCleanSelected).toBe(false);

    store.select('safe');
    expect(sorted(store.selected)).toEqual(sorted(ids(data, 'safe')));
    expect(store.choice).toBe('safe');
  });

  it('toggles a row, ignores a kept row, and says "custom" for any other selection', async () => {
    const { store, data } = await setup();
    const review = REVIEW_RULE;
    const kept = ids(data, 'keep')[0] as string;

    store.toggle(review);
    expect(store.isSelected(review)).toBe(true);
    expect(store.choice).toBe('custom');
    expect(store.reviewSelected.map((item) => item.id)).toEqual([review]);

    store.toggle(kept);
    expect(store.isSelected(kept)).toBe(false);

    store.toggle(review);
    expect(store.choice).toBe('safe');
  });

  it('selects again from the defaults when a new scan arrives', async () => {
    const { store, data, scanId } = await setup();

    store.select('none');
    store.load(scanId + 1, data);

    expect(store.choice).toBe('safe');
  });

  it('keeps what the user ticked when the same suggestions are loaded again', async () => {
    const { store, data, scanId } = await setup();

    store.select('none');
    store.load(scanId, data);

    expect(store.choice).toBe('none');
  });
});

describe('suggestions fetched again for the same scan', () => {
  it('keeps the rows the user ticked while they still exist, and drops the ones that are gone', async () => {
    const { store, data, scanId } = await setup();
    const gone = ids(data, 'safe')[0] as string;

    store.select('all');

    const rest = data.suggestions.filter((item) => item.id !== gone);

    store.load(scanId, { ...data, suggestions: rest });

    expect(store.selected.has(gone)).toBe(false);
    expect(sorted(store.selected)).toEqual(
      sorted(rest.filter((item) => item.verdict !== 'keep').map((item) => item.id)),
    );
  });

  it('does not tick a review row that appears, and ticks a safe one that appears', async () => {
    const { store, data, scanId } = await setup();
    const row = (id: string, verdict: 'safe' | 'review') => ({
      ...(data.suggestions[0] as SuggestionsDto['suggestions'][number]),
      id,
      verdict,
    });

    store.select('none');
    store.load(scanId, {
      ...data,
      suggestions: [...data.suggestions, row('advisor', 'review'), row('mine', 'safe')],
    });

    expect(sorted(store.selected)).toEqual(['mine']);
  });

  it('keeps a row the user unticked unticked, and ticks no review row, when the same scan is fetched again', async () => {
    const { store, data, scanId } = await setup();
    const unticked = ids(data, 'safe')[0] as string;

    store.toggle(unticked);
    expect(store.selected.has(unticked)).toBe(false);

    store.load(scanId, { ...data, suggestions: [...data.suggestions] });

    expect(store.selected.has(unticked)).toBe(false);
    expect(ids(data, 'review').length).toBeGreaterThan(0);

    for (const review of ids(data, 'review')) {
      expect(store.selected.has(review)).toBe(false);
    }

    expect(sorted(store.selected)).toEqual(sorted(ids(data, 'safe').filter((id) => id !== unticked)));
  });

  it('holds the stamp that triggers a fetch while a cleanup runs, and lets it go afterwards', async () => {
    const { store } = await setup();

    expect(store.stampFor(1)).toBe(1);

    store.phase = { kind: 'planning' };
    expect(store.stampFor(2)).toBe(1);

    store.phase = { kind: 'idle' };
    expect(store.stampFor(2)).toBe(2);
  });
});

describe('the units the suggestions are fetched in', () => {
  it('holds the units while a cleanup runs, and lets them go afterwards', async () => {
    const { store } = await setup();

    expect(store.unitsFor('GiB')).toBe('GiB');

    store.phase = { kind: 'planning' };
    expect(store.unitsFor('GB')).toBe('GiB');

    store.phase = { kind: 'idle' };
    expect(store.unitsFor('GB')).toBe('GB');
  });

  it('asks for the suggestions in the units of the settings when it has to fetch them', async () => {
    const { api, scan, scanId } = await setup();
    const getSuggestions = vi.spyOn(api, 'getSuggestions');
    const store = new CleanupStore(api, scan);

    settings.set('units', 'GiB');
    await store.ensureSuggestions(scanId);
    settings.set('units', 'GB');

    expect(getSuggestions).toHaveBeenCalledExactlyOnceWith(scanId, 'GiB');
  });
});

describe('planning', () => {
  it('plans only the ids of the selection, and runs a plan of safe items without asking', async () => {
    const { api, store, data, scanId } = await setup();
    const planCleanup = vi.spyOn(api, 'planCleanup');

    await store.cleanSelected();

    expect(planCleanup).toHaveBeenCalledWith(scanId, ids(data, 'safe'), {
      keepPackageVersions: 2,
      askForAdmin: true,
    });
    await vi.waitFor(done(store));
    expect(store.reviewing).toBeUndefined();
  });

  it('Clean safe selects exactly the safe rows and plans those', async () => {
    const { api, store, data, scanId } = await setup();
    const planCleanup = vi.spyOn(api, 'planCleanup');

    store.select('all');
    await store.cleanSafe();

    expect(planCleanup).toHaveBeenCalledWith(scanId, ids(data, 'safe'), {
      keepPackageVersions: 2,
      askForAdmin: true,
    });
    expect(store.choice).not.toBe('all');
    await vi.waitFor(done(store));
  });

  it('opens the review step for a plan with a review group, and runs nothing until it is confirmed', async () => {
    const { api, store, data } = await setup();
    const startCleanup = vi.spyOn(api, 'startCleanup');
    const review = REVIEW_RULE;

    expect(ids(data, 'review')).toContain(review);
    store.select('safe');
    store.toggle(review);
    await store.cleanSelected();

    const plan = store.reviewing;

    expect(plan?.groups.some((group) => group.verdict === 'review' && group.ruleId === review)).toBe(true);
    expect(store.busy).toBe(true);
    expect(startCleanup).not.toHaveBeenCalled();

    await store.confirmReview();
    await vi.waitFor(done(store));

    expect(startCleanup).toHaveBeenCalledOnce();
    expect(startCleanup.mock.calls[0]?.[0]).toBe(plan?.planId);
  });

  it('drops the plan when the review step is cancelled, says "Not cleaned." and starts nothing', async () => {
    const { api, store, data } = await setup();
    const startCleanup = vi.spyOn(api, 'startCleanup');

    expect(ids(data, 'review')).toContain(REVIEW_RULE);
    store.toggle(REVIEW_RULE);
    await store.cleanSelected();
    store.dismissReview();

    expect(store.phase.kind).toBe('idle');
    expect(store.notice).toBe(NOT_CLEANED);
    expect(startCleanup).not.toHaveBeenCalled();
    expect(store.canCleanSelected).toBe(true);
  });

  it('keeps the refusals and the rules that matched nothing, so they can be shown', async () => {
    const { store, data } = await setup();

    store.select('all');
    await store.cleanSelected();

    // Some rules of the mock have matches the plan refuses (the scan root, for example); the plan lists them.
    expect(store.plan?.refused.length).toBeGreaterThan(0);
    expect(store.plan?.groups.length).toBeGreaterThan(0);
    expect(ids(data, 'review').length).toBeGreaterThan(0);
    expect(store.reviewing).toEqual(store.plan);
  });

  it('says in a toast why a plan could not be made and does not try again', async () => {
    const { api, store, toast } = await setup((api) => ({
      ...api,
      planCleanup: vi.fn().mockRejectedValue(new ApiRequestError('invalid', 'the disk changed')),
    }));

    await store.cleanSelected();

    expect(store.phase.kind).toBe('idle');
    expect(store.notice).toBeUndefined();
    expect(toast).toHaveBeenCalledExactlyOnceWith('Could not plan the cleanup', 'the disk changed', 'error');
    expect(api.planCleanup).toHaveBeenCalledOnce();
  });

  it('says in a toast when nothing of the selection can be cleaned, with the most common reason', async () => {
    const { store, toast } = await setup((api) => ({
      ...api,
      planCleanup: async (scanId) => ({
        planId: 1,
        scanId,
        options: { keepPackageVersions: 2, askForAdmin: true },
        groups: [],
        refused: [
          { ruleId: 'x', path: '/x', reason: 'it is, or lies below, another mounted filesystem' },
          { ruleId: 'x', path: '/y', reason: 'it is, or lies below, another mounted filesystem' },
          { ruleId: 'x', path: '/z', reason: 'kept on purpose' },
          {
            ruleId: 'x',
            path: '/w',
            reason: 'Needs administrator rights, which are turned off in Settings.',
          },
        ],
        refusedMore: 6555,
        notFound: [],
        allocated: 0,
        needsConfirmation: false,
      }),
    }));

    await store.cleanSelected();

    expect(store.phase.kind).toBe('idle');
    expect(store.notice).toBeUndefined();
    expect(toast).toHaveBeenCalledExactlyOnceWith(
      'Nothing could be cleaned',
      '6,559 items were left out. Most often: it is, or lies below, another mounted filesystem.',
      'error',
    );
    expect(store.plan?.refused).toHaveLength(4);
  });

  it('does not double the full stop of a reason that is a sentence already', () => {
    const reason = 'Needs administrator rights, which are turned off in Settings.';
    const plan: CleanupPlanDto = {
      planId: 1,
      scanId: 1,
      options: { keepPackageVersions: 2, askForAdmin: false },
      groups: [],
      refused: [{ ruleId: 'x', path: '/x', reason }],
      refusedMore: 0,
      notFound: [],
      allocated: 0,
      needsConfirmation: false,
    };

    expect(refusedSummary(plan)).toBe(`1 item was left out. Why: ${reason}`);
  });

  it('drops a plan that arrives after the scan was replaced', async () => {
    let release: () => void = () => {};
    const { store, data, scanId } = await setup((api) => ({
      ...api,
      planCleanup: async (scan, suggestions) => {
        await new Promise<void>((resolve) => (release = resolve));

        return api.planCleanup(scan, suggestions);
      },
    }));

    const planning = store.cleanSelected();

    expect(store.phase.kind).toBe('planning');
    store.load(scanId + 1, data);
    release();
    await planning;

    expect(store.phase.kind).toBe('idle');
    expect(store.reviewing).toBeUndefined();
  });

  it('does nothing for a second click while one flow runs', async () => {
    const { api, store } = await setup();
    const planCleanup = vi.spyOn(api, 'planCleanup');

    const first = store.cleanSelected();

    await store.cleanSelected();
    await store.cleanSafe();
    store.select('none');
    store.toggle('rust-target');

    expect(planCleanup).toHaveBeenCalledOnce();
    expect(store.selected.size).toBeGreaterThan(0);

    await first;
    await vi.waitFor(done(store));
  });
});

describe('the run', () => {
  it('shows progress, then the result: scan outdated, "Already freed" grown, and a toast with no button', async () => {
    const { api, scan, store, scanId, toast } = await setup();
    const seen: number[] = [];
    const startCleanup = api.startCleanup.bind(api);

    vi.spyOn(api, 'startCleanup').mockImplementation((plan, onEvent) =>
      startCleanup(plan, (event) => {
        onEvent(event);

        if (store.phase.kind === 'running') {
          seen.push(store.phase.progress.done);
        }
      }),
    );

    expect(store.freedTotal).toBe(0);
    expect(store.outdated).toBe(false);

    await store.cleanSelected();
    await vi.waitFor(done(store));

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    const result = store.phase.result;

    expect(seen.length).toBeGreaterThan(0);
    expect(Math.max(...seen)).toBeGreaterThan(0);
    expect(result.kind).toBe('finished');
    expect(result.cleaned).toBeGreaterThan(0);
    expect(result.freed).toBeGreaterThan(0);
    expect(store.freedTotal).toBe(result.freed);
    expect(scan.state.status === 'ready' && scan.state.outdated && scan.state.scanId === scanId).toBe(true);
    expect(store.outdated).toBe(true);
    // Rescan is the outdated banner's button: neither the footer nor the toast offers one.
    expect(store.actions).toEqual([]);
    expect(toast).toHaveBeenCalledOnce();
    expect(toast.mock.calls[0]).toHaveLength(3);
    expect(result.endedAt).toBeGreaterThan(0);
    expect(store.canCleanSafe).toBe(false);
    expect(store.canCleanSelected).toBe(false);
  });

  it('adds up what several sessions of cleaning freed', async () => {
    const { store } = await setup();

    store.freedTotal = 500;
    await store.cleanSelected();
    await vi.waitFor(done(store));

    expect(store.freedTotal).toBeGreaterThan(500);
  });

  it('marks the rows that were cleaned and takes them out of the selection', async () => {
    const { store, data } = await setup();

    await store.cleanSelected();
    await vi.waitFor(done(store));

    expect(ids(data, 'safe').some((id) => store.cleaned.has(id))).toBe(true);
    expect(store.selected.size).toBe(0);
  });

  it('refuses a new plan once the scan is outdated: the buttons are off and a click does nothing', async () => {
    const { api, store } = await setup();

    await store.cleanSelected();
    await vi.waitFor(done(store));

    const planCleanup = vi.spyOn(api, 'planCleanup');

    await store.cleanSafe();
    await store.cleanSelected();

    expect(planCleanup).not.toHaveBeenCalled();
  });

  it('offers undo for a cleanup that moved things to the trash, and puts them back once', async () => {
    const { store } = await setup();
    const trashed = TRASH_RULE;

    store.select('none');
    store.toggle(trashed);
    await store.cleanSelected();
    await store.confirmReview();
    await vi.waitFor(done(store));

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    expect(store.phase.result.trashed).toBeGreaterThan(0);
    expect(store.phase.result.undoable).toBe(true);
    expect(store.actions).toEqual(['undo']);
    // Moving to the trash frees nothing yet.
    expect(store.freedTotal).toBe(0);

    await store.undo();

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    expect(store.phase.result.undo?.restored).toBe(store.phase.result.trashed);
    expect(store.phase.result.undo?.failures).toEqual([]);
    expect(store.phase.result.undoable).toBe(false);
    expect(store.actions).toEqual([]);
  });

  it('shows each failure of an undo and keeps undo available for another click', async () => {
    const { store } = await setup((api) => ({
      ...api,
      undoCleanup: async () => [
        { path: '/home/demo/a', restored: true },
        { path: '/home/demo/b', restored: false, error: 'something is there already' },
      ],
    }));

    store.select('none');
    store.toggle(TRASH_RULE);
    await store.cleanSelected();
    await store.confirmReview();
    await vi.waitFor(done(store));
    await store.undo();

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    expect(store.phase.result.undo).toEqual({
      restored: 1,
      failures: [{ path: '/home/demo/b', error: 'something is there already' }],
    });
    expect(store.phase.result.undoable).toBe(true);
  });

  it('an undo that cannot be done says so; "not found" is final, other errors can be tried again by hand', async () => {
    for (const [code, undoable] of [
      ['notFound', false],
      ['io', true],
    ] as const) {
      const { store } = await setup((api) => ({
        ...api,
        undoCleanup: async () => {
          throw new ApiRequestError(code, `undo says ${code}`);
        },
      }));

      store.select('none');
      store.toggle(TRASH_RULE);
      await store.cleanSelected();
      await store.confirmReview();
      await vi.waitFor(done(store));
      await store.undo();

      if (store.phase.kind !== 'done') {
        throw new Error('expected a result');
      }

      expect(store.phase.result.undo?.error).toBe(`undo says ${code}`);
      expect(store.phase.result.undoable).toBe(undoable);
    }
  });

  it('says an undo is running from the click until the backend answers, whatever it answers', async () => {
    for (const answer of ['restored', 'declined', 'failed'] as const) {
      let reply: (entries: Awaited<ReturnType<Api['undoCleanup']>>) => void = () => {};
      let refuse: (error: Error) => void = () => {};
      const { store } = await setup((api) => ({
        ...api,
        undoCleanup: () =>
          new Promise((resolve, reject) => {
            reply = resolve;
            refuse = reject;
          }),
      }));

      store.select('none');
      store.toggle(TRASH_RULE);
      await store.cleanSelected();
      await store.confirmReview();
      await vi.waitFor(done(store));

      expect(store.undoing).toBe(false);

      const undo = store.undo();

      expect(store.undoing).toBe(true);

      if (answer === 'restored') {
        reply([{ path: '/home/demo/b', restored: true }]);
      } else {
        refuse(new ApiRequestError(answer === 'declined' ? 'cancelled' : 'io', answer));
      }

      await undo;

      expect(store.undoing).toBe(false);
    }
  });

  it('a declined Undo dialog ("cancelled") says nothing and keeps undo offered, also after an earlier partial undo', async () => {
    const undoCleanup = vi
      .fn<Api['undoCleanup']>()
      .mockResolvedValueOnce([{ path: '/home/demo/b', restored: false, error: 'something is there already' }])
      .mockRejectedValueOnce(new ApiRequestError('cancelled', 'declined'));
    const { store } = await setup((api) => ({ ...api, undoCleanup }));

    store.select('none');
    store.toggle(TRASH_RULE);
    await store.cleanSelected();
    await store.confirmReview();
    await vi.waitFor(done(store));
    await store.undo();

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    const before = store.phase.result.undo;

    await store.undo();

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    expect(undoCleanup).toHaveBeenCalledTimes(2);
    expect(store.phase.result.undo).toEqual(before);
    expect(store.phase.result.undo?.error).toBeUndefined();
    expect(store.phase.result.undoable).toBe(true);
    expect(store.actions).toEqual(['undo']);
  });

  it('does not offer undo for a cleanup that moved nothing to the trash', async () => {
    const { store } = await setup();

    await store.cleanSelected();
    await vi.waitFor(done(store));

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    expect(store.phase.result.undoable).toBe(false);

    await store.undo();
    expect(store.phase.result.undo).toBeUndefined();
  });

  it('the next scan starts without the old result', async () => {
    const { scan, store, data } = await setup();

    await store.cleanSelected();
    await vi.waitFor(done(store));
    await scan.rescan();
    await vi.waitFor(() => expect(scan.state.status).toBe('ready'));

    if (scan.state.status !== 'ready') {
      throw new Error('expected a finished scan');
    }

    expect(scan.state.outdated).toBe(false);

    store.load(scan.state.scanId, data);

    expect(store.phase.kind).toBe('idle');
    expect(store.cleaned.size).toBe(0);
    expect(store.outdated).toBe(false);
    expect(store.canCleanSafe).toBe(true);
    // What the earlier runs freed is still counted.
    expect(store.freedTotal).toBeGreaterThan(0);
  });
});

describe('when the backend says no', () => {
  const rejecting = (code: 'cancelled' | 'invalid' | 'notFound' | 'io', message: string) => (api: Api) => ({
    ...api,
    startCleanup: vi.fn().mockRejectedValue(new ApiRequestError(code, message)),
  });

  it('"cancelled" (the native dialog was declined) returns to idle with "Not cleaned." and no retry', async () => {
    const { api, store, scan } = await setup(rejecting('cancelled', 'declined'));

    await store.cleanSelected();

    expect(store.phase.kind).toBe('idle');
    expect(store.notice).toBe(NOT_CLEANED);
    expect(api.startCleanup).toHaveBeenCalledOnce();
    expect(scan.state.status === 'ready' && scan.state.outdated).toBe(false);
    expect(store.canCleanSelected).toBe(true);
  });

  it('"invalid" says why in a toast, and a new plan is made only on the next click', async () => {
    const { api, store, toast } = await setup(rejecting('invalid', 'a cleanup is already running'));
    const planCleanup = vi.spyOn(api, 'planCleanup');

    await store.cleanSelected();
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(toast).toHaveBeenCalledExactlyOnceWith(
      'The cleanup did not start',
      'A cleanup is already running',
      'error',
    );
    expect(store.phase.kind).toBe('idle');
    expect(api.startCleanup).toHaveBeenCalledOnce();
    expect(planCleanup).toHaveBeenCalledOnce();

    await store.cleanSelected();

    expect(planCleanup).toHaveBeenCalledTimes(2);
  });

  it('"notFound" says in a toast the plan is gone and waits for a click for a new one', async () => {
    const { store, toast } = await setup(rejecting('notFound', 'plan 1 does not exist'));

    await store.cleanSelected();

    expect(toast).toHaveBeenCalledExactlyOnceWith(
      'The cleanup did not start',
      'The plan was used or it expired. Click again to make a new one.',
      'error',
    );
    expect(store.phase.kind).toBe('idle');
    expect(store.canCleanSelected).toBe(true);
  });

  it('any other error is shown in a toast, not in the view', async () => {
    const { store, toast } = await setup(rejecting('io', 'the disk went away'));

    await store.cleanSelected();

    expect(store.notice).toBeUndefined();
    expect(toast).toHaveBeenCalledExactlyOnceWith('The cleanup did not start', 'The disk went away', 'error');
  });
});

describe('events', () => {
  /** A backend that holds its events until the test sends them. */
  async function held() {
    let send: (event: CleanupEvent) => void = () => {};
    let resolve: (id: number) => void = () => {};
    const cancelCleanup = vi.fn().mockResolvedValue(undefined);

    const context = await setup((api) => ({
      ...api,
      cancelCleanup,
      startCleanup: (_plan, onEvent) => {
        send = onEvent;

        return new Promise<number>((done) => (resolve = done));
      },
    }));

    // The call stays open until `resolve`, as the backend's does while it works.
    void context.store.cleanSelected();
    await vi.waitFor(() => expect(context.store.phase.kind).toBe('running'));

    return {
      ...context,
      send: (event: CleanupEvent) => send(event),
      resolve: (id: number) => resolve(id),
      cancelCleanup,
    };
  }

  it('accepts events before the id is known, and a last event before the call returns', async () => {
    const { store, send, resolve, scanId, scan } = await held();

    const first = store.plan?.groups[0];

    send({ type: 'groupStarted', ruleId: first?.ruleId ?? '', targets: 1 });
    expect(store.phase.kind === 'running' && store.phase.progress.group).toBe(first?.title);

    send({
      type: 'targetDone',
      ruleId: first?.ruleId ?? '',
      path: '/x',
      allocated: 5,
      outcome: { kind: 'removed', problems: 0 },
    });
    expect(store.phase.kind === 'running' && store.phase.progress.done).toBe(1);

    send({
      type: 'finished',
      cleanupId: 7,
      outdatedScan: scanId,
      cleaned: 1,
      withProblems: 0,
      freed: 5,
      cancelled: false,
      toolsNotRun: [],
      canUndo: false,
    });
    resolve(7);
    await Promise.resolve();

    expect(store.phase.kind).toBe('done');
    expect(store.freedTotal).toBe(5);
    expect(scan.state.status === 'ready' && scan.state.outdated).toBe(true);
  });

  it('a failed run shows its message, marks the scan outdated, and offers undo only when it can', async () => {
    const { store, send, scanId, scan } = await held();

    send({ type: 'failed', message: 'the disk is full', outdatedScan: scanId, canUndo: true, cleanupId: 3 });

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    expect(store.phase.result).toMatchObject({ kind: 'failed', message: 'the disk is full', undoable: true });
    expect(store.actions).toEqual(['undo']);
    expect(scan.state.status === 'ready' && scan.state.outdated).toBe(true);
    expect(store.freedTotal).toBe(0);
  });

  it('keeps the skipped and failed targets and the tools that did not run, and not what went well', async () => {
    const { store, send, scanId } = await held();
    const rule = store.plan?.groups[0]?.ruleId ?? '';

    send({
      type: 'targetDone',
      ruleId: rule,
      path: '/ok',
      allocated: 1,
      outcome: { kind: 'removed', problems: 0 },
    });
    send({
      type: 'targetDone',
      ruleId: rule,
      path: '/bad',
      allocated: 1,
      outcome: { kind: 'failed', detail: 'permission denied', problems: 1 },
    });
    send({
      type: 'finished',
      cleanupId: 1,
      outdatedScan: scanId,
      cleaned: 1,
      withProblems: 1,
      cancelled: false,
      toolsNotRun: ['paccache -rk2'],
      canUndo: false,
    });

    if (store.phase.kind !== 'done') {
      throw new Error('expected a result');
    }

    expect(store.phase.result.problems).toEqual([{ path: '/bad', detail: 'permission denied' }]);
    expect(store.phase.result.toolsNotRun).toEqual(['paccache -rk2']);
    expect(store.phase.result.freed).toBeUndefined();
    // A rule with a problem is not marked as cleaned.
    expect(store.cleaned.has(rule)).toBe(false);
  });

  it('cancels a run: asks the backend once the id is known, and ignores a second click', async () => {
    const { store, resolve, cancelCleanup } = await held();

    await store.cancel();
    expect(store.phase.kind === 'running' && store.phase.cancelling).toBe(true);
    expect(store.actions).toEqual([]);
    expect(cancelCleanup).not.toHaveBeenCalled();

    resolve(9);
    await vi.waitFor(() => expect(cancelCleanup).toHaveBeenCalledWith(9));
    await store.cancel();

    expect(cancelCleanup).toHaveBeenCalledOnce();
  });

  it('cancels at once when the id is already known', async () => {
    const { store, resolve, cancelCleanup } = await held();

    resolve(4);
    await vi.waitFor(() => expect(store.phase.kind === 'running' && store.phase.cleanupId).toBe(4));
    expect(store.actions).toEqual(['cancel']);

    await store.cancel();

    expect(cancelCleanup).toHaveBeenCalledWith(4);
  });

  it('ignores events of a run that is not the current one', async () => {
    const { store, send, scanId } = await held();

    send({
      type: 'finished',
      cleanupId: 1,
      outdatedScan: scanId,
      cleaned: 0,
      withProblems: 0,
      cancelled: true,
      toolsNotRun: [],
      canUndo: false,
    });
    send({
      type: 'finished',
      cleanupId: 2,
      outdatedScan: scanId,
      cleaned: 0,
      withProblems: 0,
      freed: 999,
      cancelled: false,
      toolsNotRun: [],
      canUndo: false,
    });

    expect(store.freedTotal).toBe(0);
  });

  describe('the groups of a run', () => {
    const removed = { kind: 'removed', problems: 0 } as const;

    it('lists every planned rule as queued, then turns each one as its group finishes', async () => {
      const { store, send } = await held();
      const [first, second] = store.plan?.groups ?? [];

      expect(second).toBeDefined();
      expect([...store.groups]).toEqual((store.plan?.groups ?? []).map((group) => [group.ruleId, 'queued']));

      send({ type: 'groupStarted', ruleId: first?.ruleId ?? '', targets: 1 });
      send({ type: 'targetDone', ruleId: first?.ruleId ?? '', path: '/a', allocated: 1, outcome: removed });
      expect(store.groups.get(first?.ruleId ?? '')).toBe('queued');

      send({ type: 'groupFinished', ruleId: first?.ruleId ?? '' });
      expect(store.groups.get(first?.ruleId ?? '')).toBe('done');
      expect(store.groups.get(second?.ruleId ?? '')).toBe('queued');
    });

    it('marks a group that had a problem, and a group that did nothing, as a problem', async () => {
      const { store, send } = await held();
      const [first, second] = store.plan?.groups ?? [];

      send({
        type: 'targetDone',
        ruleId: first?.ruleId ?? '',
        path: '/a',
        allocated: 1,
        outcome: { kind: 'failed', detail: 'busy', problems: 0 },
      });
      send({ type: 'groupFinished', ruleId: first?.ruleId ?? '' });
      send({ type: 'groupFinished', ruleId: second?.ruleId ?? '' });

      expect(store.groups.get(first?.ruleId ?? '')).toBe('problem');
      expect(store.groups.get(second?.ruleId ?? '')).toBe('problem');
    });

    it('drops the groups a cancelled run never reached, and keeps the finished ones', async () => {
      const { store, send, scanId } = await held();
      const [first, second] = store.plan?.groups ?? [];

      send({ type: 'targetDone', ruleId: first?.ruleId ?? '', path: '/a', allocated: 1, outcome: removed });
      send({ type: 'groupFinished', ruleId: first?.ruleId ?? '' });
      send({
        type: 'finished',
        cleanupId: 7,
        outdatedScan: scanId,
        cleaned: 1,
        withProblems: 0,
        freed: 1,
        cancelled: true,
        toolsNotRun: [],
        canUndo: false,
      });

      expect(store.groups.get(first?.ruleId ?? '')).toBe('done');
      expect(store.groups.has(second?.ruleId ?? '')).toBe(false);
    });

    it('stays after the run for a result to show, and a new scan forgets it', async () => {
      const { store, send, scanId, data } = await held();
      const first = store.plan?.groups[0];

      send({ type: 'targetDone', ruleId: first?.ruleId ?? '', path: '/a', allocated: 1, outcome: removed });
      send({ type: 'groupFinished', ruleId: first?.ruleId ?? '' });
      send({
        type: 'finished',
        cleanupId: 7,
        outdatedScan: scanId,
        cleaned: 1,
        withProblems: 0,
        freed: 1,
        cancelled: false,
        toolsNotRun: [],
        canUndo: false,
      });
      expect(store.groups.get(first?.ruleId ?? '')).toBe('done');

      store.load(scanId + 1, data);
      expect(store.groups.size).toBe(0);
    });

    it('keeps the selection when the same scan is loaded again while a cleanup runs', async () => {
      const { store, scanId, data } = await held();

      // Something the reset to "safe only" would take away.
      store.selected.add(REVIEW_RULE);
      store.load(scanId, { ...data });

      expect(store.selected.has(REVIEW_RULE)).toBe(true);
      expect(store.phase.kind).toBe('running');
    });
  });
});

describe('progressTitle', () => {
  const group = (ruleId: string, title: string, elevated: boolean) =>
    ({
      ruleId,
      title,
      verdict: 'safe',
      action: elevated ? 'runTool' : 'delete',
      items: 1,
      allocated: 1,
      needsElevation: elevated,
      samplePaths: [],
    }) as PlanGroupDto;
  const groups = [
    group('node-modules', 'node_modules', false),
    group('journal', 'System journal', true),
    group('pacman-cache', 'Pacman cache', true),
  ];

  it('names every administrator tool on the first of them, which runs them all', () => {
    expect(progressTitle(groups, 'journal')).toBe('System journal, Pacman cache');
    expect(progressTitle(groups, 'pacman-cache')).toBe('Pacman cache');
    expect(progressTitle(groups, 'node-modules')).toBe('node_modules');
  });

  it('names a lone administrator tool by itself', () => {
    expect(progressTitle(groups.slice(0, 2), 'journal')).toBe('System journal');
  });
});
