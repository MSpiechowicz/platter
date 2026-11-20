import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { UpdateInfo, UpdatePhase } from '../../state/update.svelte';
import UpdateNotice from './UpdateNotice.svelte';

const info: UpdateInfo = {
  current: '0.1.0',
  next: '0.2.0',
  size: '14 MB',
  notes: ['Faster scans'],
  checked: '2 hours ago',
  canInstall: true,
};

function setup(
  props: {
    info?: UpdateInfo;
    phase?: UpdatePhase;
    progress?: number;
    indeterminate?: boolean;
    collapsed?: boolean;
  } = {},
) {
  const onaction = vi.fn();

  render(UpdateNotice, {
    props: { info, phase: 'available', progress: 0, collapsed: false, onaction, ...props },
  });

  return onaction;
}

describe('UpdateNotice', () => {
  it('offers Install and Later, with the version and size', async () => {
    const onaction = setup();

    expect(screen.getByText('Update available')).toBeTruthy();
    expect(screen.getByText('Platter 0.2.0 · 14 MB')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Later' }));

    expect(onaction.mock.calls).toEqual([['install'], ['later']]);
  });

  it('leaves the size out when the release does not say', () => {
    setup({ info: { ...info, size: '' } });

    expect(screen.getByText('Platter 0.2.0')).toBeTruthy();
  });

  it('offers "Get it" and Later when a package manager installs the update', async () => {
    const onaction = setup({ info: { ...info, canInstall: false } });

    expect(screen.getByText('Update available')).toBeTruthy();
    expect(screen.getByText('Platter 0.2.0 · 14 MB')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Install' })).toBeNull();

    await fireEvent.click(screen.getByRole('button', { name: 'Get it' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Later' }));

    expect(onaction.mock.calls).toEqual([['open'], ['later']]);
  });

  it('shows download progress instead of the buttons', () => {
    setup({ phase: 'downloading', progress: 0.4 });

    expect(screen.getByText('Downloading 0.2.0…')).toBeTruthy();
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('40');
    expect(screen.queryByRole('button', { name: 'Install' })).toBeNull();
  });

  it('slides the bar when the size is unknown, and while installing', () => {
    setup({ phase: 'downloading', indeterminate: true });

    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBeNull();
  });

  it('says it is installing once the download is done, with the bar left full', () => {
    setup({ phase: 'installing', progress: 0.3, indeterminate: true });

    expect(screen.getByText('Installing 0.2.0…')).toBeTruthy();
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('100');
    expect(screen.queryByRole('button', { name: 'Install' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Restart to finish' })).toBeNull();
  });

  it('keeps the full bar a moment after the download ends, then offers the restart', async () => {
    vi.useFakeTimers();

    try {
      const onaction = vi.fn();
      const { rerender } = render(UpdateNotice, {
        props: { info, phase: 'downloading', progress: 0.3, collapsed: false, onaction },
      });

      await rerender({ phase: 'ready', progress: 1 });

      expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('100');
      expect(screen.queryByRole('button', { name: 'Restart to finish' })).toBeNull();

      await vi.advanceTimersByTimeAsync(700);

      expect(screen.queryByRole('progressbar')).toBeNull();
      expect(screen.getByText('Update ready')).toBeTruthy();
      expect(screen.getByRole('button', { name: 'Restart to finish' })).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });

  it('ends with "Restart to finish"', async () => {
    const onaction = setup({ phase: 'ready' });

    expect(screen.getByText('Update ready')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Restart to finish' }));

    expect(onaction).toHaveBeenCalledWith('restart');
  });

  it('is only a download button with the version in its name when the sidebar is collapsed', async () => {
    const onaction = setup({ collapsed: true });

    expect(screen.queryByText('Update available')).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Update to Platter 0.2.0' }));

    expect(onaction).toHaveBeenCalledWith('install');
  });

  it('opens the release page from the collapsed button when a package manager installs the update', async () => {
    const onaction = setup({ info: { ...info, canInstall: false }, collapsed: true });

    await fireEvent.click(screen.getByRole('button', { name: 'Get Platter 0.2.0' }));

    expect(onaction).toHaveBeenCalledWith('open');
  });
});
