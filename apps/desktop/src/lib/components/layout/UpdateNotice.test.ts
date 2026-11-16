import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { UpdateInfo } from '../../state/update.svelte';
import UpdateNotice from './UpdateNotice.svelte';

const info: UpdateInfo = {
  current: '0.1.0',
  next: '0.2.0',
  size: '14 MB',
  notes: ['Faster scans'],
  checked: '2 hours ago',
};

function setup(
  props: { phase?: 'available' | 'downloading' | 'ready'; progress?: number; collapsed?: boolean } = {},
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

  it('shows download progress instead of the buttons', () => {
    setup({ phase: 'downloading', progress: 0.4 });

    expect(screen.getByText('Downloading 0.2.0…')).toBeTruthy();
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('40');
    expect(screen.queryByRole('button', { name: 'Install' })).toBeNull();
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
});
