import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Api } from '../../api/client';
import type { HistorySampleDto } from '../../api/types';
import { ApiRequestError } from '../../api/client';
import { createMockHistory, type MockHistoryVariant } from '../../api/mock-history';
import { LOADING_DELAY_MS, LOADING_MIN_VISIBLE_MS } from '../../state/delayed.svelte';
import { CleanupStore } from '../../state/cleanup.svelte';
import { createMockApi } from '../../api/mock';
import { HistoryStore } from '../../state/history.svelte';
import { settings } from '../../state/settings.svelte';
import { ui } from '../../state/ui.svelte';
import { DATA, fixture, GB, type Fixture } from '../cleanup/testFixture';
import History from './History.svelte';

const NOW = Math.floor(new Date(2026, 10, 17, 12).getTime() / 1000);

type HistoryApi = Pick<Api, 'historyOverview' | 'restoreHistoryCleanup'>;

const storeFor = (api: HistoryApi): HistoryStore => new HistoryStore(api as Api, ui);

const mock = (variant?: MockHistoryVariant): HistoryApi => createMockHistory({ variant, now: () => NOW });

/** The cleanup store of a finished scan with three safe suggestions (105 GB) and a review one. */
const withSuggestions = async (): Promise<Fixture> => fixture();

async function open(api: HistoryApi, suggestions?: Fixture): Promise<HistoryStore> {
  const store = storeFor(api);

  render(History, {
    props: { store, ...(suggestions && { suggestions: suggestions.store, scanId: suggestions.scanId }) },
  });
  await screen.findByRole('heading', { name: /^Cleanups/ });

  return store;
}

/** The row of the Cleanups table that says `text`: the same words are also in the list of what is cleaned most. */
const row = (text: string | RegExp): HTMLElement =>
  within(screen.getByRole('region', { name: 'Cleanups table' }))
    .getByText(text)
    .closest('tr') ?? (document.body as HTMLElement);

describe('the History view', () => {
  beforeEach(() => {
    settings.set('units', 'GB');
  });

  it('shows the four tiles, the chart with its legend and the cleanups table', async () => {
    await open(mock());

    expect(screen.getByText('HISTORY')).toBeTruthy();
    expect(screen.getByText('last 90 days · nvme0n1p2')).toBeTruthy();
    expect(screen.getByText('156 GB')).toBeTruthy();
    expect(screen.getByText('freed in 5 cleanups')).toBeTruthy();
    expect(screen.getByText(/^\+3\.\d GB$/)).toBeTruthy();
    expect(screen.getByText('~5 months')).toBeTruthy();
    // The tile of what can be restored, and the row that is in the trash.
    expect(screen.getAllByText('31 GB')).toHaveLength(2);

    const chart = screen.getByRole('img', { name: /Used space of the disk/ });

    expect(within(chart as unknown as HTMLElement).getByText('156 GB saved')).toBeTruthy();
    expect(within(chart as unknown as HTMLElement).getByText('today')).toBeTruthy();
    expect(screen.getByText('your disk')).toBeTruthy();
    // The legend and the end label.
    expect(screen.getAllByText('without cleanups')).toHaveLength(2);

    // A marker for each cleanup, each with its tooltip.
    const titles = [...chart.querySelectorAll('.marker title')].map((title) => title.textContent);

    expect(titles).toHaveLength(5);
    expect(titles[1]).toBe('28 Oct · Rust build artifacts · node_modules · −74 GB · deleted');

    expect(screen.getAllByRole('row')).toHaveLength(6);
    expect(within(row('Pacman package cache')).getByText('paccache -rk2')).toBeTruthy();
    expect(within(row('Pacman package cache')).getByText('permanent')).toBeTruthy();
    expect(screen.queryByText(/Export CSV/)).toBeNull();
  });

  it('offers Restore for the run in the trash only', async () => {
    await open(mock());

    expect(screen.getAllByRole('button', { name: 'Restore' })).toHaveLength(1);
    expect(within(row('Old recordings · Old downloads')).getByText('in trash')).toBeTruthy();
  });

  it('shows how far a thin history has come, and when the growth rate and the time until full will show', async () => {
    await open(mock('thin'));

    const days = screen.getByRole('progressbar', { name: 'Days recorded' });

    expect(days.getAttribute('aria-valuenow')).toBe('3');
    expect(days.getAttribute('aria-valuemax')).toBe('7');
    expect(screen.getByText('3').closest('b')?.textContent).toBe('3 of 7 days');
    expect(screen.getByText('growth rate shows in 4 days')).toBeTruthy();
    // The free space of the disk stands in for the time until full: 1900 GB total, 1408 GB used.
    expect(screen.getByText('492 GB').closest('b')?.textContent).toBe('492 GB free');
    expect(screen.getByText('time until full shows in 4 days')).toBeTruthy();
    expect(screen.queryByText(/not enough days/)).toBeNull();
    expect(
      screen.getByText(
        'Platter records disk usage once a day while it is open. 3 days so far. The growth rate shows after 7.',
      ),
    ).toBeTruthy();
    expect(screen.queryByText(/\d saved/)).toBeNull();
  });

  it('says "1 day" when one day is left', async () => {
    const api = mock('thin');
    const thin = await api.historyOverview(90);
    const sample = thin.samples[0] as HistorySampleDto;
    const week = Array.from({ length: 6 }, (_, index) => ({ ...sample, day: `2026-11-${10 + index}` }));

    await open({ ...api, historyOverview: async () => ({ ...thin, samples: week }) });

    expect(screen.getByText('growth rate shows in 1 day')).toBeTruthy();
    expect(screen.getByText('time until full shows in 1 day')).toBeTruthy();
  });

  it('shows a first day with an empty ring, and no figure that is not there', async () => {
    await open(mock('empty'));

    expect(screen.getByRole('progressbar', { name: 'Days recorded' }).getAttribute('aria-valuenow')).toBe(
      '0',
    );
    expect(screen.getByText('0').closest('b')?.textContent).toBe('0 of 7 days');
    expect(screen.getByText('First sample').closest('b')?.textContent).toBe('First sample today');
    expect(screen.getByText('growth rate shows in 7 days')).toBeTruthy();
    expect(screen.getByText('time until full shows in 7 days')).toBeTruthy();
  });

  it('says the trash is clear when nothing can be restored', async () => {
    await open(mock('thin'));

    expect(screen.getByText('Trash is clear')).toBeTruthy();
    expect(screen.getByText('nothing waiting to be restored')).toBeTruthy();
    expect(screen.queryByText('nothing')).toBeNull();
  });

  it('shows an empty history without a chart, and says what is recorded and when', async () => {
    await open(mock('empty'));

    expect(screen.getByText('last 90 days')).toBeTruthy();
    expect(screen.queryByRole('img', { name: /Used space/ })).toBeNull();
    expect(screen.getByText('The chart appears once a day is recorded.')).toBeTruthy();
    expect(screen.getByText(/No day is recorded yet\./)).toBeTruthy();
    expect(screen.getByText('0 B')).toBeTruthy();
  });

  it('says a disk that does not grow is not growing, and not that there are too few days', async () => {
    await open(mock('flat'));

    expect(screen.getByText('no growth')).toBeTruthy();
    expect(screen.getByText('not at this rate')).toBeTruthy();
    expect(screen.queryByText(/of 7 days/)).toBeNull();
  });

  it('shows the problem of a history file that cannot be used, and nothing else', async () => {
    render(History, { props: { store: storeFor(mock('broken')) } });

    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(screen.getByText(/The history file could not be read/)).toBeTruthy();
    expect(screen.queryByRole('heading', { name: /^Cleanups/ })).toBeNull();
    expect(screen.queryByRole('img')).toBeNull();
  });

  it('shows why the history could not be read when the backend fails', async () => {
    const api: HistoryApi = {
      historyOverview: () => Promise.reject(new ApiRequestError('io', 'the backend is gone')),
      restoreHistoryCleanup: mock().restoreHistoryCleanup,
    };

    render(History, { props: { store: storeFor(api) } });

    expect(await screen.findByText('The history could not be read: the backend is gone')).toBeTruthy();
  });

  it('keeps the last history on screen and says it may be out of date when a later read fails', async () => {
    const working = mock();
    let fail = false;
    const api: HistoryApi = {
      historyOverview: (days) =>
        fail
          ? Promise.reject(new ApiRequestError('invalid', 'try again in a moment'))
          : working.historyOverview(days),
      restoreHistoryCleanup: working.restoreHistoryCleanup,
    };
    const store = await open(api);

    fail = true;
    await store.load();

    expect(
      await screen.findByText(
        'The history could not be read again, so this may be out of date: try again in a moment',
      ),
    ).toBeTruthy();
    expect(screen.getByText('156 GB')).toBeTruthy();

    fail = false;
    await store.load();

    await waitFor(() => expect(screen.queryByText(/could not be read again/)).toBeNull());
  });

  it('starts a new read for a call made right after a read ended, and leaves nothing to read twice', async () => {
    const working = mock();
    let calls = 0;
    const api: HistoryApi = {
      historyOverview: (days) => {
        calls += 1;

        return working.historyOverview(days);
      },
      restoreHistoryCleanup: working.restoreHistoryCleanup,
    };
    const store = storeFor(api);

    const first = store.load();

    // A call in the microtasks after the read's loop ended, before `first` settles for its awaiters.
    void first.then(() => store.load());
    await first;
    await waitFor(() => expect(calls).toBe(2));
    await store.load();

    expect(calls).toBe(3);
  });

  it('never has more than one read of the history at the backend, and reads once more for calls made meanwhile', async () => {
    const working = mock();
    let inFlight = 0;
    let most = 0;
    let calls = 0;
    const api: HistoryApi = {
      historyOverview: async (days) => {
        calls += 1;
        inFlight += 1;
        most = Math.max(most, inFlight);

        try {
          return await working.historyOverview(days);
        } finally {
          inFlight -= 1;
        }
      },
      restoreHistoryCleanup: working.restoreHistoryCleanup,
    };
    const store = storeFor(api);

    await Promise.all([store.load(), store.load(), store.load()]);

    expect(most).toBe(1);
    expect(calls).toBe(2);
  });
});

describe('the Cleanups card without a cleanup', () => {
  beforeEach(() => {
    settings.set('units', 'GB');
    ui.go('history');
  });

  it('says what the scan found that is safe, names the biggest kinds, and goes to Cleanup from its button', async () => {
    await open(mock('empty'), await withSuggestions());

    expect(screen.getByRole('heading', { name: 'No cleanups yet' })).toBeTruthy();
    expect(screen.queryByText(/No cleanup in the last/)).toBeNull();

    const text = screen.getByText(/This scan found/).textContent;

    expect(text).toContain('This scan found 105 GB Platter can free safely: ');
    expect(text).toContain('Title of target, Title of trash and Title of thin.');
    expect(text).toContain('Every cleanup you run shows here, and what went to the trash can be restored.');
    expect(text).not.toContain(';');

    await fireEvent.click(screen.getByRole('button', { name: 'Review suggestions' }));

    expect(ui.page).toBe('cleanup');
  });

  it('greys the figure of the scan once a cleanup made it old', async () => {
    const setup = await withSuggestions();

    setup.scan.markOutdated(setup.scanId);
    await open(mock('empty'), setup);

    const figure = document.querySelector('.found') as HTMLElement;

    expect(figure.textContent).toBe('105 GB');
    expect(figure.classList.contains('stale')).toBe(true);
    expect(figure.getAttribute('title')).toMatch(/scan before the cleanup/);
  });

  it('greys the figure after a restart too, when the outdated scan was restored and the store knows no scan yet', async () => {
    const setup = await withSuggestions();

    setup.scan.markOutdated(setup.scanId);

    // A store that was never loaded: its scan is unset, as it is right after the app starts.
    const fresh = new CleanupStore(
      { ...createMockApi({ durationMs: 0 }), getSuggestions: async () => DATA },
      setup.scan,
    );

    await open(mock('empty'), { ...setup, store: fresh });

    const figure = await waitFor(() => {
      const found = document.querySelector('.found') as HTMLElement | null;

      expect(found).not.toBeNull();

      return found as HTMLElement;
    });

    expect(figure.classList.contains('stale')).toBe(true);
  });

  it('offers the review when nothing is safe but something needs review', async () => {
    const setup = await withSuggestions();

    setup.store.load(setup.scanId, {
      suggestions: DATA.suggestions.filter((item) => item.verdict === 'review'),
      totals: { safe: 0, review: 268 * GB, keep: 0 },
    });
    await open(mock('thin'), setup);

    expect(screen.getByText(/This scan found nothing that is safe to free right now\./)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Review suggestions' })).toBeTruthy();
  });

  it('hides the button when the scan found nothing to suggest at all', async () => {
    const setup = await withSuggestions();

    setup.store.load(setup.scanId, { suggestions: [], totals: { safe: 0, review: 0, keep: 0 } });
    await open(mock('empty'), setup);

    expect(screen.getByText(/nothing that is safe to free right now/)).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Review suggestions' })).toBeNull();
  });

  it('fetches the suggestions itself when the store holds none, as when History is opened first', async () => {
    const setup = await withSuggestions();

    setup.store.load(setup.scanId, undefined);
    await open(mock('empty'), setup);

    expect((await screen.findByText(/This scan found/)).textContent).toMatch(/found \d+ GB Platter can free/);
  });

  it('says nothing about the scan when its suggestions cannot be read', async () => {
    const setup = await fixture((api) => ({
      ...api,
      getSuggestions: () => Promise.reject(new ApiRequestError('io', 'the backend is gone')),
    }));

    setup.store.load(setup.scanId, undefined);
    await open(mock('empty'), setup);

    expect(screen.queryByText(/This scan found/)).toBeNull();
    expect(screen.getByText(/Every cleanup you run shows here/)).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Review suggestions' })).toBeNull();
  });

  it('shows no such card when there are cleanups', async () => {
    await open(mock(), await withSuggestions());

    expect(screen.queryByRole('heading', { name: 'No cleanups yet' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Review suggestions' })).toBeNull();
  });
});

describe('Restore', () => {
  beforeEach(() => {
    settings.set('units', 'GB');
  });

  it('shows progress, then the row says "restored" and nothing is left to restore', async () => {
    const api = mock();
    const original = api.restoreHistoryCleanup.bind(api);
    let finish: () => void = () => undefined;
    const restoring = vi.spyOn(api, 'restoreHistoryCleanup').mockImplementation(async (id) => {
      await new Promise<void>((resolve) => {
        finish = resolve;
      });

      return original(id);
    });

    await open(api);
    await fireEvent.click(screen.getByRole('button', { name: 'Restore' }));

    expect(restoring).toHaveBeenCalledWith(5);
    expect(screen.getByRole('button', { name: 'Restoring…' }).hasAttribute('disabled')).toBe(true);

    finish();

    await waitFor(() =>
      expect(within(row('Old recordings · Old downloads')).getByText('restored')).toBeTruthy(),
    );
    expect(screen.queryByRole('button', { name: /Restor/ })).toBeNull();
    expect(screen.getByText('Trash is clear')).toBeTruthy();
    expect(ui.toasts.at(-1)?.title).toBe('Restored 2 items');
  });

  it('shows the items that could not be put back, and offers Restore again', async () => {
    const api = mock();

    vi.spyOn(api, 'restoreHistoryCleanup').mockResolvedValue({
      restored: 1,
      items: [
        { path: '~/Videos/Old recordings', restored: true },
        {
          path: '~/Downloads/Old downloads',
          restored: false,
          error: 'something is already at the old place',
        },
      ],
    });

    await open(api);
    await fireEvent.click(screen.getByRole('button', { name: 'Restore' }));

    const alert = await screen.findByRole('alert');

    expect(alert.textContent).toContain('1 of 2 items could not be put back.');
    expect(alert.textContent).toContain('~/Downloads/Old downloads');
    expect(alert.textContent).toContain('something is already at the old place');
    // The mock still holds the run in the trash, so the user may try again.
    expect(screen.getByRole('button', { name: 'Restore' })).toBeTruthy();
  });

  it('a declined confirmation is quiet: the row stays "in trash" with Restore, no error and no toast', async () => {
    const api = createMockHistory({ now: () => NOW, confirmRestore: () => false });
    const restoring = vi.spyOn(api, 'restoreHistoryCleanup');
    const toasts = ui.toasts.length;

    const store = await open(api);

    await fireEvent.click(screen.getByRole('button', { name: 'Restore' }));

    await waitFor(() => expect(restoring).toHaveBeenCalledWith(5));
    await waitFor(() => expect(store.restoring).toBeUndefined());

    expect(screen.queryByRole('alert')).toBeNull();
    expect(within(row('Old recordings · Old downloads')).getByText('in trash')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Restore' }).hasAttribute('disabled')).toBe(false);
    expect(ui.toasts).toHaveLength(toasts);
    expect(store.failures).toEqual({});
  });

  it('shows the error of a restore that the backend refused', async () => {
    const api = mock();

    vi.spyOn(api, 'restoreHistoryCleanup').mockRejectedValue(
      new ApiRequestError('invalid', 'a cleanup is running'),
    );

    await open(api);
    await fireEvent.click(screen.getByRole('button', { name: 'Restore' }));

    expect((await screen.findByRole('alert')).textContent).toContain('a cleanup is running');
    expect(screen.getByRole('button', { name: 'Restore' }).hasAttribute('disabled')).toBe(false);
  });
});

describe('the first read', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows a spinner after a short wait and keeps it for a second before the content replaces it', async () => {
    const api = mock();
    const inner = api.historyOverview;
    // The data arrives 100 ms after the spinner appeared.
    const slow = vi.fn(async (days: number) => {
      await new Promise((resolve) => setTimeout(resolve, LOADING_DELAY_MS + 100));

      return inner(days);
    });

    render(History, { props: { store: storeFor({ ...api, historyOverview: slow }) } });

    expect(document.querySelector('[role="status"]')).toBeNull();

    await vi.advanceTimersByTimeAsync(LOADING_DELAY_MS + 1);
    expect(screen.getByRole('status')).toBeTruthy();

    // The data is in, but the spinner has not been there for a second yet.
    await vi.advanceTimersByTimeAsync(200);
    expect(screen.getByRole('status')).toBeTruthy();
    expect(screen.queryByRole('heading', { name: /^Cleanups/ })).toBeNull();

    await vi.advanceTimersByTimeAsync(LOADING_MIN_VISIBLE_MS);
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.getByRole('heading', { name: /^Cleanups/ })).toBeTruthy();
  });

  it('shows no spinner at all when the answer is quick', async () => {
    render(History, { props: { store: storeFor(mock()) } });

    await vi.advanceTimersByTimeAsync(0);

    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.getByRole('heading', { name: /^Cleanups/ })).toBeTruthy();
  });
});

describe('what you clean most', () => {
  beforeEach(() => {
    settings.set('units', 'GB');
  });

  it('lists the items by how many cleanups had them, with when each was cleaned last', async () => {
    const api = mock();
    const overview = await api.historyOverview(90);
    const [first, ...others] = overview.cleanups;
    const again = { ...first, id: 99, at: (first?.at ?? 0) + 60, what: ['Old recordings'] };

    await open({
      ...api,
      historyOverview: async () => ({ ...overview, cleanups: [first, ...others, again] }),
    });

    const list = within(screen.getByRole('region', { name: 'Items cleaned most' }));
    const items = list.getAllByRole('listitem').map((item) => item.textContent);

    expect(screen.getByText('LAST 90 DAYS')).toBeTruthy();
    expect(items).toHaveLength(8);
    expect(items[0]).toMatch(/^Old recordings\s*twice\s*last on \d+ \w+$/);
    expect(items[1]).toMatch(/once/);
    expect(items.join(' ')).not.toContain(';');
  });

  it('says it is empty when nothing was cleaned, with no list to scroll', async () => {
    await open(mock('empty'));

    expect(screen.getByText('What you clean most')).toBeTruthy();
    expect(screen.queryByRole('region', { name: 'Items cleaned most' })).toBeNull();
    expect(screen.getByText(/^Nothing cleaned in the last \d+ days\./)).toBeTruthy();
  });

  it('does not stretch the row of an empty history down the page', async () => {
    await open(mock('empty'));

    const row = document.querySelector('.split') as HTMLElement;

    expect(row.classList.contains('empty')).toBe(true);
    expect(row.style.height).toBe('');
  });
});

describe('Ready to clean now', () => {
  beforeEach(() => {
    settings.set('units', 'GB');
    ui.go('history');
  });

  /** The size on the row of the Ready card with this label. */
  const sizeOf = (label: string): string | undefined =>
    screen.getByText(label).closest('.recr')?.querySelector('.num')?.textContent?.trim();

  it('says how much is safe and how much needs a review, and goes to Cleanup', async () => {
    await open(mock(), await withSuggestions());

    expect(screen.getByRole('heading', { name: 'Ready to clean now' })).toBeTruthy();
    expect(sizeOf('Safe to clean')).toBe('105 GB');
    expect(sizeOf('Needs your review')).toBe('268 GB');

    await fireEvent.click(screen.getByRole('button', { name: 'Go to Cleanup' }));

    expect(ui.page).toBe('cleanup');
  });

  it('shows 0 B to review when nothing needs a review, and still goes to Cleanup', async () => {
    const setup = await withSuggestions();

    setup.store.load(setup.scanId, {
      suggestions: DATA.suggestions.filter((item) => item.verdict === 'safe'),
      totals: { safe: 105.4 * GB, review: 0, keep: 0 },
    });
    await open(mock(), setup);

    expect(sizeOf('Safe to clean')).toBe('105 GB');
    expect(sizeOf('Needs your review')).toBe('0 B');
    expect(screen.getByRole('button', { name: 'Go to Cleanup' })).toBeTruthy();
  });

  it('offers a review instead when nothing is safe', async () => {
    const setup = await withSuggestions();

    setup.store.load(setup.scanId, {
      suggestions: DATA.suggestions.filter((item) => item.verdict === 'review'),
      totals: { safe: 0, review: 268 * GB, keep: 0 },
    });
    await open(mock(), setup);

    expect(sizeOf('Safe to clean')).toBe('0 B');
    expect(sizeOf('Needs your review')).toBe('268 GB');
    expect(screen.queryByRole('button', { name: 'Go to Cleanup' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Review in Cleanup' })).toBeTruthy();
  });

  it('greys the bar and rows and tags the card once a cleanup made the scan old', async () => {
    const current = await withSuggestions();

    await open(mock(), current);

    expect(screen.queryByText('OLD SCAN')).toBeNull();
    expect(document.querySelectorAll('.stale')).toHaveLength(0);
    cleanup();

    const old = await withSuggestions();

    old.scan.markOutdated(old.scanId);
    await open(mock(), old);

    expect(screen.getByRole('heading', { name: /^Ready to clean now/ }).textContent).toContain('OLD SCAN');
    expect(screen.getByText('Safe to clean').closest('.recr')?.classList.contains('stale')).toBe(true);
    expect(screen.getByText('Needs your review').closest('.recr')?.classList.contains('stale')).toBe(true);
    expect(screen.getByRole('img', { name: /safe to clean/ }).classList.contains('stale')).toBe(true);
  });

  it('asks for a scan when there is none, and offers no button', async () => {
    await open(mock());

    expect(screen.getByText('Scan a disk to see what can be cleaned.')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Go to Cleanup' })).toBeNull();
  });
});

describe('waiting for the suggestions', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows the page only once the suggestions are in, and does not bring the spinner back for a later read', async () => {
    let release: () => void = () => undefined;
    const setup = await fixture((api) => ({
      ...api,
      getSuggestions: async (...args: Parameters<typeof api.getSuggestions>) => {
        await new Promise<void>((resolve) => {
          release = resolve;
        });

        return api.getSuggestions(...args);
      },
    }));
    const store = storeFor(mock());

    vi.useFakeTimers();
    setup.store.load(setup.scanId, undefined);
    render(History, { props: { store, suggestions: setup.store, scanId: setup.scanId } });

    // The history is in, but the suggestions are not: nothing is shown, then the spinner.
    await vi.advanceTimersByTimeAsync(LOADING_DELAY_MS - 1);
    expect(screen.queryByRole('heading', { name: /^Cleanups/ })).toBeNull();
    expect(screen.queryByRole('status')).toBeNull();

    await vi.advanceTimersByTimeAsync(2);
    expect(screen.getByRole('status')).toBeTruthy();

    release();
    await vi.advanceTimersByTimeAsync(LOADING_MIN_VISIBLE_MS + 1);

    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.getByRole('heading', { name: /^Cleanups/ })).toBeTruthy();
    expect(screen.queryByText('Scan a disk to see what can be cleaned.')).toBeNull();
    expect(screen.getByRole('button', { name: 'Go to Cleanup' })).toBeTruthy();

    void store.load();
    await vi.advanceTimersByTimeAsync(LOADING_DELAY_MS + 1);

    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.getByRole('heading', { name: /^Cleanups/ })).toBeTruthy();
  });

  it('stops waiting when the suggestions cannot be read', async () => {
    const setup = await fixture((api) => ({
      ...api,
      getSuggestions: () => Promise.reject(new ApiRequestError('io', 'the backend is gone')),
    }));

    vi.useFakeTimers();
    setup.store.load(setup.scanId, undefined);
    render(History, { props: { store: storeFor(mock()), suggestions: setup.store, scanId: setup.scanId } });
    await vi.advanceTimersByTimeAsync(0);

    expect(screen.getByRole('heading', { name: /^Cleanups/ })).toBeTruthy();
    // There is a scan, only its suggestions failed: the card says so and does not ask for a scan.
    expect(screen.getByText('What can be cleaned could not be read.')).toBeTruthy();
    expect(screen.queryByText('Scan a disk to see what can be cleaned.')).toBeNull();
    expect(screen.getByRole('button', { name: 'Go to Cleanup' })).toBeTruthy();
  });
});
