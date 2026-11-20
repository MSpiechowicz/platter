import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { NodeId } from '../../api/types';
import { cleanup } from '../../state/cleanup.svelte';
import { scan } from '../../state/scan.svelte';
import { settings } from '../../state/settings.svelte';
import { ui } from '../../state/ui.svelte';
import Explorer from './Explorer.svelte';

// The demo disk, fast, with the one call that opens a file manager left alone.
vi.mock('../../api', async () => {
  const { createMockApi } = await import('../../api/mock');
  const api = createMockApi({ durationMs: 0, now: () => 1_750_000_000 });

  return { api: { ...api, openInFileManager: vi.fn() } };
});

// jsdom has no layout: the map is told it is 800 x 520.
class FakeResizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}

  observe(target: Element): void {
    this.callback([{ target, contentRect: { width: 800, height: 520 } } as ResizeObserverEntry], this);
  }

  unobserve(): void {}

  disconnect(): void {}
}

async function startScan(root = '/'): Promise<void> {
  await scan.loadVolumes();
  await scan.start({ root, oneFileSystem: true });
  await waitFor(() => expect(scan.state.status).toBe('ready'));
}

const scanId = (): number => (scan.state.status === 'ready' ? scan.state.scanId : 0);

/** The node at a path of the demo disk. */
async function nodeAt(path: string): Promise<NodeId> {
  const { api } = await import('../../api');
  let at = 0;

  for (const name of path.split('/').filter(Boolean)) {
    const page = await api.getChildren(scanId(), at, 0, 50, 'nameAsc', name);

    at = (page.items.find((item) => item.name === name) as { id: NodeId }).id;
  }

  return at;
}

/** Opens the Explorer on `/home/demo`, where the personal folders are kept and the rest can be marked. */
async function openDemo(): Promise<void> {
  render(Explorer, { props: { focusId: await nodeAt('/home/demo') } });
  await screen.findByRole('button', { name: /^Downloads \d/ });
  // The suggestions arrive on their own; the kept rows show once they have.
  await waitFor(() => expect(cleanup.keptNodes.size).toBeGreaterThan(0));
}

const box = (name: string): HTMLInputElement =>
  screen.getByRole('checkbox', { name: `Mark ${name} for cleanup` }) as HTMLInputElement;
const rows = (): HTMLElement => document.querySelector('.child-list .rows') as HTMLElement;
const row = (name: string): HTMLElement =>
  within(rows()).getByRole('button', { name: new RegExp(`^${name.replace('.', '\\.')}( Kept)? \\d`) });
const footer = (): HTMLElement => document.querySelector('.child-list .total') as HTMLElement;
const markButton = (): HTMLButtonElement =>
  screen.getByRole('button', { name: /^(Mark for cleanup|Unmark)$/ }) as HTMLButtonElement;

describe('marking in the Explorer', () => {
  beforeEach(async () => {
    vi.stubGlobal('ResizeObserver', FakeResizeObserver);
    scan.reset();
    settings.explorerView = 'flat';
    ui.forgetExplorer();
    await startScan();
  });

  describe('the list', () => {
    it('has a checkbox per row, and a lock with "KEPT" for the items a keep rule covers', async () => {
      await openDemo();

      for (const kept of ['Videos', 'Pictures', 'Music', 'Documents']) {
        const item = row(kept).closest('.item') as HTMLElement;

        expect(within(item).queryByRole('checkbox')).toBeNull();
        expect(within(item).getByRole('img', { name: /Kept on purpose/ })).toBeTruthy();
        expect(item.querySelector('.kept')?.textContent).toBe('Kept');
      }

      for (const free of ['Downloads', '.cache', 'Projekty']) {
        expect(box(free).disabled).toBe(false);
        expect((row(free).closest('.item') as HTMLElement).querySelector('.kept')).toBeNull();
      }
    });

    it('marks by the checkbox without selecting the row, and unmarks again', async () => {
      await openDemo();

      await fireEvent.click(box('Downloads'));

      expect(box('Downloads').checked).toBe(true);
      expect(row('Downloads').getAttribute('aria-pressed')).toBe('false');
      expect(footer().textContent).toContain('1 item');
      expect(footer().textContent).toContain('Clean (1)');

      await fireEvent.click(box('Downloads'));

      expect(box('Downloads').checked).toBe(false);
      expect(footer().textContent).toContain('Tick folders in the list.');
    });

    it('keeps the checkbox column, and the row, where they were when something is marked', async () => {
      await openDemo();

      const before = [...document.querySelectorAll('.rows .item')].map((item) => item.className);

      await fireEvent.click(box('Downloads'));

      expect([...document.querySelectorAll('.rows .item')].map((item) => item.className)).toEqual(before);
      expect(document.querySelectorAll('.rows .item .box, .rows .item .lock')).toHaveLength(before.length);
    });

    it('does not count a mark inside another mark twice', async () => {
      await openDemo();

      await fireEvent.click(box('Projekty'));
      expect(footer().textContent).toContain('1 item');
      const { api } = await import('../../api');
      const projekty = await api.getNode(scanId(), await nodeAt('/home/demo/Projekty'));
      const total = footer().querySelector('b')?.textContent;

      // A folder inside it: open Projekty, mark one of its folders.
      await fireEvent.click(row('Projekty'));
      await fireEvent.click(row('Projekty'));
      await screen.findByRole('button', { name: /^web-shop \d/ });
      await fireEvent.click(box('web-shop'));

      expect(cleanup.marks.size).toBe(2);
      expect(footer().textContent).toContain('1 item · 1 inside another');
      expect(footer().querySelector('b')?.textContent).toBe(total);
      expect(cleanup.markedBytes).toBe(projekty.allocated);
      expect(screen.getByText('This folder is inside one that is marked for cleanup')).toBeTruthy();
    });
  });

  describe('the details card', () => {
    it('has "Mark for cleanup" beside "Go into folder", always, and the file manager button', async () => {
      await openDemo();

      expect(screen.getByRole('button', { name: 'Go into folder' })).toBeTruthy();
      expect(markButton().textContent?.trim()).toBe('Mark for cleanup');
      expect(screen.getByRole('button', { name: 'Show in file manager' })).toBeTruthy();
    });

    it('marks the open folder when nothing is selected', async () => {
      await openDemo();

      // /home/demo is not covered by a keep rule itself (it only holds kept folders), so the button is on.
      expect(screen.getByRole('heading', { name: 'Current folder' })).toBeTruthy();
      expect(markButton().title).toBe('');
      expect(markButton().disabled).toBe(false);

      await fireEvent.click(markButton());

      expect(markButton().textContent?.trim()).toBe('Unmark');
      expect(cleanup.isMarked(await nodeAt('/home/demo'))).toBe(true);
      expect(footer().textContent).toContain('Clean (1)');
    });

    it('marks and unmarks the selected folder', async () => {
      await openDemo();
      await fireEvent.click(row('Downloads'));

      expect(markButton().disabled).toBe(false);

      await fireEvent.click(markButton());

      expect(markButton().textContent?.trim()).toBe('Unmark');
      expect(box('Downloads').checked).toBe(true);
      expect(footer().textContent).toContain('Clean (1)');

      await fireEvent.click(markButton());

      expect(markButton().textContent?.trim()).toBe('Mark for cleanup');
      expect(box('Downloads').checked).toBe(false);
    });

    it('is off for a kept item, with the reason, and the card keeps its height', async () => {
      await openDemo();

      const heights = (): number[] =>
        [...document.querySelectorAll('.actions button')].map(
          (button) => (button as HTMLElement).offsetHeight,
        );
      const before = heights();

      await fireEvent.click(row('Videos'));

      expect(markButton().disabled).toBe(true);
      expect(markButton().title).toBe('Kept on purpose: Platter will not clean this.');
      expect(screen.getByRole('heading', { name: 'Selected' })).toBeTruthy();
      expect(heights()).toEqual(before);
    });

    it('is off at the scan root, with the reason, while both buttons stay', async () => {
      render(Explorer, { props: {} });
      await screen.findByRole('button', { name: /^home \d/ });

      expect(markButton().disabled).toBe(true);
      expect(markButton().title).toBe('The scanned folder itself is never cleaned.');
      expect(screen.getByRole('button', { name: 'Go into folder' })).toBeTruthy();
    });

    it('is off for a folder inside a kept one: nothing under Videos can be marked', async () => {
      render(Explorer, { props: { focusId: await nodeAt('/home/demo/Videos') } });
      await screen.findByRole('heading', { name: 'Current folder' });
      await waitFor(() => expect(cleanup.keptNodes.size).toBeGreaterThan(0));

      expect(markButton().disabled).toBe(true);
      expect(markButton().title).toBe('Kept on purpose: Platter will not clean this.');
      expect(rows().querySelectorAll('input[type="checkbox"]')).toHaveLength(0);
      expect(rows().querySelectorAll('.lock').length).toBeGreaterThan(0);
    });

    it('is off for everything once a cleanup has run: the scan is outdated', async () => {
      await openDemo();
      await fireEvent.click(box('Downloads'));
      await fireEvent.click(within(footer()).getByRole('button', { name: 'Clean (1)' }));
      await fireEvent.click(
        within(await screen.findByRole('dialog')).getByRole('button', { name: /^Clean \d/ }),
      );
      await waitFor(() => expect(cleanup.phase.kind).toBe('done'));
      // The page opens again over the outdated scan, on the same folder; its marks are still there.
      await waitFor(() => expect(markButton().title).toBe('Marking is paused until you rescan.'));

      expect(markButton().disabled).toBe(true);
      expect(box('Projekty').disabled).toBe(true);
      // A mark that was there stays visible, but cannot be changed.
      expect(box('Downloads').checked).toBe(true);
      expect(box('Downloads').disabled).toBe(true);
    });
  });

  describe('the footer', () => {
    it('plans what is marked, asks first, runs, shows the result, and undoes', async () => {
      await openDemo();
      await fireEvent.click(box('Downloads'));
      await fireEvent.click(box('.cache'));

      expect(footer().textContent).toContain('2 items');

      await fireEvent.click(within(footer()).getByRole('button', { name: 'Clean (2)' }));

      // Every marked item needs review: the dialog lists them before anything happens.
      const dialog = (await screen.findByRole('dialog')) as HTMLElement;

      expect(within(dialog).getByRole('heading', { name: 'Clean 2 items that need review?' })).toBeTruthy();
      expect(dialog.textContent).toContain('/home/demo/Downloads');
      expect(dialog.textContent).toContain('/home/demo/.cache');
      expect(dialog.textContent).toContain('Moved to the trash. This can be undone.');
      expect(cleanup.phase.kind).toBe('review');

      await fireEvent.click(within(dialog).getByRole('button', { name: /^Clean \d/ }));
      await waitFor(() => expect(cleanup.phase.kind).toBe('done'));

      const status = within(footer()).getByRole('status');

      expect(status.textContent).toContain('Moved to the trash: 2 items.');
      expect(status.textContent).toContain('This can be undone.');
      // What was freed is a toast, with no button of its own: Rescan is the outdated banner's.
      expect(ui.toasts.at(-1)).toMatchObject({ title: 'Cleanup finished', description: '' });
      expect(ui.toasts.at(-1)?.action).toBeUndefined();
      expect(within(footer()).getByRole('button', { name: 'Undo' })).toBeTruthy();
      expect(within(footer()).queryByRole('button', { name: 'Rescan' })).toBeNull();

      await fireEvent.click(within(footer()).getByRole('button', { name: 'Undo' }));

      await waitFor(() => expect(footer().textContent).toContain('Put back 2 items.'));
      expect(within(footer()).queryByRole('button', { name: 'Undo' })).toBeNull();
    });

    it('starts nothing when the review is cancelled, and keeps the marks and the button', async () => {
      await openDemo();
      await fireEvent.click(box('Downloads'));
      await fireEvent.click(within(footer()).getByRole('button', { name: 'Clean (1)' }));
      await fireEvent.click(
        within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }),
      );

      await waitFor(() => expect(document.querySelector('dialog[open]')).toBeNull());

      expect(cleanup.phase.kind).toBe('idle');
      expect(footer().textContent).toContain('Not cleaned.');
      expect(box('Downloads').checked).toBe(true);
      expect(
        (within(footer()).getByRole('button', { name: 'Clean (1)' }) as HTMLButtonElement).disabled,
      ).toBe(false);
    });

    it('says in a toast what the backend refused, and the footer keeps the marks', async () => {
      await openDemo();
      // `.local` holds the Steam library: no checkbox is kept for it, the backend says no.
      await fireEvent.click(box('.local'));
      await fireEvent.click(within(footer()).getByRole('button', { name: 'Clean (1)' }));

      await waitFor(() =>
        expect(ui.toasts.at(-1)).toMatchObject({
          title: 'Nothing could be cleaned',
          description: '1 item was left out. Why: kept on purpose.',
          kind: 'error',
        }),
      );
      expect(footer().textContent).not.toContain('Not cleaned:');
      expect(box('.local').checked).toBe(true);
      expect(document.querySelector('dialog[open]')).toBeNull();
    });
  });

  describe('across folders, views and scans', () => {
    it('keeps the marks when the user opens another folder and comes back', async () => {
      await openDemo();
      await fireEvent.click(box('Downloads'));

      await fireEvent.click(row('Projekty'));
      await fireEvent.click(row('Projekty'));
      await screen.findByRole('button', { name: /^web-shop \d/ });

      // In another folder the footer still has the total.
      expect(footer().textContent).toContain('1 item');

      await fireEvent.click(screen.getByRole('button', { name: /^Back to/ }));
      await screen.findByRole('button', { name: /^Downloads \d/ });

      expect(box('Downloads').checked).toBe(true);
    });

    it('keeps the marks when the Explorer is left and opened again on the same scan', async () => {
      await openDemo();
      await fireEvent.click(box('Downloads'));

      document.body.innerHTML = '';
      render(Explorer, { props: { focusId: await nodeAt('/home/demo') } });
      await screen.findByRole('button', { name: /^Downloads \d/ });

      expect(box('Downloads').checked).toBe(true);
      expect(footer().textContent).toContain('Clean (1)');
    });

    it('forgets them when a new scan is made', async () => {
      await openDemo();
      await fireEvent.click(box('Downloads'));
      expect(cleanup.marks.size).toBe(1);

      scan.reset();
      await startScan();
      document.body.innerHTML = '';
      render(Explorer, { props: { focusId: await nodeAt('/home/demo') } });
      await screen.findByRole('button', { name: /^Downloads \d/ });

      expect(cleanup.marks.size).toBe(0);
      expect(box('Downloads').checked).toBe(false);
      expect(footer().textContent).toContain('Tick folders in the list.');
    });
  });

  describe('the map', () => {
    const marked = (): string[] =>
      [...document.querySelectorAll('svg rect.marked')].map(
        (rect) => rect.closest('g.top')?.getAttribute('aria-label')?.split(', ')[0] ?? '',
      );

    it('draws a dashed outline on a marked folder and says so to a screen reader', async () => {
      await openDemo();

      expect(marked()).toEqual([]);

      await fireEvent.click(box('.cache'));

      expect(marked()).toEqual(['.cache']);
      expect(screen.getByRole('button', { name: /^\.cache, .*marked for cleanup$/ })).toBeTruthy();

      await fireEvent.click(box('.cache'));

      expect(marked()).toEqual([]);
    });

    it('draws a green outline and a SAFE tag on a safe folder, and says so to a screen reader', async () => {
      render(Explorer, { props: { focusId: await nodeAt('/home/demo/Projekty/rust-playground') } });
      await screen.findByRole('button', { name: /^target, / });
      await waitFor(() => expect(cleanup.safeNodes.size).toBeGreaterThan(0));

      const safe = [...document.querySelectorAll('svg rect.safe')];

      expect(safe).toHaveLength(1);
      expect(safe[0]?.closest('g.top')?.getAttribute('aria-label')).toMatch(/^target, .*safe to clean$/);
      expect(document.querySelector('svg tspan.tag')?.textContent).toBe('SAFE');
      expect(document.querySelector('svg text.sz')?.textContent).toMatch(/^SAFE · /);
    });

    it('outlines the safe folder itself, deep in the map, and not the folders around it', async () => {
      render(Explorer, { props: { focusId: await nodeAt('/home/demo/Projekty') } });
      await screen.findByRole('button', { name: /^web-shop, / });
      await waitFor(() => expect(cleanup.safeNodes.size).toBeGreaterThan(0));

      const outlined = [...document.querySelectorAll('svg rect.safe')];

      // The projects hold `node_modules` and `target` folders that a safe rule lists; the project tiles stay plain.
      expect(outlined.length).toBeGreaterThan(0);
      expect(outlined.every((rect) => !rect.classList.contains('first'))).toBe(true);
      expect(document.querySelectorAll('svg tspan.tag').length).toBeGreaterThan(0);
    });

    it('has the two new keys in the legend', async () => {
      await openDemo();

      expect(screen.getByText('Safe to clean')).toBeTruthy();
      expect(screen.getByText('Marked for cleanup', { selector: '.legend span' })).toBeTruthy();
      expect(screen.getByText('Selected', { selector: '.legend span' })).toBeTruthy();
    });
  });
});
