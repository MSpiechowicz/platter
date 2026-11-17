import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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
import { advisor } from '../../state/advisor.svelte';
import { CleanupStore } from '../../state/cleanup.svelte';
import { ScanStore } from '../../state/scan.svelte';
import { settings } from '../../state/settings.svelte';
import AdvisorCard from './AdvisorCard.svelte';

const GB = 1_000_000_000;

function suggestion(id: string, verdict: Verdict, gb: number, nodes: number[]): SuggestionDto {
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

const DATA: SuggestionsDto = {
  suggestions: [
    suggestion('target', 'safe', 88, [11, 12]),
    suggestion('trash', 'safe', 17, [13]),
    suggestion('snapshots', 'review', 268, [14]),
    suggestion('steam', 'keep', 142, [15]),
  ],
  totals: { safe: 105 * GB, review: 268 * GB, keep: 142 * GB },
};

const group = (
  id: string,
  verdict: Verdict,
  gb: number,
  extra: Partial<PlanGroupDto> = {},
): PlanGroupDto => ({
  ruleId: id,
  title: `Title of ${id}`,
  verdict,
  action: 'delete',
  items: 1,
  allocated: gb * GB,
  needsElevation: false,
  samplePaths: [`/home/demo/${id}`],
  ...extra,
});

const planOf = (groups: PlanGroupDto[], extra: Partial<CleanupPlanDto> = {}): CleanupPlanDto => ({
  planId: 5,
  scanId: 1,
  options: { keepPackageVersions: 2, askForAdmin: true },
  groups,
  refused: [],
  refusedMore: 0,
  notFound: [],
  allocated: groups.reduce((sum, item) => sum + item.allocated, 0),
  needsConfirmation: true,
  ...extra,
});

interface Fixture {
  scan: ScanStore;
  store: CleanupStore;
  api: Api;
  send: (event: CleanupEvent) => void;
  scanId: number;
}

/** A card over a finished mock scan and a scripted backend that answers every plan with `plan` and holds the run. */
async function fixture(plan: (ids: string[]) => CleanupPlanDto): Promise<Fixture> {
  let send: (event: CleanupEvent) => void = () => {};
  const mock = createMockApi({ durationMs: 0 });
  const api: Api = {
    ...mock,
    planCleanup: vi.fn(async (_scan, ids) => plan(ids)),
    startCleanup: vi.fn(async (_plan, onEvent) => {
      send = onEvent;

      return 77;
    }),
  };
  const scan = new ScanStore(api);

  await scan.start({ root: '/', oneFileSystem: true });
  await waitFor(() => expect(scan.state.status).toBe('ready'));

  if (scan.state.status !== 'ready') {
    throw new Error('expected a finished scan');
  }

  return {
    scan,
    store: new CleanupStore(api, scan),
    api,
    send: (event) => send(event),
    scanId: scan.state.scanId,
  };
}

const safePlan = (ids: string[]): CleanupPlanDto =>
  planOf(ids.map((id) => group(id, DATA.suggestions.find((item) => item.id === id)?.verdict ?? 'safe', 1)));

function show(
  context: Pick<Fixture, 'store' | 'scanId'>,
  props: Partial<{
    suggestions: SuggestionsDto | undefined;
    loading: boolean;
    error: unknown;
    onretry: () => void;
    onopen: (id: number) => void;
  }> = {},
): void {
  render(AdvisorCard, {
    props: {
      suggestions: DATA,
      scanId: context.scanId,
      store: context.store,
      loading: false,
      error: undefined,
      onretry: () => {},
      onopen: () => {},
      ...props,
    },
  });
}

const button = (name: string | RegExp): HTMLButtonElement =>
  screen.getByRole('button', { name }) as HTMLButtonElement;

const checkbox = (name: string): HTMLInputElement =>
  screen.getByRole('checkbox', { name }) as HTMLInputElement;

const footer = (): HTMLElement => screen.getByRole('status');

describe('AdvisorCard', () => {
  let context: Fixture;

  beforeEach(async () => {
    settings.reduceMotion = true;
    context = await fixture(safePlan);
  });

  afterEach(() => {
    settings.reduceMotion = false;
    advisor.status = undefined;
    vi.useRealTimers();
  });

  it('says LOCAL MODEL only while the model is loaded, and RULES whenever it is not', async () => {
    const tag = (): string => screen.getByRole('heading', { name: /^Advisor/ }).textContent ?? '';
    const hint = (state: 'loaded' | 'onDisk' | 'loading' | 'notDownloaded' | 'downloading') => ({
      state,
      model: {
        name: 'Qwen3.5-4B',
        format: 'GGUF · Q4_K_M',
        bytes: 1,
        host: 'huggingface.co',
      },
      options: { enabled: true, unloadAfter: 'fiveMinutes' as const, logQuestions: true, useGpu: true },
      ramTotal: 1,
      gpuSupported: false,
    });

    advisor.status = hint('loaded');
    show(context);
    expect(tag()).toContain('LOCAL MODEL');
    expect(tag()).not.toContain('RULES');

    advisor.status = hint('onDisk');
    await tick();
    expect(tag()).toContain('RULES');
    expect(tag()).not.toContain('LOCAL MODEL');

    for (const state of ['loading', 'notDownloaded', 'downloading'] as const) {
      advisor.status = hint(state);
      await tick();
      expect(tag(), state).toContain('RULES');
    }

    advisor.status = hint('loaded');
    await tick();
    advisor.status = undefined;
    await tick();
    expect(tag()).toContain('RULES');
  });

  it('says it works by rules and counts what is selected of what can be cleaned', () => {
    show(context);

    expect(screen.getByRole('heading', { name: /^Advisor/ }).textContent).toContain('RULES');
    expect(screen.queryByText(/local model/i)).toBeNull();
    expect(screen.getByText('2 of 3 selected')).toBeTruthy();
  });

  it('groups the rows as safe, review, kept, each header with its item count and size', () => {
    show(context);

    // The card itself is a region too; the groups are the ones inside its list.
    const groups = screen.getAllByRole('region').filter((region) => region.closest('.list'));

    expect(groups.map((item) => item.getAttribute('aria-label'))).toEqual([
      'Safe to clean',
      'Needs your review',
      'Kept',
    ]);

    const safe = within(groups[0] as HTMLElement);

    expect(safe.getByRole('heading').textContent).toContain('Safe to clean');
    expect(safe.getByText('2 items · 105 GB')).toBeTruthy();
    expect(within(groups[1] as HTMLElement).getByText('1 item · 268 GB')).toBeTruthy();
    expect(within(groups[2] as HTMLElement).getByText('1 item · 142 GB')).toBeTruthy();
  });

  it('keeps every group header inside the scrolling list, moving with its rows', () => {
    show(context);

    const headers = screen.getAllByRole('heading', { level: 4 });

    expect(headers).toHaveLength(3);

    for (const header of headers) {
      expect(header.classList.contains('gh')).toBe(true);
      expect(header.closest('.list')).toBeTruthy();
    }
  });

  it('shows the title, the path, the reason and the size of every suggestion', () => {
    show(context);

    const row = screen.getByText('Title of snapshots').closest('.sug') as HTMLElement;

    expect(within(row).getByText('Title of snapshots')).toBeTruthy();
    expect(within(row).getByText('~/snapshots')).toBeTruthy();
    expect(within(row).getByText('Reason of snapshots.')).toBeTruthy();
    expect(within(row).getByText('268 GB')).toBeTruthy();
  });

  it('gives each row that can be cleaned a checkbox: safe ticked, review not, kept none', () => {
    show(context);

    expect(screen.getAllByRole('checkbox')).toHaveLength(3);
    expect(checkbox('Title of target').checked).toBe(true);
    expect(checkbox('Title of trash').checked).toBe(true);
    expect(checkbox('Title of snapshots').checked).toBe(false);
    expect(screen.queryByRole('checkbox', { name: 'Title of steam' })).toBeNull();
  });

  it('selects Safe, All and None; All never ticks a kept row', async () => {
    show(context);

    await fireEvent.click(button('All'));
    expect(screen.getByText('3 of 3 selected')).toBeTruthy();
    expect(checkbox('Title of snapshots').checked).toBe(true);
    expect(button('All').getAttribute('aria-pressed')).toBe('true');

    await fireEvent.click(button('None'));
    expect(screen.getByText('0 of 3 selected')).toBeTruthy();
    expect(screen.getAllByRole('checkbox').every((box) => !(box as HTMLInputElement).checked)).toBe(true);
    expect(button('None').getAttribute('aria-pressed')).toBe('true');

    await fireEvent.click(button('Safe'));
    expect(screen.getByText('2 of 3 selected')).toBeTruthy();
    expect(checkbox('Title of snapshots').checked).toBe(false);
    expect(button('Safe').getAttribute('aria-pressed')).toBe('true');
  });

  it('shows no segment as chosen for a selection that is none of them', async () => {
    show(context);

    await fireEvent.click(checkbox('Title of trash'));

    for (const name of ['Safe', 'All', 'None']) {
      expect(button(name).getAttribute('aria-pressed')).toBe('false');
    }
  });

  it('shows the selected total and what it contains, and the two buttons with their sizes', async () => {
    show(context);

    expect(screen.getByText('SELECTED')).toBeTruthy();
    expect(screen.getAllByText('105 GB').length).toBeGreaterThan(0);
    expect(footer().textContent).toContain('All safe items, nothing else.');
    expect(button('Clean safe · 105 GB').disabled).toBe(false);
    expect(button('Clean selected (2)').disabled).toBe(false);

    await fireEvent.click(checkbox('Title of snapshots'));

    expect(footer().textContent).toContain('Includes 1 review item (268 GB). You will confirm them first.');
    expect(button('Clean selected (3)')).toBeTruthy();

    await fireEvent.click(checkbox('Title of snapshots'));
    await fireEvent.click(checkbox('Title of trash'));

    expect(footer().textContent).toContain('1 safe item selected.');
    expect(button('Clean selected (1)')).toBeTruthy();
  });

  it('keeps both buttons, disabled, when nothing is selected', async () => {
    show(context);

    await fireEvent.click(button('None'));

    expect(footer().textContent).toContain('Nothing selected.');
    expect(button('Clean selected (0)').disabled).toBe(true);
    expect(button('Clean safe · 105 GB').disabled).toBe(false);
  });

  it('ticks and unticks a row when its text is clicked: the whole row is the label of its checkbox', async () => {
    show(context);

    expect(checkbox('Title of snapshots').checked).toBe(false);

    await fireEvent.click(screen.getByText('Title of snapshots'));
    expect(checkbox('Title of snapshots').checked).toBe(true);
    expect(screen.getByText('3 of 3 selected')).toBeTruthy();

    await fireEvent.click(screen.getByText('Reason of snapshots.'));
    expect(checkbox('Title of snapshots').checked).toBe(false);

    expect(checkbox('Title of target').checked).toBe(true);
  });

  it('opens the Explorer from the path line, which leaves the checkbox alone', async () => {
    const onopen = vi.fn();
    show(context, { onopen });

    await fireEvent.click(screen.getByText('~/target'));

    expect(onopen).toHaveBeenCalledOnce();
    expect(checkbox('Title of target').checked).toBe(true);
  });

  it('does nothing when the text of a kept row is clicked: it has no checkbox and does not open the Explorer', async () => {
    const onopen = vi.fn();

    show(context, { onopen });

    await fireEvent.click(screen.getByText('Title of steam'));
    await fireEvent.click(screen.getByText('Reason of steam.'));

    expect(onopen).not.toHaveBeenCalled();
    expect(screen.getByText('2 of 3 selected')).toBeTruthy();
  });

  it('opens the Explorer from a small button of its own on every row, on the largest matched node', async () => {
    const onopen = vi.fn();

    show(context, { onopen });

    // Kept rows have one too: nothing is ticked or cleaned by it.
    await fireEvent.click(screen.getByRole('button', { name: 'Open Title of target in the Explorer' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Open Title of steam in the Explorer' }));

    expect(onopen.mock.calls).toEqual([[11], [15]]);
    expect(screen.getAllByRole('button', { name: /in the Explorer$/ })).toHaveLength(4);
  });

  it('does not tick the row when its Explorer button is clicked, and does not open the Explorer from the text', async () => {
    const onopen = vi.fn();

    show(context, { onopen });

    await fireEvent.click(screen.getByRole('button', { name: 'Open Title of snapshots in the Explorer' }));

    expect(onopen).toHaveBeenCalledWith(14);
    expect(checkbox('Title of snapshots').checked).toBe(false);

    await fireEvent.click(screen.getByText('Title of snapshots'));

    expect(onopen).toHaveBeenCalledOnce();
    expect(checkbox('Title of snapshots').checked).toBe(true);
  });

  it('leaves a suggestion without a node nothing to open', () => {
    show(context, {
      suggestions: { ...DATA, suggestions: [suggestion('empty', 'safe', 1, [])], totals: DATA.totals },
    });

    expect((button('Open Title of empty in the Explorer') as HTMLButtonElement).disabled).toBe(true);
  });

  it('lets a disabled row neither tick nor open while a cleanup runs', async () => {
    const context = await fixture(safePlan);
    const onopen = vi.fn();

    show(context, { onopen });
    await fireEvent.click(button('Clean selected (2)'));
    await waitFor(() => expect(context.api.startCleanup).toHaveBeenCalled());

    await fireEvent.click(screen.getByText('Title of snapshots'));
    await fireEvent.click(screen.getByText('Title of target'));
    await fireEvent.click(screen.getByRole('button', { name: 'Open Title of target in the Explorer' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Open Title of steam in the Explorer' }));

    expect(checkbox('Title of snapshots').checked).toBe(false);
    expect(checkbox('Title of target').checked).toBe(true);
    expect((button('Open Title of target in the Explorer') as HTMLButtonElement).disabled).toBe(true);
    expect(onopen).not.toHaveBeenCalled();
  });

  it('lets a disabled row neither tick nor open while the scan is outdated', async () => {
    const onopen = vi.fn();

    show(context, { onopen });
    context.scan.markOutdated(context.scanId);
    await waitFor(() => expect(checkbox('Title of target').disabled).toBe(true));

    await fireEvent.click(screen.getByText('Title of snapshots'));
    await fireEvent.click(screen.getByRole('button', { name: 'Open Title of snapshots in the Explorer' }));

    expect(checkbox('Title of snapshots').checked).toBe(false);
    expect(onopen).not.toHaveBeenCalled();
  });

  it('says so in one sentence when there is nothing to suggest, and keeps the footer, disabled', () => {
    show(context, { suggestions: { suggestions: [], totals: { safe: 0, review: 0, keep: 0 } } });

    expect(screen.getByText('No suggestions')).toBeTruthy();
    expect(screen.getByText(/^Nothing to suggest here/)).toBeTruthy();
    expect(screen.queryAllByRole('checkbox')).toEqual([]);
    expect(screen.getByText('SELECTED')).toBeTruthy();
    expect(footer().textContent).toContain('Nothing to clean here.');
    expect(button('Clean safe').disabled).toBe(true);
    expect(button('Clean selected (0)').disabled).toBe(true);
    expect(button('None').disabled).toBe(true);
  });

  it('shows the logo spinner only after 300 ms of waiting', async () => {
    vi.useFakeTimers();
    show(context, { suggestions: undefined, loading: true });

    expect(screen.queryByRole('status')).toBeNull();

    await vi.advanceTimersByTimeAsync(350);

    expect(screen.getByRole('status').textContent).toContain('Looking for things to clean');
  });

  it('offers "Try again" when the suggestions could not be loaded', async () => {
    const onretry = vi.fn();

    show(context, { suggestions: undefined, error: new Error('boom'), onretry });

    expect(screen.getByRole('alert').textContent).toContain('could not be loaded');

    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    expect(onretry).toHaveBeenCalledTimes(1);
  });
});

describe('AdvisorCard and the rules of the user', () => {
  const USER: SuggestionsDto = {
    suggestions: [
      { ...suggestion('user-safe', 'safe', 30, [21]), title: 'Folders you marked safe', source: 'user' },
      suggestion('target', 'safe', 88, [11]),
      { ...suggestion('user-keep', 'keep', 12, [22]), title: 'Folders you keep', source: 'user' },
    ],
    totals: { safe: 118 * GB, review: 0, keep: 12 * GB },
  };

  beforeEach(() => {
    settings.reduceMotion = true;
  });

  afterEach(() => {
    settings.reduceMotion = false;
  });

  it('lists them like any other suggestion: a safe one is ticked and cleanable, a kept one is shown and has no checkbox', async () => {
    const context = await fixture(safePlan);

    show(context, { suggestions: USER });

    expect(checkbox('Folders you marked safe').checked).toBe(true);
    expect(screen.queryByRole('checkbox', { name: 'Folders you keep' })).toBeNull();
    expect(within(screen.getByRole('region', { name: 'Kept' })).getByText('Folders you keep')).toBeTruthy();
    expect(screen.getByText('2 of 2 selected')).toBeTruthy();
  });

  it('plans a safe folder of the user by its rule id with the other ticked rows', async () => {
    const context = await fixture(safePlan);

    show(context, { suggestions: USER });
    await fireEvent.click(button(/^Clean selected/));

    await waitFor(() => expect(context.api.planCleanup).toHaveBeenCalledOnce());
    expect(context.api.planCleanup).toHaveBeenCalledWith(context.scanId, ['user-safe', 'target'], {
      keepPackageVersions: 2,
      askForAdmin: true,
    });
  });
});

describe('AdvisorCard cleaning', () => {
  beforeEach(() => {
    settings.reduceMotion = true;
  });

  afterEach(() => {
    settings.reduceMotion = false;
  });

  it('plans the ticked rules on "Clean selected" and runs a plan of safe items without the dialog', async () => {
    const context = await fixture(safePlan);

    show(context);
    await fireEvent.click(button('Clean selected (2)'));

    await waitFor(() => expect(context.api.startCleanup).toHaveBeenCalledOnce());
    expect(context.api.planCleanup).toHaveBeenCalledWith(context.scanId, ['target', 'trash'], {
      keepPackageVersions: 2,
      askForAdmin: true,
    });
    expect(document.querySelector('dialog[open]')).toBeNull();
    expect(button('Clean safe · 105 GB').disabled).toBe(true);
    expect(button('Clean selected (2)').disabled).toBe(true);
    expect(footer().textContent).toContain('Starting');
  });

  it('offers no Cancel of its own while the backend dialog is open: that dialog is where to say no', async () => {
    const context = await fixture(safePlan);
    // The backend answers only after the user answers its dialog; here, never.
    context.api.startCleanup = vi.fn(() => new Promise<number>(() => {}));

    show(context);
    await fireEvent.click(button('Clean selected (2)'));

    await waitFor(() => expect(footer().textContent).toContain('Confirm or cancel in the dialog.'));
    expect(within(footer()).queryByRole('button', { name: 'Cancel' })).toBeNull();
  });

  it('"Clean safe" cleans the safe rows only, whatever else is ticked', async () => {
    const context = await fixture(safePlan);

    show(context);
    await fireEvent.click(button('All'));
    await fireEvent.click(button('Clean safe · 105 GB'));

    await waitFor(() => expect(context.api.planCleanup).toHaveBeenCalledOnce());
    expect(context.api.planCleanup).toHaveBeenCalledWith(context.scanId, ['target', 'trash'], {
      keepPackageVersions: 2,
      askForAdmin: true,
    });
  });

  it('shows the review dialog for review items: one row per group, the safe rest, the total, Cancel and Clean', async () => {
    const context = await fixture((ids) =>
      planOf(
        ids.map((id) =>
          id === 'snapshots'
            ? group(id, 'review', 268, { action: 'trash', samplePaths: ['/.snapshots'] })
            : group(id, 'safe', 52.5),
        ),
      ),
    );

    show(context);
    await fireEvent.click(checkbox('Title of snapshots'));
    await fireEvent.click(button('Clean selected (3)'));

    const dialog = (await screen.findByRole('dialog')) as HTMLElement;
    const inside = within(dialog);

    expect(inside.getByRole('heading', { name: 'Clean 1 item that needs review?' })).toBeTruthy();
    expect(inside.getByText('Title of snapshots')).toBeTruthy();
    expect(inside.getByText('/.snapshots')).toBeTruthy();
    expect(inside.getByText('Reason of snapshots.')).toBeTruthy();
    expect(inside.getByText('Moved to the trash. This can be undone.')).toBeTruthy();
    expect(inside.getByText('Plus 2 safe items · 105 GB')).toBeTruthy();
    expect(dialog.textContent).toContain('Total 373 GB');
    expect(inside.getByRole('button', { name: 'Clean 373 GB' })).toBeTruthy();
    expect(document.activeElement).toBe(inside.getByRole('button', { name: 'Cancel' }));
    expect(context.api.startCleanup).not.toHaveBeenCalled();
  });

  it('closes the dialog on Cancel, starts nothing and says "Not cleaned."', async () => {
    const context = await fixture((ids) => planOf(ids.map((id) => group(id, 'review', 1))));

    show(context);
    await fireEvent.click(checkbox('Title of snapshots'));
    await fireEvent.click(button('Clean selected (3)'));
    await fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }));

    await waitFor(() => expect(document.querySelector('dialog[open]')).toBeNull());
    expect(context.api.startCleanup).not.toHaveBeenCalled();
    expect(footer().textContent).toContain('Not cleaned.');
    expect(button('Clean selected (3)').disabled).toBe(false);
  });

  it('starts the cleanup when the dialog is confirmed, and then shows progress with a Cancel in the status', async () => {
    const context = await fixture((ids) => planOf(ids.map((id) => group(id, 'review', 2))));

    show(context);
    await fireEvent.click(checkbox('Title of snapshots'));
    await fireEvent.click(button('Clean selected (3)'));
    await fireEvent.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: /^Clean 6/ }),
    );

    await waitFor(() => expect(context.api.startCleanup).toHaveBeenCalledWith(5, expect.any(Function)));

    context.send({ type: 'groupStarted', ruleId: 'target', targets: 1 });
    context.send({
      type: 'targetDone',
      ruleId: 'target',
      path: '/home/demo/target',
      allocated: 1,
      outcome: { kind: 'removed', problems: 0 },
    });

    await waitFor(() => expect(footer().textContent).toContain('Cleaning Title of target · 1 of 3'));
    expect(button('Cancel').closest('[role="status"]')).toBe(footer());
    expect(button('Clean selected (3)').disabled).toBe(true);
    expect(checkbox('Title of target').disabled).toBe(true);
    expect(button('All').disabled).toBe(true);
  });

  it('shows what was freed, offers Undo only when it can, and Rescan; the clean buttons stay off', async () => {
    const context = await fixture(safePlan);
    const rescan = vi.spyOn(context.scan, 'rescan').mockResolvedValue();

    show(context);
    await fireEvent.click(button('Clean selected (2)'));
    await waitFor(() => expect(context.api.startCleanup).toHaveBeenCalled());

    context.send({
      type: 'finished',
      cleanupId: 77,
      outdatedScan: context.scanId,
      cleaned: 3,
      withProblems: 0,
      freed: 1_200_000_000,
      cancelled: false,
      toolsNotRun: [],
      canUndo: false,
    });

    await waitFor(() => expect(footer().textContent).toContain('Freed 1.2 GB.'));
    expect(footer().textContent).toContain('The disk changed. Rescan to clean again.');
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull();
    expect(button('Clean safe · 105 GB').disabled).toBe(true);
    expect(button('Clean selected (2)').disabled).toBe(true);

    await fireEvent.click(button('Rescan'));
    expect(rescan).toHaveBeenCalledOnce();
  });

  it('offers Undo after a cleanup that moved things to the trash, and shows each failure as text', async () => {
    const context = await fixture(safePlan);

    vi.spyOn(context.api, 'undoCleanup').mockResolvedValue([
      { path: '/home/demo/a', restored: true },
      { path: '/home/demo/\u202Eb', restored: false, error: 'already exists' },
    ]);

    show(context);
    await fireEvent.click(button('Clean selected (2)'));
    await waitFor(() => expect(context.api.startCleanup).toHaveBeenCalled());
    context.send({
      type: 'targetDone',
      ruleId: 'target',
      path: '/home/demo/target',
      allocated: 1,
      outcome: { kind: 'trashed', problems: 0 },
    });
    context.send({
      type: 'finished',
      cleanupId: 77,
      outdatedScan: context.scanId,
      cleaned: 1,
      withProblems: 0,
      freed: 0,
      cancelled: false,
      toolsNotRun: [],
      canUndo: true,
    });

    await waitFor(() =>
      expect(footer().textContent).toContain('Moved to the trash: 1 item. This can be undone.'),
    );
    expect(footer().textContent).not.toContain('Freed');

    await fireEvent.click(button('Undo'));

    await waitFor(() => expect(footer().textContent).toContain('Not put back:'));
    expect(footer().textContent).toContain('(already exists)');
    expect(context.api.undoCleanup).toHaveBeenCalledWith(77);
  });

  it('says what went wrong when the cleanup failed', async () => {
    const context = await fixture(safePlan);

    show(context);
    await fireEvent.click(button('Clean selected (2)'));
    await waitFor(() => expect(context.api.startCleanup).toHaveBeenCalled());
    context.send({
      type: 'failed',
      message: 'the disk is full',
      outdatedScan: context.scanId,
      canUndo: false,
      cleanupId: 77,
    });

    await waitFor(() => expect(footer().textContent).toContain('Cleaning failed: the disk is full.'));
  });

  it('returns to idle with "Not cleaned." when the backend says the user declined its own dialog', async () => {
    const context = await fixture(safePlan);

    context.api.startCleanup = vi.fn().mockRejectedValue({ code: 'cancelled', message: 'declined' });
    show(context);
    await fireEvent.click(button('Clean selected (2)'));

    await waitFor(() => expect(footer().textContent).toContain('Not cleaned.'));
    expect(button('Clean selected (2)').disabled).toBe(false);
    expect(context.api.startCleanup).toHaveBeenCalledOnce();
  });

  it('disables the clean buttons with one sentence while the scan is outdated', async () => {
    const context = await fixture(safePlan);

    show(context);
    context.scan.markOutdated(context.scanId);

    await waitFor(() => expect(footer().textContent).toContain('The disk changed. Rescan to clean again.'));
    expect(button('Clean safe · 105 GB').disabled).toBe(true);
    expect(button('Clean selected (2)').disabled).toBe(true);
    expect(checkbox('Title of target').disabled).toBe(true);
    expect(button('Rescan')).toBeTruthy();
  });

  it('greys out the group sums and the row sizes once the scan is outdated, with a tooltip, and nothing else', async () => {
    const context = await fixture(safePlan);
    const note = 'From the scan before the cleanup — rescan to update';

    show(context);

    const sizes = (): HTMLElement[] => [
      screen.getByText('2 items · 105 GB'),
      screen.getByText('88 GB'),
      screen.getByText('268 GB', { selector: 'b' }),
    ];

    for (const element of sizes()) {
      expect(element.classList.contains('stale')).toBe(false);
      expect(element.getAttribute('title')).toBeNull();
    }

    context.scan.markOutdated(context.scanId);

    await waitFor(() => expect(sizes()[0]?.classList.contains('stale')).toBe(true));

    for (const element of sizes()) {
      expect(element.classList.contains('stale')).toBe(true);
      expect(element.getAttribute('title')).toBe(note);
    }

    for (const text of ['Title of target', '~/target', 'Reason of target.']) {
      expect(screen.getByText(text).closest('.stale')).toBeNull();
    }
  });

  it('does not grey a size twice when its row is already shown as cleaned', async () => {
    const context = await fixture(safePlan);

    show(context);
    await fireEvent.click(button('Clean selected (2)'));
    await waitFor(() => expect(context.api.startCleanup).toHaveBeenCalled());
    context.send({ type: 'groupStarted', ruleId: 'target', targets: 1 });
    context.send({
      type: 'targetDone',
      ruleId: 'target',
      path: '/home/demo/target',
      allocated: 1,
      outcome: { kind: 'removed', problems: 0 },
    });
    context.send({ type: 'groupFinished', ruleId: 'target' });
    context.send({
      type: 'finished',
      cleanupId: 77,
      outdatedScan: context.scanId,
      cleaned: 1,
      withProblems: 0,
      freed: 1,
      cancelled: false,
      toolsNotRun: [],
      canUndo: false,
    });

    await waitFor(() =>
      expect(screen.getByText('Title of target').closest('.sug')?.classList).toContain('done'),
    );

    const done = screen.getByText('88 GB');

    expect(done.classList.contains('stale')).toBe(false);
    expect(screen.getByText('268 GB', { selector: 'b' }).classList.contains('stale')).toBe(true);
  });

  it('shows the refusals and the rules that matched nothing as plain text, not hidden', async () => {
    const context = await fixture((ids) =>
      planOf([group(ids[0] as string, 'safe', 1)], {
        refused: [{ ruleId: 'trash', path: '/home/demo/.local/share/Trash', reason: 'it is a link' }],
        refusedMore: 2,
        notFound: ['snapshots'],
      }),
    );

    show(context);
    await fireEvent.click(button('Clean selected (2)'));
    await waitFor(() => expect(context.api.startCleanup).toHaveBeenCalled());

    expect(footer().textContent).toContain('Not cleaned: /home/demo/.local/share/Trash (it is a link)');
    expect(footer().textContent).toContain('… and 2 more not cleaned');
    expect(footer().textContent).toContain('Found nothing for: Title of snapshots');
  });
});

describe('AdvisorCard and backend text', () => {
  beforeEach(() => {
    settings.reduceMotion = true;
  });

  afterEach(() => {
    settings.reduceMotion = false;
  });

  it('shows markup in a title or a reason as text, and makes hidden characters visible', async () => {
    const context = await fixture(safePlan);
    const tricky = {
      ...suggestion('target', 'safe', 1, [11]),
      title: '<img src=x onerror=alert(1)>',
      path: '~/\u202Efdp.exe\n',
      reason: '<b>bold</b> **not bold** https://example.com',
    };

    show(context, {
      suggestions: { suggestions: [tricky], totals: { safe: GB, review: 0, keep: 0 } },
    });

    expect(document.querySelector('img')).toBeNull();
    expect(document.querySelector('.why b')).toBeNull();
    expect(document.querySelector('.why a')).toBeNull();
    expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeTruthy();
    expect(screen.getByText('<b>bold</b> **not bold** https://example.com')).toBeTruthy();
    expect(document.querySelector('.p')?.textContent).toBe('~/\\u{202E}fdp.exe\\u{A}');
  });
});

describe('AdvisorCard and hard links', () => {
  it('adds the hard-link note to the row of a suggestion with shared files', async () => {
    const context = await fixture(safePlan);
    const shared = { ...suggestion('target', 'safe', 88, [11]), sharedFiles: 3 };

    show(context, { suggestions: { suggestions: [shared], totals: { safe: 88 * GB, review: 0, keep: 0 } } });

    expect(
      screen.getByText(
        'Reason of target. 3 files are also linked from elsewhere, so the space freed can differ.',
      ),
    ).toBeTruthy();
  });
});
