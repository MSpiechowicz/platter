import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { NodeDto } from '../../api/types';
import { fmt } from '../../state/fmt.svelte';
import { scan } from '../../state/scan.svelte';
import { settings } from '../../state/settings.svelte';
import { ui } from '../../state/ui.svelte';
import Explorer from './Explorer.svelte';

// A fast demo disk with a spy on the one call that has a side effect.
const { openInFileManager } = vi.hoisted(() => ({ openInFileManager: vi.fn() }));

vi.mock('../../api', async () => {
  const { createMockApi } = await import('../../api/mock');
  const api = createMockApi({ durationMs: 0, now: () => 1_750_000_000 });

  return { api: { ...api, openInFileManager } };
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

const map = (): HTMLElement => screen.getByLabelText('Map of the open folder');
const tile = (name: RegExp): HTMLElement => within(map()).getByRole('button', { name });
const crumbs = (): string[] =>
  within(screen.getByRole('navigation', { name: 'Path' }))
    .getAllByRole('button')
    .map((crumb) => crumb.textContent?.trim() ?? '');

describe('the Explorer', () => {
  beforeEach(async () => {
    vi.stubGlobal('ResizeObserver', FakeResizeObserver);
    openInFileManager.mockReset();
    scan.reset();
    settings.explorerView = 'flat';
    ui.forgetExplorer();
    await startScan();
  });

  it('opens on the scan root: path, map, details of the folder and the list', async () => {
    render(Explorer, { props: {} });

    expect(await screen.findByRole('heading', { name: 'EXPLORER' })).toBeTruthy();
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });

    expect(crumbs()).toEqual(['/']);
    expect(screen.getByRole('heading', { name: 'Current folder' })).toBeTruthy();
    expect(screen.getByText('Click a folder to select it, click again to open it')).toBeTruthy();
    expect(screen.getByText(/^Bigger tile = more space/)).toBeTruthy();
    expect((screen.getByRole('button', { name: /^Back/ }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'Whole disk' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('7 items')).toBeTruthy();
  });

  it('calls the button "Whole scan" when the scan root is a folder, not a whole volume', async () => {
    scan.reset();
    await startScan('/home/demo');
    render(Explorer, { props: {} });

    expect(await screen.findByRole('button', { name: 'Whole scan' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Whole disk' })).toBeNull();
  });

  it('names the first crumb of a folder scan by its whole path, and every crumb has its path as a tooltip', async () => {
    scan.reset();
    await startScan('/home/demo');
    render(Explorer, { props: {} });

    await within(await screen.findByLabelText('Map of the open folder')).findAllByRole('button');
    expect(crumbs()).toEqual(['/home/demo']);

    const first = (await within(map()).findAllByRole('button'))[0] as HTMLElement;
    const child = first.getAttribute('aria-label')?.split(', ')[0] as string;

    await fireEvent.click(first);
    await fireEvent.click(first);

    await waitFor(() => expect(crumbs()).toEqual(['/home/demo', child]));
    expect(screen.getByRole('button', { name: 'Back to demo' })).toBeTruthy();

    const titles = within(screen.getByRole('navigation', { name: 'Path' }))
      .getAllByRole('button')
      .map((crumb) => crumb.getAttribute('title'));
    expect(titles).toEqual(['/home/demo', `/home/demo/${child}`]);
  });

  it('selects on the first click and opens on the second, then goes back', async () => {
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });

    await fireEvent.click(tile(/^home \/ demo, /));

    expect(screen.getByRole('heading', { name: 'Selected' })).toBeTruthy();
    expect(tile(/^home \/ demo, /).getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByRole('button', { name: /^home \d/ }).getAttribute('aria-pressed')).toBe('true');
    expect(crumbs()).toEqual(['/']);
    expect((screen.getByRole('button', { name: 'Go into folder' }) as HTMLButtonElement).disabled).toBe(
      false,
    );

    await fireEvent.click(tile(/^home \/ demo, /));

    // The chain tile opens its last folder, and the crumbs show the whole way to it.
    await waitFor(() => expect(crumbs()).toEqual(['/', 'home', 'demo']));
    await within(map()).findByRole('button', { name: /^Projekty, / });
    expect(screen.getByRole('heading', { name: 'Current folder' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Back to home' })).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Whole disk' }));
    await waitFor(() => expect(crumbs()).toEqual(['/']));
  });

  it('clears the selection when the empty space is clicked', async () => {
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });

    await fireEvent.click(tile(/^home \/ demo, /));
    expect(screen.getByRole('heading', { name: 'Selected' })).toBeTruthy();

    await fireEvent.click(map());
    expect(screen.getByRole('heading', { name: 'Current folder' })).toBeTruthy();
  });

  it('keeps both buttons of the details card, enabled only when they apply', async () => {
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });

    const open = screen.getByRole('button', { name: 'Go into folder' }) as HTMLButtonElement;
    const reveal = screen.getByRole('button', { name: 'Show in file manager' }) as HTMLButtonElement;

    expect(open.disabled).toBe(true);
    expect(reveal.disabled).toBe(false);

    await fireEvent.click(reveal);
    expect(openInFileManager).toHaveBeenCalledWith(scan.state.status === 'ready' ? scan.state.scanId : -1, 0);

    await fireEvent.click(tile(/^home \/ demo, /));
    expect(open.disabled).toBe(false);

    await fireEvent.click(open);
    await waitFor(() => expect(crumbs()).toEqual(['/', 'home', 'demo']));
  });

  it('goes up on Backspace and Escape, but not while typing', async () => {
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });
    await fireEvent.click(tile(/^home \/ demo, /));
    await fireEvent.click(tile(/^home \/ demo, /));
    await waitFor(() => expect(crumbs()).toEqual(['/', 'home', 'demo']));
    await within(map()).findByRole('button', { name: /^Projekty, / });

    const field = document.body.appendChild(document.createElement('input'));

    await fireEvent.keyDown(field, { key: 'Backspace' });
    expect(crumbs()).toEqual(['/', 'home', 'demo']);
    field.remove();

    await fireEvent.keyDown(document.body, { key: 'Escape' });
    await waitFor(() => expect(crumbs()).toEqual(['/', 'home']));
  });

  it('sorts the list by name from the dropdown', async () => {
    render(Explorer, { props: {} });
    await screen.findByText('7 items');

    const names = () => [...document.querySelectorAll('.row .nm')].map((row) => row.textContent);

    expect(names()[0]).toBe('home');

    await fireEvent.click(screen.getByRole('button', { name: /Sort by: Size/ }));
    await fireEvent.click(screen.getByRole('option', { name: 'Name' }));

    await waitFor(() =>
      expect(names()).toEqual([...names()].sort((a, b) => (a ?? '').localeCompare(b ?? ''))),
    );
    expect(names()[0]).not.toBe('home');
  });

  it('links the list and the map: hovering a row lights its folder, clicking selects it', async () => {
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });

    const row = screen.getByRole('button', { name: /^var / });

    await fireEvent.mouseEnter(row);
    expect(tile(/^var, /).classList.contains('hot')).toBe(true);
    expect(tile(/^home \/ demo, /).classList.contains('hot')).toBe(false);

    await fireEvent.click(row);
    expect(tile(/^var, /).classList.contains('sel')).toBe(true);
    expect(screen.getByRole('heading', { name: 'Selected' })).toBeTruthy();
  });

  it('has the marking controls from the start, off where they do not apply, and nothing marked', async () => {
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });

    // The scan root is never marked; both buttons are there all the same, so the card never changes.
    expect((screen.getByRole('button', { name: 'Mark for cleanup' }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect((screen.getByRole('button', { name: 'Go into folder' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('MARKED FOR CLEANUP')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Clean' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('Selected', { selector: 'span' })).toBeTruthy();
  });

  it('opens on the node it is asked for', async () => {
    const lod = await (
      await import('../../api')
    ).api.getLod(scan.state.status === 'ready' ? scan.state.scanId : 0, 0, 20, 1);
    const home = lod.children.find((child) => child.node.name === 'home')?.node.id as number;

    render(Explorer, { props: { focusId: home } });

    await waitFor(() => expect(crumbs()).toEqual(['/', 'home']));
  });

  it('explains a folder with nothing to draw', async () => {
    const lod = await (
      await import('../../api')
    ).api.getLod(scan.state.status === 'ready' ? scan.state.scanId : 0, 0, 20, 1);
    const mount = lod.children.find((child) => child.node.kind === 'mount')?.node.id as number;

    render(Explorer, { props: { focusId: mount } });

    expect(await screen.findByText(/another disk or a system folder/)).toBeTruthy();
    expect(screen.queryByLabelText('Map of the open folder')).toBeNull();
  });

  it('falls back to the flat map with a note when the 3D view cannot start', async () => {
    // jsdom has no WebGL: the same failure a machine without it would show.
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });

    await fireEvent.click(screen.getByRole('button', { name: '3D' }));

    expect(await screen.findByText(/needs WebGL/, undefined, { timeout: 4000 })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Flat' }).getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByLabelText('Map of the open folder')).toBeTruthy();
    expect(screen.getByText(/^Bigger tile = more space/)).toBeTruthy();
  });

  it('remembers the chosen view', async () => {
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });

    await fireEvent.click(screen.getByRole('button', { name: '3D' }));

    expect(settings.explorerView).toBe('3d');
    expect(screen.getByText(/^Column = folder · floor = sub-folder · volume = space/)).toBeTruthy();
  });

  it('selects a chain tile as its first folder, shows the whole path, and opens its last folder', async () => {
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });

    await fireEvent.click(tile(/^home \/ demo, /));

    expect(screen.getByRole('heading', { name: 'Selected' })).toBeTruthy();
    expect(screen.getByText('/home/demo')).toBeTruthy();
    expect(document.querySelector('.details .share')?.textContent).toMatch(/^\d+(\.\d)?% of this folder$/);
    // The tooltip has the whole path.
    expect(tile(/^home \/ demo, /).querySelector('title')?.textContent).toContain('home / demo');
  });

  it('describes a selected chain tile by its last folder only: path, size and "Show in file manager"', async () => {
    const { api } = await import('../../api');
    const scanId = scan.state.status === 'ready' ? scan.state.scanId : 0;
    const home = (await api.getLod(scanId, 0, 20, 1)).children.find((child) => child.node.name === 'home')
      ?.node as NodeDto;
    const demo = (await api.getChildren(scanId, home.id, 0, 20, 'sizeDesc')).items.find(
      (child) => child.name === 'demo',
    ) as NodeDto;

    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });
    await fireEvent.click(tile(/^home \/ demo, /));

    expect(screen.getByText('/home/demo')).toBeTruthy();
    expect(document.querySelector('.details .big')?.textContent).toBe(fmt.size(demo.allocated));
    expect(document.querySelector('.details .meta')?.textContent).toContain(
      `apparent ${fmt.size(demo.apparent)}`,
    );

    await fireEvent.click(screen.getByRole('button', { name: 'Show in file manager' }));
    expect(openInFileManager).toHaveBeenCalledWith(scanId, demo.id);

    // The same folder that "Go into folder" goes to.
    await fireEvent.click(screen.getByRole('button', { name: 'Go into folder' }));
    await waitFor(() => expect(crumbs()).toEqual(['/', 'home', 'demo']));
  });

  it('gives the share of the folder in the details of every selection, and not for the open folder', async () => {
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });

    expect(document.querySelector('.details .share')).toBeNull();

    await fireEvent.click(tile(/^var, /));

    expect(document.querySelector('.details .share')?.textContent).toMatch(/^\d+(\.\d)?% of this folder$/);
  });

  it('keeps Backspace and Escape in the search field for the field, and goes up only on an Escape with nothing to clear', async () => {
    render(Explorer, { props: {} });
    await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
      name: /^home \/ demo, /,
    });
    await fireEvent.click(tile(/^home \/ demo, /));
    await fireEvent.click(tile(/^home \/ demo, /));
    await waitFor(() => expect(crumbs()).toEqual(['/', 'home', 'demo']));
    await within(map()).findByRole('button', { name: /^Projekty, / });

    // "/" goes to the field.
    await fireEvent.keyDown(document.body, { key: '/' });

    const field = screen.getByPlaceholderText('Search in this folder') as HTMLInputElement;

    expect(document.activeElement).toBe(field);

    await fireEvent.input(field, { target: { value: 'pro' } });
    await fireEvent.keyDown(field, { key: 'Backspace' });
    expect(crumbs()).toEqual(['/', 'home', 'demo']);

    await fireEvent.keyDown(field, { key: 'Escape' });
    expect(crumbs()).toEqual(['/', 'home', 'demo']);
    await waitFor(() => expect(field.value).toBe(''));

    await fireEvent.keyDown(field, { key: 'Escape' });
    await waitFor(() => expect(crumbs()).toEqual(['/', 'home']));
  });

  it('keeps "/" for typing when another field has the focus, and takes Ctrl+F for the search', async () => {
    render(Explorer, { props: {} });
    await screen.findByPlaceholderText('Search in this folder');

    const other = document.body.appendChild(document.createElement('input'));

    other.focus();
    await fireEvent.keyDown(other, { key: '/' });
    expect(document.activeElement).toBe(other);
    other.remove();

    await fireEvent.keyDown(document.body, { key: 'f', ctrlKey: true });
    expect(document.activeElement).toBe(screen.getByPlaceholderText('Search in this folder'));
  });

  it('filters the list by name and starts again when another folder is opened', async () => {
    render(Explorer, { props: {} });
    await screen.findByText('7 items');

    const field = screen.getByPlaceholderText('Search in this folder') as HTMLInputElement;

    await fireEvent.input(field, { target: { value: 'hom' } });
    await waitFor(() => expect(document.querySelectorAll('.row .nm').length).toBe(1));
    expect(document.querySelector('.head .count')?.textContent).toBe('1 of 7 items');

    await fireEvent.click(tile(/^home \/ demo, /));
    await fireEvent.click(tile(/^home \/ demo, /));
    await waitFor(() => expect(crumbs()).toEqual(['/', 'home', 'demo']));
    await waitFor(() => expect(field.value).toBe(''));
  });

  describe('remembers where the user was', () => {
    const scanId = (): number => (scan.state.status === 'ready' ? scan.state.scanId : 0);
    const rootChild = async (name: string): Promise<number> => {
      const lod = await (await import('../../api')).api.getLod(scanId(), 0, 20, 1);

      return lod.children.find((child) => child.node.name === name)?.node.id as number;
    };

    it('shows the whole disk again after it was chosen, even though the Overview had opened a folder', async () => {
      ui.openInExplorer(await rootChild('home'));

      const first = render(Explorer, { props: {} });

      await waitFor(() => expect(crumbs()).toEqual(['/', 'home']));
      await fireEvent.click(await screen.findByRole('button', { name: 'Whole disk' }));
      await waitFor(() => expect(crumbs()).toEqual(['/']));
      await waitFor(() =>
        expect(ui.explorerFocus).toBe(scan.state.status === 'ready' ? scan.state.rootId : -1),
      );

      // Leaving for another page destroys the Explorer; coming back creates a new one.
      first.unmount();
      render(Explorer, { props: {} });

      await within(await screen.findByLabelText('Map of the open folder')).findAllByRole('button');
      expect(crumbs()).toEqual(['/']);
    });

    it('comes back to a deep folder and its colour', async () => {
      const first = render(Explorer, { props: {} });

      await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
        name: /^home \/ demo, /,
      });
      await fireEvent.click(tile(/^home \/ demo, /));
      await fireEvent.click(tile(/^home \/ demo, /));
      await waitFor(() => expect(crumbs()).toEqual(['/', 'home', 'demo']));
      await waitFor(() => expect(ui.explorerFocusColor).toBeDefined());

      const color = ui.explorerFocusColor;

      first.unmount();
      render(Explorer, { props: {} });

      await waitFor(() => expect(crumbs()).toEqual(['/', 'home', 'demo']));
      expect(ui.explorerFocusColor).toBe(color);
      expect(ui.explorerScan).toBe(scanId());
    });

    it('goes where the Overview sends the user, whatever was remembered', async () => {
      const first = render(Explorer, { props: {} });

      await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
        name: /^home \/ demo, /,
      });
      await fireEvent.click(tile(/^home \/ demo, /));
      await fireEvent.click(tile(/^home \/ demo, /));
      await waitFor(() => expect(crumbs()).toEqual(['/', 'home', 'demo']));
      first.unmount();

      ui.openInExplorer(await rootChild('home'), '#123456');
      render(Explorer, { props: {} });

      await waitFor(() => expect(crumbs()).toEqual(['/', 'home']));
    });

    it('ignores a place that was remembered in another scan', async () => {
      ui.explorerFocus = await rootChild('home');
      ui.explorerScan = scanId() + 1000;
      render(Explorer, { props: {} });

      await within(await screen.findByLabelText('Map of the open folder')).findAllByRole('button');
      expect(crumbs()).toEqual(['/']);
    });

    it('opens on the scan root, without an error, when the remembered folder is gone', async () => {
      ui.explorerFocus = 987_654_321;
      ui.explorerScan = scanId();
      render(Explorer, { props: {} });

      await within(await screen.findByLabelText('Map of the open folder')).findAllByRole('button');
      expect(crumbs()).toEqual(['/']);
      expect(screen.queryByText(/could not open/)).toBeNull();
    });
  });

  describe('on a heavily skewed disk', () => {
    beforeEach(async () => {
      scan.reset();
      await startScan('/skew');
    });

    it('says which child holds most of the folder, in place of the usual hint, and opens it from there', async () => {
      render(Explorer, { props: {} });

      const link = await screen.findByRole('button', { name: /home holds 97% of this folder — open it/ });

      expect(screen.queryByText('Click a folder to select it, click again to open it')).toBeNull();

      await fireEvent.click(link);

      await waitFor(() => expect(crumbs().at(-1)).toBe('home'));
      expect(screen.getByText('Click a folder to select it, click again to open it')).toBeTruthy();
    });

    it('draws the small folders as one tile that selects its rows in the list and is never opened', async () => {
      render(Explorer, { props: {} });
      await fireEvent.click(await screen.findByRole('button', { name: /home holds 97%/ }));
      await waitFor(() => expect(crumbs().at(-1)).toBe('home'));

      const rest = await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
        name: /smaller items, /,
      });

      await fireEvent.click(rest);

      await waitFor(() => expect(document.querySelectorAll('.row.sel').length).toBeGreaterThan(10));

      const lit = [...document.querySelectorAll('.row.sel .nm')].map((row) => row.textContent);

      expect(lit.length).toBeGreaterThan(10);
      expect(lit).toContain('misc-1');
      expect(lit).not.toContain('Videos');
      expect(lit).not.toContain('vm-images');
      expect(screen.getByRole('heading', { name: 'Selected' })).toBeTruthy();

      // A second click does not open anything.
      await fireEvent.click(rest);
      expect(crumbs().at(-1)).toBe('home');
    });

    it('says that small folders are drawn larger, and what their true share is', async () => {
      render(Explorer, { props: {} });

      const small = await within(await screen.findByLabelText('Map of the open folder')).findByRole(
        'button',
        {
          name: /^opt, /,
        },
      );

      expect(
        screen.getByText('Bigger tile = more space · small folders are drawn larger so they can be read'),
      ).toBeTruthy();
      expect(small.querySelector('title')?.textContent).toBe('opt · 3.6 GB · 0.2% of this folder');

      await fireEvent.click(small);
      expect(document.querySelector('.details .share')?.textContent).toBe('0.2% of this folder');
    });

    it('does not repeat the share in the tooltip of a tile that is drawn at its size', async () => {
      render(Explorer, { props: {} });

      const big = await within(await screen.findByLabelText('Map of the open folder')).findByRole('button', {
        name: /^home, /,
      });

      expect(big.querySelector('title')?.textContent).toMatch(/^home · [\d.]+ TB$/);
    });

    it('lists the items without a size in one folded row, with the reason for each', async () => {
      render(Explorer, { props: {} });

      const footer = await screen.findByRole('button', { name: /items not counted/ });

      expect(footer.textContent).toContain('10 items not counted');
      expect(within(document.querySelector('.rows') as HTMLElement).queryByText('0 B')).toBeNull();

      await fireEvent.click(footer);

      const why = [...document.querySelectorAll('.row.off')].map((row) => [
        row.querySelector('.nm')?.textContent,
        row.querySelector('.why')?.textContent,
      ]);

      expect(why).toContainEqual(['root', 'no access']);
      expect(why).toContainEqual(['boot', 'other disk']);
      expect(why).toContainEqual(['srv', 'empty']);
      expect(within(document.querySelector('.rows') as HTMLElement).queryByText('0 B')).toBeNull();
    });
  });
});
