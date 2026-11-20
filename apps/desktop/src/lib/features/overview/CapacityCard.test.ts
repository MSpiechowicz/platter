import { flushSync } from 'svelte';
import { render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Totals, VolumeDto } from '../../api/types';
import { settings } from '../../state/settings.svelte';
import CapacityCard from './CapacityCard.svelte';

const GB = 1_000_000_000;

const volume: VolumeDto = {
  id: 'nvme0n1p2',
  mountPoint: '/',
  label: '',
  device: '/dev/nvme0n1p2',
  fsType: 'btrfs',
  total: 1000 * GB,
  available: 260 * GB,
  removable: false,
};

const totals: Totals = { allocated: 740 * GB, apparent: 700 * GB, files: 2000, dirs: 300, errors: 0 };

/** The gauge's figure, whose "%" is a smaller element of its own: matched by the whole text of the figure. */
function gaugeFigure(text: string): HTMLElement {
  return screen.getByText(
    (_, element) => element?.classList.contains('pct') === true && element.textContent === text,
  );
}

describe('CapacityCard', () => {
  beforeEach(() => {
    vi.useFakeTimers({
      toFake: ['requestAnimationFrame', 'cancelAnimationFrame', 'performance', 'setTimeout'],
    });
    settings.reduceMotion = false;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
    settings.reduceMotion = false;
  });

  it('counts up from zero to the used share of the volume, never below zero', () => {
    // Browsers hand `requestAnimationFrame` the frame start, which can be older than the moment the animation began.
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) =>
      setTimeout(() => callback(performance.now() - 50), 16),
    );
    vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id));

    render(CapacityCard, { props: { volume, totals } });

    expect(gaugeFigure('0%')).toBeTruthy();

    for (let ms = 0; ms < 1300; ms += 16) {
      vi.advanceTimersByTime(16);
      flushSync();
      expect(document.body.textContent).not.toMatch(/-\d/);
    }

    expect(gaugeFigure('74%')).toBeTruthy();
    expect(screen.getByText('740 GB')).toBeTruthy();
    expect(screen.getByText('260 GB')).toBeTruthy();
    expect(screen.getByText('1.00 TB')).toBeTruthy();
  });

  it('shows the final numbers at once when motion is reduced', () => {
    settings.reduceMotion = true;

    render(CapacityCard, { props: { volume, totals } });

    expect(gaugeFigure('74%')).toBeTruthy();
    expect(screen.getByText('740 GB')).toBeTruthy();
  });

  it('shows only what was scanned when the root is not a whole volume', () => {
    settings.reduceMotion = true;

    render(CapacityCard, { props: { totals } });

    expect(screen.getByText('Scanned')).toBeTruthy();
    expect(screen.getByText('ON DISK')).toBeTruthy();
    expect(screen.queryByText('SCANNED')).toBeNull();
    expect(screen.getByText('740 GB')).toBeTruthy();
    expect(screen.getByText('2,000')).toBeTruthy();
    expect(screen.getByText('300')).toBeTruthy();
    expect(screen.queryByText('USED')).toBeNull();
    expect(screen.queryByText('free')).toBeNull();
  });

  it('names a file size only when it differs from the space on disk, else the share of the disk', () => {
    settings.reduceMotion = true;

    // 700 GB of files on 740 GB of disk: more than 5% apart, worth showing.
    const differs = render(CapacityCard, { props: { totals, container: volume } });
    expect(screen.getByText('file size')).toBeTruthy();
    expect(screen.getByText('700 GB')).toBeTruthy();
    differs.unmount();

    const same = { ...totals, apparent: totals.allocated };
    const onDisk = render(CapacityCard, { props: { totals: same, container: volume } });
    expect(screen.queryByText('file size')).toBeNull();
    expect(screen.getByText('of its disk')).toBeTruthy();
    expect(screen.getByText('74%')).toBeTruthy();
    onDisk.unmount();

    const tiny = { ...same, allocated: 25_000_000, apparent: 25_000_000 };
    const small = render(CapacityCard, { props: { totals: tiny, container: volume } });
    expect(screen.getByText('<0.1%')).toBeTruthy();
    small.unmount();

    // Nothing more to say: files and folders only, no repeated size.
    render(CapacityCard, { props: { totals: same } });
    expect(screen.queryByText('file size')).toBeNull();
    expect(screen.queryByText('of its disk')).toBeNull();
  });

  it('ignores fields of the totals and of the volume that it does not know', () => {
    settings.reduceMotion = true;

    const richer = { ...totals, warnings: ['mountTableUnavailable'] } as Totals;
    const newer = { ...volume, somethingNew: 1 } as VolumeDto;

    render(CapacityCard, { props: { volume: newer, totals: richer } });

    expect(gaugeFigure('74%')).toBeTruthy();
    expect(screen.getByText('260 GB')).toBeTruthy();
  });

  it('says that blocks are shared when a btrfs scan counts more than the disk uses', () => {
    settings.reduceMotion = true;
    const NOTE = /share disk blocks/;

    const { unmount } = render(CapacityCard, {
      props: { volume, totals: { ...totals, allocated: 900 * GB } },
    });
    expect(screen.queryByText(NOTE)).toBeTruthy();
    unmount();

    // Exactly what the disk uses, or less: nothing to explain.
    const exact = render(CapacityCard, { props: { volume, totals } });
    expect(screen.queryByText(NOTE)).toBeNull();
    exact.unmount();

    // Another filesystem: more than used is not explained by shared blocks, so the line is not shown.
    const ext4 = render(CapacityCard, {
      props: { volume: { ...volume, fsType: 'ext4' }, totals: { ...totals, allocated: 900 * GB } },
    });
    expect(screen.queryByText(NOTE)).toBeNull();
    ext4.unmount();

    // A folder scan has no disk to compare with.
    render(CapacityCard, { props: { totals: { ...totals, allocated: 900 * GB } } });
    expect(screen.queryByText(NOTE)).toBeNull();
  });

  it('greys its figures and tags the card as an old scan after a cleanup, and not before', () => {
    settings.reduceMotion = true;

    const current = render(CapacityCard, { props: { volume, totals } });

    expect(screen.queryByText('OLD SCAN')).toBeNull();
    expect(document.querySelectorAll('.stale')).toHaveLength(0);
    current.unmount();

    render(CapacityCard, { props: { volume, totals, stale: true } });

    expect(screen.getByText('OLD SCAN')).toBeTruthy();
    expect(screen.getByText('740 GB').closest('.stale')?.getAttribute('title')).toMatch(
      /scan before the cleanup/,
    );
  });
});
