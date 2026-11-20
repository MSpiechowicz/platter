import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { diskAccess } from '../../state/access.svelte';
import AccessBanner from './AccessBanner.svelte';
import OutdatedBanner from './OutdatedBanner.svelte';

describe('AccessBanner', () => {
  afterEach(() => {
    diskAccess.status = 'unknown';
    diskAccess.dismissed = false;
    vi.restoreAllMocks();
  });

  it('asks for Full Disk Access and opens System Settings', async () => {
    const open = vi.spyOn(diskAccess, 'openSettings').mockResolvedValue();

    render(AccessBanner);

    expect(screen.getByRole('region', { name: 'Platter needs Full Disk Access' })).toBeTruthy();
    expect(screen.getByText(/After an update, switch it on again/)).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Open System Settings' }));

    expect(open).toHaveBeenCalledOnce();
  });

  it('"Not now" hides it', async () => {
    diskAccess.status = 'missing';

    render(AccessBanner);
    expect(diskAccess.shown).toBe(true);

    await fireEvent.click(screen.getByRole('button', { name: 'Not now' }));

    expect(diskAccess.shown).toBe(false);
  });

  it('counts the room of both banners when they are on screen together, and gives back each one', () => {
    const main = document.createElement('main');

    document.body.append(main);

    const height = vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(50);
    const access = render(AccessBanner, { target: main });
    const outdated = render(OutdatedBanner, { target: main });

    // jsdom has no layout: each banner is 50 tall, and its 16px margin comes from no stylesheet here.
    expect(main.style.getPropertyValue('--banner-space')).toBe('100px');

    access.unmount();
    expect(main.style.getPropertyValue('--banner-space')).toBe('50px');

    outdated.unmount();
    expect(main.style.getPropertyValue('--banner-space')).toBe('');

    height.mockRestore();
    main.remove();
  });
});
