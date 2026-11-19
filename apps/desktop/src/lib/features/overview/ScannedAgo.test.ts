import { render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ScannedAgo from './ScannedAgo.svelte';

const NOW = 1_750_000_000;
const HOUR = 3600;

describe('ScannedAgo', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(NOW * 1000));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('says how long ago the scan finished', () => {
    render(ScannedAgo, { props: { at: NOW - 2 * HOUR - 60 } });

    expect(screen.getByText('Scanned 2 hours ago')).toBeTruthy();
  });

  it('says "just now" for a scan that has just finished, and moves on by itself', async () => {
    render(ScannedAgo, { props: { at: NOW } });

    expect(screen.getByText('Scanned just now')).toBeTruthy();

    await vi.advanceTimersByTimeAsync(5 * 60_000);

    expect(screen.getByText('Scanned 5 minutes ago')).toBeTruthy();
  });

  it('turns amber once the scan is a day old', () => {
    const fresh = render(ScannedAgo, { props: { at: NOW - 23 * HOUR } });
    expect(screen.getByText(/^Scanned /).classList.contains('old')).toBe(false);
    fresh.unmount();

    render(ScannedAgo, { props: { at: NOW - 25 * HOUR } });
    expect(screen.getByText(/^Scanned /).classList.contains('old')).toBe(true);
  });

  it('stops its timer when it goes away', () => {
    const { unmount } = render(ScannedAgo, { props: { at: NOW } });

    expect(vi.getTimerCount()).toBe(1);

    unmount();

    expect(vi.getTimerCount()).toBe(0);
  });
});
