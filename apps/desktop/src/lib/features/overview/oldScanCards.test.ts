import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import type { NodeDto, SummaryDto } from '../../api/types';
import AgeCard from './AgeCard.svelte';
import DiskStackCard from './DiskStackCard.svelte';
import LargestFoldersCard from './LargestFoldersCard.svelte';
import type { Slab } from './slabs';

const GB = 1_000_000_000;

const byAge: SummaryDto['byAge'] = [
  { bucket: 'week', allocated: 10 * GB, files: 1 },
  { bucket: 'older', allocated: 30 * GB, files: 1 },
];

const slabs: Slab[] = [
  { key: 'a', name: '/home', size: 600 * GB, share: 0.6, color: 'red', inside: '', nodeId: 7 },
  { key: 'free', name: 'free space', size: 400 * GB, share: 0.4, color: 'grey', inside: '', free: true },
];

const folder = (id: number, name: string, gb: number): NodeDto => ({
  id,
  name,
  kind: 'dir',
  allocated: gb * GB,
  apparent: gb * GB,
  files: 1,
  hasChildren: true,
  category: 'other',
  mtime: 0,
});

const folders = [folder(1, 'Videos', 90), folder(2, 'Games', 60), folder(3, 'Music', 30)];
const paths = new Map([
  [1, '/home/Videos'],
  [2, '/home/Games'],
  [3, '/home/Music'],
]);

const NOTE = /scan before the cleanup/;

describe('the Overview cards of a scan that a cleanup made old', () => {
  it('AgeCard greys its chart and sum, and tags the card', () => {
    const current = render(AgeCard, { byAge, total: 100 * GB });

    expect(screen.queryByText('OLD SCAN')).toBeNull();
    expect(document.querySelectorAll('.stale')).toHaveLength(0);
    current.unmount();

    render(AgeCard, { byAge, total: 100 * GB, stale: true });

    expect(screen.getByText('OLD SCAN')).toBeTruthy();

    // The 30 GB of the oldest bar and the sum of what was not touched for a year.
    const figures = [...screen.getAllByText('10 GB'), ...screen.getAllByText('30 GB')];

    expect(figures).toHaveLength(3);
    for (const figure of figures) {
      expect(figure.closest('.stale')?.getAttribute('title')).toMatch(NOTE);
    }
  });

  it('DiskStackCard shows the tag in place of its hint, greys the drawing, and still opens a layer', async () => {
    const onopen = vi.fn();
    const current = render(DiskStackCard, { slabs, total: 1000 * GB, onopen });

    expect(screen.getByText('layer thickness = space used')).toBeTruthy();
    expect(screen.queryByText('OLD SCAN')).toBeNull();
    current.unmount();

    const { container } = render(DiskStackCard, { slabs, total: 1000 * GB, onopen, stale: true });

    expect(screen.getByText('OLD SCAN')).toBeTruthy();
    expect(screen.queryByText('layer thickness = space used')).toBeNull();
    expect(container.querySelector('.stale')?.getAttribute('title')).toMatch(NOTE);

    await fireEvent.click(screen.getByRole('button', { name: /^\/home, 600 GB/ }));

    expect(onopen).toHaveBeenCalledWith(7, 'red');
  });

  it('LargestFoldersCard greys the sizes and bars, tags the card, and still opens a folder', async () => {
    const onopen = vi.fn();
    const colors = new Map<number, string>();
    const current = render(LargestFoldersCard, { folders, paths, colors, onopen });

    expect(screen.queryByText('OLD SCAN')).toBeNull();
    expect(document.querySelectorAll('.stale')).toHaveLength(0);
    current.unmount();

    render(LargestFoldersCard, { folders, paths, colors, onopen, stale: true });

    expect(screen.getByText('OLD SCAN')).toBeTruthy();
    expect(screen.getByText('90 GB').classList.contains('stale')).toBe(true);
    expect(screen.getByText('90 GB').getAttribute('title')).toMatch(NOTE);

    await fireEvent.click(screen.getByText('/home/Videos'));

    expect(onopen).toHaveBeenCalledWith(1, undefined);
  });
});
