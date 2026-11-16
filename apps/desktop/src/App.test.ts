import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { scan } from './lib/state/scan.svelte';
import { settings } from './lib/state/settings.svelte';
import { ui } from './lib/state/ui.svelte';

// A fast demo disk, so a whole scan takes a fraction of a second.
vi.mock('./lib/api', async () => {
  const { createMockApi } = await import('./lib/api/mock');

  return { api: createMockApi({ durationMs: 60, tickMs: 10, now: () => 1_750_000_000 }) };
});

const main = (): HTMLElement => screen.getByRole('main');
const nav = (name: string): HTMLElement =>
  within(screen.getByRole('navigation', { name: 'Main' })).getByRole('button', { name });

async function startScan(): Promise<void> {
  await fireEvent.click(await screen.findByRole('button', { name: /^\/\s/ }));
  await fireEvent.click(within(main()).getByRole('button', { name: 'Scan' }));
}

describe('the app', () => {
  beforeEach(() => {
    scan.reset();
    ui.go('scan');
    settings.set('units', 'GB');
    settings.set('sidebarCollapsed', false);
  });

  it('starts on the scan chooser with the volumes, and the folder picker adds a folder', async () => {
    render(App);

    expect(await screen.findByRole('button', { name: /nvme0n1p2/ })).toBeTruthy();
    const targets = screen.getAllByRole('button').filter((button) => button.hasAttribute('aria-pressed'));
    expect(targets).toHaveLength(3);
    expect(targets[0]?.getAttribute('aria-pressed')).toBe('true');

    await fireEvent.click(screen.getByRole('button', { name: /Choose a folder/ }));

    const folder = await screen.findByRole('button', { name: /\/home\/demo/ });
    expect(folder.getAttribute('aria-pressed')).toBe('true');
  });

  it('scans, shows progress with a cancel button, then the overview with real numbers', async () => {
    render(App);
    await startScan();

    expect(await screen.findByText('files read')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeTruthy();

    expect(await screen.findByText(/Scan complete/, undefined, { timeout: 3000 })).toBeTruthy();
    expect(screen.getByText('STORAGE OVERVIEW')).toBeTruthy();

    // Capacity of the whole volume, file types, age buckets and largest folders come from the API.
    expect(await screen.findByText('USED', undefined, { timeout: 3000 })).toBeTruthy();
    expect(screen.getByText('By file type')).toBeTruthy();
    expect(screen.getByText('Space by last modified')).toBeTruthy();
    expect(screen.getByText('Largest folders')).toBeTruthy();
    expect(screen.getByText('What is on the disk')).toBeTruthy();
    // The cards are there with the cleaning controls, nothing is cleaned until a click, and no update notice shows.
    expect(await screen.findByText('Reclaimable')).toBeTruthy();
    expect(await screen.findByText('Pacman package cache', undefined, { timeout: 3000 })).toBeTruthy();
    expect(screen.getByRole('heading', { name: /^Advisor/ })).toBeTruthy();
    expect(screen.queryByText('Update available')).toBeNull();
    expect(screen.getByRole('button', { name: /^Clean safe/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: /^Clean selected/ })).toBeTruthy();
    expect(screen.getAllByRole('checkbox').length).toBeGreaterThan(0);
    expect(document.querySelector('dialog[open]')).toBeNull();
  });

  it('cancels a scan and returns to the chooser with a note', async () => {
    render(App);
    await startScan();

    await fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }));

    expect(await screen.findByText(/last scan was cancelled/)).toBeTruthy();
    expect(within(main()).getByRole('button', { name: 'Scan' })).toBeTruthy();
  });

  it('opens the Explorer on a clicked folder, and goes back to the start screen with "Scan"', async () => {
    render(App);
    await startScan();

    await fireEvent.click(
      await screen.findByRole('button', { name: /^\/home\/demo/, hidden: false }, { timeout: 3000 }),
    );

    expect(ui.page).toBe('explorer');
    expect(ui.explorerFocus).toBeGreaterThan(0);
    expect(await screen.findByText('EXPLORER')).toBeTruthy();

    await fireEvent.click(nav('Overview'));
    await fireEvent.click(nav('Scan'));

    expect(await screen.findByText('NEW SCAN')).toBeTruthy();
  });

  it('returns to the folder the Explorer last showed, and to the scan root after a whole-disk click', async () => {
    // jsdom has no layout: the map is told it is 800 x 520.
    vi.stubGlobal(
      'ResizeObserver',
      class {
        constructor(private readonly callback: ResizeObserverCallback) {}

        observe(target: Element): void {
          this.callback(
            [{ target, contentRect: { width: 800, height: 520 } } as ResizeObserverEntry],
            this as unknown as ResizeObserver,
          );
        }

        unobserve(): void {}

        disconnect(): void {}
      },
    );
    render(App);
    await startScan();

    await fireEvent.click(
      await screen.findByRole('button', { name: /^\/home\/demo/, hidden: false }, { timeout: 3000 }),
    );
    expect(await screen.findByText('EXPLORER')).toBeTruthy();

    const path = (): string[] =>
      within(screen.getByRole('navigation', { name: 'Path' }))
        .getAllByRole('button')
        .map((crumb) => crumb.textContent?.trim() ?? '');

    await waitFor(() => expect(path().length).toBeGreaterThan(1));
    await fireEvent.click(await screen.findByRole('button', { name: 'Whole disk' }));
    await waitFor(() => expect(path()).toEqual(['/']));

    await fireEvent.click(nav('Overview'));
    await fireEvent.click(nav('Explorer'));

    expect(await screen.findByText('EXPLORER')).toBeTruthy();
    await waitFor(() => expect(path()).toEqual(['/']));
    expect((screen.getByRole('button', { name: 'Whole disk' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('forgets the Explorer place when the scan is dropped', async () => {
    render(App);
    await startScan();
    await screen.findByText(/Scan complete/, undefined, { timeout: 3000 });

    ui.openInExplorer(1, '#123456');
    expect(ui.explorerScan).toBeDefined();

    scan.reset();

    await waitFor(() => expect(ui.explorerFocus).toBeUndefined());
    expect(ui.explorerFocusColor).toBeUndefined();
    expect(ui.explorerScan).toBeUndefined();
  });

  it('offers Overview and Explorer only after a scan: dimmed, not clickable and out of the tab order before', async () => {
    render(App);

    for (const name of ['Overview', 'Explorer']) {
      const item = nav(name);

      expect(item.getAttribute('aria-disabled')).toBe('true');
      expect(item.getAttribute('tabindex')).toBe('-1');
      expect(item.getAttribute('title')).toBe('Scan a disk or folder first');

      await fireEvent.click(item);
      expect(ui.page).toBe('scan');
    }

    expect(nav('Scan').getAttribute('aria-current')).toBe('page');
    expect(nav('Scan').getAttribute('aria-disabled')).toBeNull();
    expect(screen.getByText('NEW SCAN')).toBeTruthy();

    await startScan();
    await screen.findByText(/Scan complete/, undefined, { timeout: 3000 });

    for (const name of ['Overview', 'Explorer']) {
      expect(nav(name).getAttribute('aria-disabled')).toBeNull();
      expect(nav(name).getAttribute('tabindex')).toBeNull();
    }

    // The app went to the Overview on its own.
    expect(nav('Overview').getAttribute('aria-current')).toBe('page');

    await fireEvent.click(nav('Explorer'));
    expect(await screen.findByText('EXPLORER')).toBeTruthy();
  });

  it('shows the progress under "Scan" while a scan runs', async () => {
    render(App);
    await startScan();

    expect(await screen.findByText('files read')).toBeTruthy();
    expect(nav('Scan').getAttribute('aria-current')).toBe('page');
    expect(nav('Overview').getAttribute('aria-disabled')).toBe('true');

    // Let it finish: a running scan would leak into the next test.
    await screen.findByText(/Scan complete/, undefined, { timeout: 3000 });
  });

  it('keeps the last result while "Scan" shows the start screen, until a new scan starts', async () => {
    render(App);
    await startScan();
    await screen.findByText(/Scan complete/, undefined, { timeout: 3000 });
    const finished = scan.state;

    await fireEvent.click(nav('Scan'));

    expect(await screen.findByText('NEW SCAN')).toBeTruthy();
    expect(scan.state).toBe(finished);
    expect(nav('Scan').getAttribute('aria-current')).toBe('page');
    expect(nav('Overview').getAttribute('aria-disabled')).toBeNull();
    expect(nav('Explorer').getAttribute('aria-disabled')).toBeNull();

    await fireEvent.click(nav('Overview'));
    expect(await screen.findByText('STORAGE OVERVIEW')).toBeTruthy();
    expect(scan.state).toBe(finished);

    // Starting a new scan from the start screen drops it, and Overview waits for the new one.
    await fireEvent.click(nav('Scan'));
    await screen.findByText('NEW SCAN');
    await fireEvent.click(within(main()).getByRole('button', { name: 'Scan' }));
    expect(await screen.findByText('files read')).toBeTruthy();
    expect(nav('Overview').getAttribute('aria-disabled')).toBe('true');

    expect(await screen.findByText('STORAGE OVERVIEW', undefined, { timeout: 3000 })).toBeTruthy();
    expect(nav('Overview').getAttribute('aria-current')).toBe('page');
  });

  it('changes every size when the unit setting changes', async () => {
    render(App);
    await startScan();
    await screen.findByText('USED', undefined, { timeout: 3000 });

    expect(screen.getAllByText(/GB/).length).toBeGreaterThan(0);
    // A rule's reason is the backend's own text and says "1 GiB" whatever the unit setting is.
    expect(screen.queryAllByText(/GiB/).filter((element) => !element.classList.contains('why'))).toHaveLength(
      0,
    );

    await fireEvent.click(nav('Settings'));
    await fireEvent.click(screen.getByRole('button', { name: 'GiB' }));

    expect(settings.units).toBe('GiB');
    expect(screen.getByText('Saved')).toBeTruthy();

    await fireEvent.click(nav('Overview'));
    await waitFor(() => expect(screen.getAllByText(/GiB|TiB/).length).toBeGreaterThan(0));
  });

  it('collapses the sidebar to the rail and keeps the labels for screen readers', async () => {
    render(App);

    await fireEvent.click(screen.getByRole('button', { name: 'Collapse menu' }));

    expect(settings.sidebarCollapsed).toBe(true);
    expect(nav('Scan').getAttribute('title')).toBe('Scan');
    expect(nav('Settings').getAttribute('title')).toBe('Settings');
    expect(nav('Overview').getAttribute('title')).toBe('Overview: Scan a disk or folder first');
    expect(screen.getByRole('button', { name: 'Expand menu' })).toBeTruthy();
  });

  it('has an Advisor item between Cleanup and Settings that waits for a scan, and no History item', async () => {
    render(App);

    const labels = within(screen.getByRole('navigation', { name: 'Main' }))
      .getAllByRole('button')
      .map((button) => button.textContent?.trim());

    expect(labels).toEqual(['Scan', 'Overview', 'Explorer', 'Cleanup', 'Advisor', 'Settings']);
    // Like Overview, Explorer and Cleanup: nothing to show before a scan (owner decision).
    expect(nav('Advisor').getAttribute('aria-disabled')).toBe('true');
    expect(nav('Advisor').getAttribute('aria-disabled')).toBe(nav('Overview').getAttribute('aria-disabled'));
    expect(screen.queryByText('ADVISOR')).toBeNull();
  });

  it('opens the Advisor view once a scan is ready, and disables it again while a new scan runs', async () => {
    render(App);
    await startScan();

    expect(nav('Advisor').getAttribute('aria-disabled')).toBe('true');

    await waitFor(() => expect(scan.state.status).toBe('ready'), { timeout: 3000 });
    await waitFor(() => expect(nav('Advisor').getAttribute('aria-disabled')).toBeNull());
    await fireEvent.click(nav('Advisor'));

    expect(await screen.findByText('ADVISOR')).toBeTruthy();
    expect(nav('Advisor').getAttribute('aria-current')).toBe('page');
    expect(within(main()).queryByText('Scan a disk or folder first.')).toBeNull();

    await fireEvent.click(nav('Scan'));
    await startScan();

    expect(nav('Advisor').getAttribute('aria-disabled')).toBe('true');
  });
});
