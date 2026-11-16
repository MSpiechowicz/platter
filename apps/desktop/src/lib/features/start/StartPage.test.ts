import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { VolumeDto } from '../../api/types';
import { scan } from '../../state/scan.svelte';
import StartPage from './StartPage.svelte';

const volume = (id: string, mountPoint: string, device = `/dev/${id}`): VolumeDto => ({
  id,
  mountPoint,
  label: '',
  device,
  fsType: 'ext4',
  total: 1_000_000_000,
  available: 400_000_000,
  removable: false,
});

const rows = (): HTMLElement[] =>
  screen.getAllByRole('button').filter((button) => button.hasAttribute('aria-pressed'));

const selected = (): string[] =>
  rows()
    .filter((row) => row.getAttribute('aria-pressed') === 'true')
    .map((row) => row.querySelector('.path')?.textContent ?? '');

describe('StartPage', () => {
  afterEach(() => {
    scan.volumes = [];
    vi.restoreAllMocks();
  });

  it('lists volumes that share an id and a mount point without throwing on the duplicate key', () => {
    scan.volumes = [volume('sda1', '/data'), volume('sda1', '/data'), volume('sdb1', '/data')];

    render(StartPage);

    // The first two are one filesystem, so they are one row.
    expect(rows()).toHaveLength(2);
  });

  it('shows one row for a filesystem mounted several times, with the other mount points dim underneath', () => {
    const root = (id: string, mountPoint: string): VolumeDto => volume(id, mountPoint, '/dev/nvme0n1p2');

    scan.volumes = [root('home', '/home'), root('root', '/'), root('srv', '/srv'), volume('sda1', '/mnt/a')];

    render(StartPage);

    expect(rows()).toHaveLength(2);
    expect(rows()[0]?.textContent).toContain('also mounted at /srv, /home');
    expect(rows()[0]?.querySelector('.path')?.textContent).toBe('/');
    expect(screen.getByTitle('Also mounted at /srv, /home')).toBeTruthy();
    expect(rows()[1]?.textContent).not.toContain('also mounted');
  });

  it('selects by volume id: two rows with the same mount point are never both selected', async () => {
    scan.volumes = [volume('a', '/data'), volume('b', '/data')];

    render(StartPage);
    expect(selected()).toEqual(['/data']);
    expect(rows()[0]?.getAttribute('aria-pressed')).toBe('true');

    await fireEvent.click(rows()[1] as HTMLElement);

    expect(selected()).toEqual(['/data']);
    expect(rows()[0]?.getAttribute('aria-pressed')).toBe('false');
    expect(rows()[1]?.getAttribute('aria-pressed')).toBe('true');
  });

  it('scans the mount point of the selected row', async () => {
    const start = vi.spyOn(scan, 'start').mockResolvedValue();

    scan.volumes = [volume('a', '/'), volume('b', '/mnt/b')];

    render(StartPage);
    await fireEvent.click(rows()[1] as HTMLElement);
    await fireEvent.click(screen.getByRole('button', { name: 'Scan' }));

    expect(start).toHaveBeenCalledWith(expect.objectContaining({ root: '/mnt/b' }));
  });
});
