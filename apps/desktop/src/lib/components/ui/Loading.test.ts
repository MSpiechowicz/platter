import { render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import Loading from './Loading.svelte';

describe('Loading', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('shows nothing for the first 300 ms, then the spinner with what is happening', () => {
    render(Loading, { props: { when: true, label: 'Reading the scan…' } });

    vi.advanceTimersByTime(299);
    flushSync();
    expect(screen.queryByRole('status')).toBeNull();

    vi.advanceTimersByTime(1);
    flushSync();
    expect(screen.getByRole('status').textContent).toContain('Reading the scan…');
  });

  it('never shows if the wait is over before 300 ms', async () => {
    const { rerender } = render(Loading, { props: { when: true, label: 'Reading…' } });

    vi.advanceTimersByTime(200);
    await rerender({ when: false, label: 'Reading…' });
    vi.advanceTimersByTime(1000);
    flushSync();

    expect(screen.queryByRole('status')).toBeNull();
  });

  it('goes away when the wait is over', async () => {
    const { rerender } = render(Loading, { props: { when: true, label: 'Reading…' } });

    vi.advanceTimersByTime(400);
    flushSync();
    expect(screen.getByRole('status')).toBeTruthy();

    await rerender({ when: false, label: 'Reading…' });
    flushSync();
    expect(screen.queryByRole('status')).toBeNull();
  });
});
