import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../../../App.svelte';
import { api } from '../../api';
import { advisor } from '../../state/advisor.svelte';
import { cleanup } from '../../state/cleanup.svelte';
import { scan } from '../../state/scan.svelte';
import { settings } from '../../state/settings.svelte';
import { ui } from '../../state/ui.svelte';
import { stubChartBox } from './testFixture';

// A fast demo disk, so a whole scan takes a fraction of a second.
vi.mock('../../api', async () => {
  const { createMockApi } = await import('../../api/mock');

  return { api: createMockApi({ durationMs: 60, tickMs: 10, now: () => 1_750_000_000 }) };
});

const main = (): HTMLElement => screen.getByRole('main');
const nav = (name: string): HTMLElement =>
  within(screen.getByRole('navigation', { name: 'Main' })).getByRole('button', { name });
const steps = (): HTMLElement => within(main()).getByRole('list', { name: 'Steps' });
const currentStep = (): string | null =>
  steps().querySelector('[aria-current="step"] > span')?.textContent ?? null;

async function scanDisk(): Promise<void> {
  await fireEvent.click(await screen.findByRole('button', { name: /^\/\s/ }));
  await fireEvent.click(within(main()).getByRole('button', { name: 'Scan' }));
  await screen.findByText(/Scan complete/, undefined, { timeout: 3000 });
}

async function openCleanup(): Promise<void> {
  await scanDisk();
  await fireEvent.click(nav('Cleanup'));
  await screen.findByText('CLEANUP');
}

describe('the Cleanup view', () => {
  let restore = (): void => {};

  beforeEach(() => {
    restore = stubChartBox();
    scan.reset();
    ui.go('scan');
    settings.set('units', 'GB');
    settings.set('sidebarCollapsed', false);
    settings.reduceMotion = true;
  });

  afterEach(() => {
    settings.reduceMotion = false;
    restore();
  });

  it('is in the sidebar between Explorer and Advisor, and off until a scan is ready', async () => {
    render(App);

    const labels = within(screen.getByRole('navigation', { name: 'Main' }))
      .getAllByRole('button')
      .map((button) => button.textContent?.trim());

    expect(labels).toEqual(['Scan', 'Overview', 'Explorer', 'Cleanup', 'Advisor', 'History', 'Settings']);
    expect(nav('Cleanup').getAttribute('aria-disabled')).toBe('true');
    expect(nav('Cleanup').getAttribute('title')).toBe('Scan a disk or folder first');

    await fireEvent.click(nav('Cleanup'));
    expect(ui.page).toBe('scan');

    await scanDisk();

    expect(nav('Cleanup').getAttribute('aria-disabled')).toBeNull();
  });

  it('opens on the Review step with the hero numbers, the chart and the Advisor list', async () => {
    render(App);
    await openCleanup();

    expect(nav('Cleanup').getAttribute('aria-current')).toBe('page');
    expect(currentStep()).toBe('Review');
    expect(await screen.findByText('Selected to free')).toBeTruthy();
    expect(screen.getByText('Disk after cleanup')).toBeTruthy();
    expect(screen.getByRole('group', { name: 'Cleanup chart' })).toBeTruthy();
    expect(await screen.findByRole('heading', { name: /^Advisor/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: /^Clean safe/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: /^Clean selected/ })).toBeTruthy();
  });

  it('says LOCAL MODEL on its Advisor card only while the model is loaded', async () => {
    render(App);
    await openCleanup();

    const tag = async (): Promise<string> =>
      (await screen.findByRole('heading', { name: /^Advisor/ })).textContent ?? '';

    expect(await tag()).toContain('RULES');

    advisor.status = {
      state: 'loaded',
      model: {
        name: 'Qwen3.5-4B',
        format: 'GGUF · Q4_K_M',
        bytes: 1,
        host: 'huggingface.co',
      },
      options: { enabled: true, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true },
      ramTotal: 1,
      gpuSupported: false,
      os: 'linux',
    };

    await waitFor(async () => expect(await tag()).toContain('LOCAL MODEL'));

    advisor.status = undefined;
  });

  it('fetches the suggestions again when the advisor changes them, but not while a cleanup is planned or runs', async () => {
    render(App);
    await openCleanup();
    await screen.findAllByText('Pacman package cache', undefined, { timeout: 3000 });

    const fetched = vi.spyOn(api, 'getSuggestions');

    advisor.suggestionsVersion++;
    await waitFor(() => expect(fetched).toHaveBeenCalledTimes(1));

    cleanup.phase = { kind: 'planning' };
    advisor.suggestionsVersion++;
    await new Promise((resolve) => setTimeout(resolve, 50));

    expect(fetched).toHaveBeenCalledTimes(1);

    cleanup.phase = { kind: 'idle' };
    await waitFor(() => expect(fetched).toHaveBeenCalledTimes(2));
  });

  it('shares its selection with the list: "All" in the list ticks every group of the chart', async () => {
    render(App);
    await openCleanup();
    await screen.findAllByText('Pacman package cache', undefined, { timeout: 3000 });

    const chart = screen.getByRole('group', { name: 'Cleanup chart' });
    const ticked = (): (string | null)[] =>
      within(chart)
        .getAllByRole('checkbox')
        .map((box) => box.getAttribute('aria-checked'));

    expect(ticked()).toContain('false');

    await fireEvent.click(within(main()).getByRole('button', { name: 'All' }));

    expect(ticked().every((value) => value === 'true')).toBe(true);

    await fireEvent.click(within(chart).getAllByRole('checkbox')[0] as HTMLElement);

    expect(cleanup.choice).toBe('custom');
    expect(within(main()).getByRole('button', { name: 'All' }).getAttribute('aria-pressed')).toBe('false');
  });

  it('cleans the safe items, goes to the Clean step and shows the result with Scan again', async () => {
    render(App);
    await openCleanup();

    await fireEvent.click(await screen.findByRole('button', { name: /^Clean safe ·/ }, { timeout: 3000 }));

    expect(await screen.findByRole('heading', { name: 'Result' }, { timeout: 3000 })).toBeTruthy();
    expect(steps().querySelectorAll('.dot svg')).toHaveLength(3);
    expect(within(main()).getByText('freed')).toBeTruthy();
    expect(within(main()).getByRole('button', { name: 'Scan again' })).toBeTruthy();
    expect(within(main()).getByText('Before')).toBeTruthy();
    expect(within(main()).getByText('After')).toBeTruthy();
  });

  it('"Scan again" shows the Scan step while it runs, and comes back to Review of the new scan', async () => {
    render(App);
    await openCleanup();
    await fireEvent.click(await screen.findByRole('button', { name: /^Clean safe ·/ }, { timeout: 3000 }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Scan again' }, { timeout: 3000 }));

    expect(await screen.findByText('files read')).toBeTruthy();
    expect(currentStep()).toBe('Scan');
    expect(within(main()).getByText('What happens')).toBeTruthy();
    expect(nav('Cleanup').getAttribute('aria-current')).toBe('page');

    await waitFor(() => expect(currentStep()).toBe('Review'), { timeout: 3000 });
    expect(ui.page).toBe('cleanup');
    expect(await screen.findByRole('button', { name: /^Clean safe ·/ }, { timeout: 3000 })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: 'Result' })).toBeNull();
    expect(cleanup.phase.kind).toBe('idle');
  });

  it('shows a cleanup that was started in the Overview', async () => {
    render(App);
    await scanDisk();

    await fireEvent.click(await screen.findByRole('button', { name: /^Clean safe ·/ }, { timeout: 3000 }));
    await waitFor(() => expect(cleanup.phase.kind).toBe('done'), { timeout: 3000 });
    await fireEvent.click(nav('Cleanup'));

    expect(await screen.findByRole('heading', { name: 'Result' })).toBeTruthy();
    expect(steps().querySelectorAll('.dot svg')).toHaveLength(3);
  });

  it('greys the figures of the old scan after a cleanup, in the Overview and here, until a rescan', async () => {
    render(App);
    await scanDisk();

    const stale = (selector: string): boolean[] =>
      [...main().querySelectorAll(selector)].map((element) => element.classList.contains('stale'));

    // The Overview: the Reclaimable card and the Advisor's sums, current until something is cleaned.
    await screen.findAllByText('Pacman package cache', undefined, { timeout: 3000 });
    expect(stale('.rec, .recr, .gsum')).not.toContain(true);

    await fireEvent.click(await screen.findByRole('button', { name: /^Clean safe ·/ }, { timeout: 3000 }));
    await waitFor(() => expect(cleanup.phase.kind).toBe('done'), { timeout: 3000 });

    expect(stale('.rec')).toEqual([true]);
    expect(stale('.recr').filter(Boolean).length).toBeGreaterThanOrEqual(3);
    expect(screen.getByText('Already freed').closest('.recr')?.classList.contains('stale')).toBe(false);
    expect(stale('.gsum').every(Boolean)).toBe(true);

    // The Cleanup view: the chart and the disk after the cleanup.
    await fireEvent.click(nav('Cleanup'));
    await screen.findByRole('heading', { name: 'Result' });

    expect(screen.getByRole('group', { name: 'Cleanup chart' }).classList.contains('stale')).toBe(true);
    expect(screen.getByText('Disk after cleanup').parentElement?.classList.contains('stale')).toBe(true);

    // A new scan describes the disk again.
    await fireEvent.click(within(main()).getByRole('button', { name: 'Scan again' }));
    await waitFor(() => expect(currentStep()).toBe('Review'), { timeout: 3000 });
    await screen.findByText('Selected to free');

    expect(screen.getByRole('group', { name: 'Cleanup chart' }).classList.contains('stale')).toBe(false);

    await fireEvent.click(nav('Overview'));
    await screen.findAllByText('Pacman package cache', undefined, { timeout: 3000 });

    expect(stale('.rec, .recr, .gsum')).not.toContain(true);
  });

  it('shows its Scan step, without the list, while the scan runs', async () => {
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: /^\/\s/ }));
    await fireEvent.click(within(main()).getByRole('button', { name: 'Scan' }));
    ui.go('cleanup');

    expect(await screen.findByText('files read')).toBeTruthy();
    expect(currentStep()).toBe('Scan');
    // The page on screen is not dimmed, though the other pages that need a scan are.
    expect(nav('Cleanup').getAttribute('aria-disabled')).toBeNull();
    expect(nav('Cleanup').getAttribute('aria-current')).toBe('page');
    expect(nav('Explorer').getAttribute('aria-disabled')).toBe('true');
    expect(within(main()).getByRole('button', { name: 'Cancel' })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: /^Advisor/ })).toBeNull();

    await screen.findByText(/Selected to free/, undefined, { timeout: 3000 });
    expect(currentStep()).toBe('Review');
  });
});
