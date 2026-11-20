import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup } from '../../state/cleanup.svelte';
import { history } from '../../state/history.svelte';
import { scan } from '../../state/scan.svelte';
import OutdatedBanner from './OutdatedBanner.svelte';

describe('OutdatedBanner', () => {
  afterEach(() => {
    cleanup.phase = { kind: 'idle' };
    cleanup.undoing = false;
    history.restoring = undefined;
    vi.restoreAllMocks();
  });

  it('says the scan is out of date and holds a Rescan that starts a new scan', async () => {
    const rescan = vi.spyOn(scan, 'rescan').mockResolvedValue();

    render(OutdatedBanner);

    expect(screen.getByRole('region', { name: 'This scan is out of date' })).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Rescan' }));

    expect(rescan).toHaveBeenCalledOnce();
  });

  it('fills its live region only once it is on the page, so a screen reader reads it out', async () => {
    vi.useFakeTimers();

    try {
      render(OutdatedBanner);

      const live = screen.getByRole('status');

      expect(live.textContent).toBe('');

      await vi.advanceTimersByTimeAsync(200);

      expect(live.textContent).toBe('This scan is out of date. Cleaning is paused until you rescan.');
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps Rescan off while a cleanup is running', () => {
    cleanup.phase = { kind: 'planning' };

    render(OutdatedBanner);

    expect((screen.getByRole('button', { name: 'Rescan' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('keeps Rescan off while an undo puts files back, so the new scan never sees half a restore', () => {
    cleanup.undoing = true;

    render(OutdatedBanner);

    expect((screen.getByRole('button', { name: 'Rescan' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('keeps Rescan off while History puts a cleanup back', () => {
    history.restoring = 3;

    render(OutdatedBanner);

    expect((screen.getByRole('button', { name: 'Rescan' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('gives the main area the height it takes, so pages that fill the window can make room, and takes it back', () => {
    const main = document.createElement('main');

    document.body.append(main);

    const { unmount } = render(OutdatedBanner, { target: main });

    expect(main.style.getPropertyValue('--banner-space')).not.toBe('');

    unmount();

    expect(main.style.getPropertyValue('--banner-space')).toBe('');
    main.remove();
  });
});
