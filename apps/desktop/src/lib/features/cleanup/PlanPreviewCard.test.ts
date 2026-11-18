import { render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Api } from '../../api/client';
import type { CleanupPlanDto, PlanGroupDto } from '../../api/types';
import { settings } from '../../state/settings.svelte';
import PlanPreviewCard from './PlanPreviewCard.svelte';
import { requestOf, type FirstPreview } from './preview';
import { DATA, fixture, group, planOf, type Fixture } from './testFixture';

beforeEach(() => {
  settings.set('units', 'GB');
  settings.reduceMotion = true;
});

afterEach(() => {
  document.querySelector('main')?.remove();
  vi.useRealTimers();
  settings.reduceMotion = false;
});

type Planner = Api['planCleanup'];

// The fixture's own selection is the three safe rows: target, trash and thin.
const trashed = (id: string, gb: number): PlanGroupDto => ({ ...group(id, 'safe', gb), action: 'trash' });
const SAMPLE = planOf([
  trashed('target', 10),
  { ...group('trash', 'safe', 5), action: 'emptyTrash' },
  {
    ...group('thin', 'safe', 2),
    action: 'runTool',
    command: 'pacman -Sc',
    needsElevation: true,
  },
]);

/** Shows the card over a fixture store, with the clock stopped so the debounce can be stepped through. */
async function show(
  planner: Planner,
  change: (context: Fixture) => void = () => {},
  initial?: (key: string) => FirstPreview,
) {
  const context = await fixture();
  const api = { planCleanup: vi.fn<Planner>(planner) };

  change(context);
  vi.useFakeTimers();

  // The page keeps 20 px at its bottom, as the app's main area does.
  const main = document.body.appendChild(document.createElement('main'));

  main.style.paddingBottom = '20px';
  render(PlanPreviewCard, {
    props: {
      store: context.store,
      scanId: context.scanId,
      api,
      initial: initial?.(requestOf(context.store.selectedItems, settings).key),
    },
    target: main,
  });

  return { ...context, api };
}

/** Lets the debounce run out and the answer arrive. */
async function settle(ms = 300): Promise<void> {
  await vi.advanceTimersByTimeAsync(ms);
  await tick();
}

const row = (name: RegExp | string): HTMLElement => screen.getByRole('row', { name });

describe('PlanPreviewCard', () => {
  it('splits the selection into what happens to it, with a share bar and a legend', async () => {
    await show(async () => SAMPLE);
    await settle();

    const bar = screen.getByRole('img', {
      name: 'Moved to the trash: 10 GB, Deleted for good: 5.0 GB, Cleaned by its own tool: 2.0 GB',
    });

    expect(bar.querySelectorAll('i')).toHaveLength(3);
    expect(screen.getByText('Undo from History for 30 days')).toBeTruthy();
    expect(screen.getByText('Cannot be undone')).toBeTruthy();
    expect(screen.getByText('The tool decides what it removes')).toBeTruthy();
    expect(screen.getByText('3 ITEMS · 17 GB')).toBeTruthy();
  });

  it('leaves an outcome nothing goes to out of the bar and the legend', async () => {
    await show(async () => planOf([trashed('target', 10)]));
    await settle();

    expect(screen.getByRole('img', { name: 'Moved to the trash: 10 GB' }).querySelectorAll('i')).toHaveLength(
      1,
    );
    expect(screen.queryByText('Cannot be undone')).toBeNull();
    expect(screen.queryByText('The tool decides what it removes')).toBeNull();
  });

  it('lists one row per group with its path, what is done, how it comes back and its size', async () => {
    await show(async () => SAMPLE);
    await settle();

    const target = row(/Title of target/);

    expect(within(target).getByText('~/target')).toBeTruthy();
    expect(
      within(within(target).getAllByRole('cell')[1] as HTMLElement).getByText('Moved to the trash'),
    ).toBeTruthy();
    expect(within(target).getByText('Reason of target.')).toBeTruthy();
    expect(within(target).getByText('10 GB')).toBeTruthy();
    expect(within(row(/Title of trash/)).getByText('Emptied for good')).toBeTruthy();
  });

  it('is reachable from the keyboard and named for screen readers, each row saying what happens to it', async () => {
    await show(async () => SAMPLE);
    await settle();

    const region = screen.getByRole('region', { name: 'What happens when you clean, by item' });

    expect(region.getAttribute('tabindex')).toBe('0');
    expect(within(region).getByRole('table', { name: 'What happens to each item' })).toBeTruthy();
    expect(row(/Title of target/).textContent).toContain('Moved to the trash: ');
    expect(row(/Title of trash/).textContent).toContain('Deleted for good: ');
    expect(row(/Title of thin/).textContent).toContain('Cleaned by its own tool: ');
  });

  it('shows the command of a tool, and says when it asks for the administrator password', async () => {
    await show(async () => SAMPLE);
    await settle();

    const tool = row(/Title of thin/);

    expect(within(tool).getByText('pacman -Sc')).toBeTruthy();
    expect(within(tool).getByText('admin password')).toBeTruthy();
    expect(within(row(/Title of target/)).queryByText('admin password')).toBeNull();
  });

  it('says how many items are left alone', async () => {
    const refused = { ruleId: 'target', path: '/p', reason: 'Kept.' };

    await show(async () => ({ ...SAMPLE, refused: [refused], refusedMore: 2 }));
    await settle();

    expect(screen.getByText('3 items will be left alone')).toBeTruthy();
  });

  it('asks nothing and says what to do when nothing is selected', async () => {
    const { api } = await show(
      async () => SAMPLE,
      (context) => context.store.select('none'),
    );

    await settle(1000);

    expect(screen.getByText('Select items in the Advisor to see what cleaning them does.')).toBeTruthy();
    expect(api.planCleanup).not.toHaveBeenCalled();
  });

  it('plans the selected ids with the settings of the Cleanup card', async () => {
    const { api, scanId } = await show(async () => SAMPLE);

    await settle();

    expect(api.planCleanup).toHaveBeenCalledWith(scanId, ['target', 'trash', 'thin'], {
      keepPackageVersions: settings.keepPackageVersions,
      askForAdmin: settings.askForAdmin,
    });
  });

  it('says so in one line when the plan cannot be made, and the page stays', async () => {
    await show(async () => {
      throw new Error('The backend is not ready.');
    });
    await settle();

    expect(screen.getByRole('alert').textContent).toBe('The backend is not ready.');
    expect(screen.queryByRole('table')).toBeNull();
  });

  it('waits for the selection to stop changing before it asks', async () => {
    const { api, store } = await show(async () => SAMPLE);

    await settle(200);
    store.toggle('thin');
    await settle(200);
    store.toggle('trash');
    await settle(299);

    expect(api.planCleanup).not.toHaveBeenCalled();

    await settle(1);

    expect(api.planCleanup).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.planCleanup).mock.calls[0]?.[1]).toEqual(['target']);
  });

  it('asks nothing while a plan is under review or a cleanup runs, and asks again once the flow is idle', async () => {
    const { api, store } = await show(
      async () => SAMPLE,
      (context) => {
        context.store.phase = { kind: 'review', plan: SAMPLE };
      },
    );

    await settle(1000);

    expect(api.planCleanup).not.toHaveBeenCalled();

    store.phase = {
      kind: 'running',
      progress: { group: undefined, done: 0, total: 3 },
      cleanupId: 1,
      cancelling: false,
    };
    store.toggle('thin');
    await settle(1000);

    expect(api.planCleanup).not.toHaveBeenCalled();

    store.phase = { kind: 'idle' };
    await settle();

    expect(api.planCleanup).toHaveBeenCalledTimes(1);
  });

  it('keeps the table on screen while a new plan is on its way, and shows only the answer for the last selection', async () => {
    const pending: ((plan: CleanupPlanDto) => void)[] = [];
    const { api, store } = await show((_scan, ids) =>
      ids.length === 3
        ? Promise.resolve(SAMPLE)
        : new Promise<CleanupPlanDto>((resolve) => pending.push(resolve)),
    );

    await settle();
    expect(row(/Title of target/)).toBeTruthy();

    // Two changes, and the answer to the first one comes last: it describes a selection that is gone.
    store.toggle('thin');
    await settle();
    store.toggle('trash');
    await settle();

    expect(api.planCleanup).toHaveBeenCalledTimes(3);
    expect(row(/Title of thin/)).toBeTruthy();

    pending[1]?.(planOf([trashed('target', 3)]));
    await settle(0);
    pending[0]?.(planOf([trashed('target', 3), trashed('trash', 4)]));
    await settle(0);

    expect(screen.queryByRole('row', { name: /Title of trash/ })).toBeNull();
    expect(screen.queryByRole('row', { name: /Title of thin/ })).toBeNull();
    expect(row(/Title of target/)).toBeTruthy();
    expect(screen.getByText('1 ITEM · 3.0 GB')).toBeTruthy();
  });

  it('drops an answer that lands after the flow left idle, and leaves no spinner behind', async () => {
    let answer: (plan: CleanupPlanDto) => void = () => {};
    const { store } = await show(() => new Promise<CleanupPlanDto>((resolve) => (answer = resolve)));

    // The request is on its way and the spinner is up.
    await settle(600);
    expect(screen.getByRole('status')).toBeTruthy();

    // The user clicks Clean: the review dialog opens, and then the late answer arrives.
    store.phase = { kind: 'review', plan: SAMPLE };
    await settle(0);
    answer(SAMPLE);
    await settle(1500);

    expect(screen.queryByRole('table')).toBeNull();
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('does not plan again when the suggestions are reloaded with the same selection', async () => {
    const { api, store, scanId } = await show(async () => SAMPLE);

    await settle();
    expect(api.planCleanup).toHaveBeenCalledTimes(1);

    store.load(scanId, { ...DATA, suggestions: DATA.suggestions.map((item) => ({ ...item })) });
    await settle(1000);

    expect(api.planCleanup).toHaveBeenCalledTimes(1);
    expect(row(/Title of target/)).toBeTruthy();
  });

  it('drops the plan once the scan is out of date, so an old plan is not shown', async () => {
    const { api, scan, scanId } = await show(async () => SAMPLE);

    await settle();
    expect(row(/Title of target/)).toBeTruthy();

    scan.markOutdated(scanId);
    await settle(1000);

    expect(screen.queryByRole('table')).toBeNull();
    expect(api.planCleanup).toHaveBeenCalledTimes(1);
  });

  it('starts from the first preview of the page and does not ask for the same request again', async () => {
    const { api, store } = await show(
      async () => planOf([trashed('target', 3)]),
      () => {},
      (key) => ({ key, plan: SAMPLE }),
    );

    // On screen at once, before the debounce could have run.
    expect(row(/Title of target/)).toBeTruthy();
    expect(screen.getByText('3 ITEMS · 17 GB')).toBeTruthy();

    await settle(1000);

    expect(api.planCleanup).not.toHaveBeenCalled();

    // Another selection goes through the debounce as always.
    store.toggle('thin');
    await settle();

    expect(api.planCleanup).toHaveBeenCalledTimes(1);
    expect(screen.getByText('1 ITEM · 3.0 GB')).toBeTruthy();
  });

  it('shows the failure of the first preview, then asks again so a passing error does not stay', async () => {
    const { api } = await show(
      async () => SAMPLE,
      () => {},
      (key) => ({ key, failure: 'The backend is not ready.' }),
    );

    expect(screen.getByRole('alert').textContent).toBe('The backend is not ready.');

    await settle(1000);

    expect(api.planCleanup).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(row(/Title of target/)).toBeTruthy();
  });

  it('does not ask again when the flow is back to idle with the selection it has a plan for', async () => {
    const { api, store } = await show(async () => SAMPLE);

    await settle();
    expect(api.planCleanup).toHaveBeenCalledTimes(1);

    store.phase = { kind: 'review', plan: SAMPLE };
    await settle(500);
    store.phase = { kind: 'idle' };
    await settle(1000);

    expect(api.planCleanup).toHaveBeenCalledTimes(1);
    expect(row(/Title of target/)).toBeTruthy();
  });

  it('puts the note of each outcome under its name, in one column next to the swatch', async () => {
    await show(async () => SAMPLE);
    await settle();

    const column = screen.getByText('Cannot be undone').parentElement as HTMLElement;

    expect(within(column).getByText('Deleted for good')).toBeTruthy();
    expect(column.parentElement?.firstElementChild?.classList.contains('swatch')).toBe(true);
  });

  it('says nothing can be cleaned when the plan has no group', async () => {
    await show(async () => planOf([]));
    await settle();

    expect(screen.getByText('Nothing from this selection can be cleaned.')).toBeTruthy();
    expect(screen.queryByRole('table')).toBeNull();
  });

  describe('height', () => {
    const card = (): HTMLElement => document.querySelector('.plan-preview') as HTMLElement;
    const innerHeight = window.innerHeight;
    const matchMedia = window.matchMedia;

    /** jsdom has no layout: the card is told it starts 300 px down the page. */
    beforeEach(() => {
      Object.defineProperty(HTMLElement.prototype, 'offsetTop', {
        configurable: true,
        get(this: HTMLElement) {
          return this.classList.contains('plan-preview') ? 300 : 0;
        },
      });
    });

    afterEach(() => {
      delete (HTMLElement.prototype as unknown as Record<string, unknown>).offsetTop;
      window.innerHeight = innerHeight;
      window.matchMedia = matchMedia;
    });

    it('fills the window from its top down to the bottom padding of the page, and follows the window', async () => {
      window.innerHeight = 900;
      await show(async () => SAMPLE);

      // 900 minus 300 above it minus the 20 px the page keeps at its bottom.
      expect(card().style.maxHeight).toBe('580px');

      window.innerHeight = 1000;
      window.dispatchEvent(new Event('resize'));
      await tick();

      expect(card().style.maxHeight).toBe('680px');
    });

    it('is never shorter than 240 px, however short the window is', async () => {
      window.innerHeight = 400;
      await show(async () => SAMPLE);

      expect(card().style.maxHeight).toBe('240px');
    });

    it('has no cap in a single column', async () => {
      window.innerHeight = 900;
      window.matchMedia = (query: string) => ({ ...matchMedia(query), matches: true }) as MediaQueryList;
      await show(async () => SAMPLE);

      expect(card().style.maxHeight).toBe('');
    });
  });
});
