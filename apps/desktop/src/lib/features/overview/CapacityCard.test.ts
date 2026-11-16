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

    expect(screen.getByText('0%')).toBeTruthy();

    for (let ms = 0; ms < 1300; ms += 16) {
      vi.advanceTimersByTime(16);
      flushSync();
      expect(document.body.textContent).not.toMatch(/-\d/);
    }

    expect(screen.getByText('74%')).toBeTruthy();
    expect(screen.getByText('740 GB')).toBeTruthy();
    expect(screen.getByText('260 GB')).toBeTruthy();
    expect(screen.getByText('1.00 TB')).toBeTruthy();
  });

  it('shows the final numbers at once when motion is reduced', () => {
    settings.reduceMotion = true;

    render(CapacityCard, { props: { volume, totals } });

    expect(screen.getByText('74%')).toBeTruthy();
    expect(screen.getByText('740 GB')).toBeTruthy();
  });

  it('shows only what was scanned when the root is not a whole volume', () => {
    settings.reduceMotion = true;

    render(CapacityCard, { props: { totals } });

    expect(screen.getByText('Scanned')).toBeTruthy();
    expect(screen.getByText('SCANNED')).toBeTruthy();
    expect(screen.getByText('740 GB')).toBeTruthy();
    expect(screen.getByText('2,000')).toBeTruthy();
    expect(screen.getByText('300')).toBeTruthy();
    expect(screen.queryByText('USED')).toBeNull();
    expect(screen.queryByText('free')).toBeNull();
  });

  it('ignores fields of the totals and of the volume that it does not know', () => {
    settings.reduceMotion = true;

    const richer = { ...totals, warnings: ['mountTableUnavailable'] } as Totals;
    const newer = { ...volume, somethingNew: 1 } as VolumeDto;

    render(CapacityCard, { props: { volume: newer, totals: richer } });

    expect(screen.getByText('74%')).toBeTruthy();
    expect(screen.getByText('260 GB')).toBeTruthy();
  });
});
