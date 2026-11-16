import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import type { VolumeDto } from '../../api/types';
import Sidebar, { type NavItem } from './Sidebar.svelte';

const ITEMS: NavItem[] = [
  { id: 'cleanup', label: 'Cleanup', icon: 'cleanup' },
  { id: 'advisor', label: 'Advisor', icon: 'advisor' },
];

const VOLUME: VolumeDto = {
  id: 'v',
  mountPoint: '/',
  label: '',
  device: '/dev/nvme0n1p2',
  fsType: 'btrfs',
  total: 1_900_000_000_000,
  available: 492_000_000_000,
  removable: false,
};

function show(props: Partial<Parameters<typeof render<typeof Sidebar>>[1]> & Record<string, unknown> = {}) {
  render(Sidebar, {
    props: { items: ITEMS, active: 'advisor', onselect: () => {}, collapsed: false, ...props },
  });
}

describe('Sidebar: the advisor row', () => {
  it('shows "Advisor model" with the memory it holds, under the disk, while the model is loaded', () => {
    show({ volume: VOLUME, advisorBytes: 2_147_483_648 });

    expect(screen.getByText('Advisor model')).toBeTruthy();
    expect(screen.getByText('2.1 GB')).toBeTruthy();
    expect(screen.getByText('nvme0n1p2')).toBeTruthy();
  });

  it('is not there when the model is not loaded', () => {
    show({ volume: VOLUME });

    expect(screen.queryByText('Advisor model')).toBeNull();
  });

  it('shows the row alone when no disk was scanned yet', () => {
    show({ advisorBytes: 1_000_000_000 });

    expect(screen.getByText('Advisor model')).toBeTruthy();
    expect(screen.getByText('1.0 GB')).toBeTruthy();
  });

  it('leaves the row out of the icon rail', () => {
    show({ volume: VOLUME, advisorBytes: 1_000_000_000, collapsed: true });

    expect(screen.queryByText('Advisor model')).toBeNull();
  });

  it('has the Advisor item with its icon', () => {
    show();

    expect(screen.getByRole('button', { name: 'Advisor' }).querySelector('svg')).toBeTruthy();
  });
});
