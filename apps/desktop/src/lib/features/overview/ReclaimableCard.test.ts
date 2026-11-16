import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SuggestionDto, SuggestionsDto } from '../../api/types';
import { settings } from '../../state/settings.svelte';
import ReclaimableCard from './ReclaimableCard.svelte';

const GB = 1_000_000_000;

const entry = (id: string, verdict: SuggestionDto['verdict'], gb: number): SuggestionDto => ({
  id,
  title: `Title of ${id}`,
  path: `/${id}`,
  verdict,
  reason: '',
  allocated: gb * GB,
  items: 1,
  nodes: [1],
});

const DATA: SuggestionsDto = {
  suggestions: [
    entry('a', 'safe', 88),
    entry('b', 'safe', 61),
    entry('c', 'review', 268),
    entry('d', 'keep', 142),
  ],
  totals: { safe: 149 * GB, review: 268 * GB, keep: 142 * GB },
};

function show(
  props: Partial<{ suggestions: SuggestionsDto | undefined; loading: boolean; error: unknown }> = {},
) {
  render(ReclaimableCard, {
    props: {
      suggestions: DATA,
      loading: false,
      error: undefined,
      onretry: () => {},
      scanned: 1000 * GB,
      ...props,
    },
  });
}

describe('ReclaimableCard', () => {
  beforeEach(() => {
    settings.reduceMotion = true;
  });

  afterEach(() => {
    settings.reduceMotion = false;
    vi.useRealTimers();
  });

  it('shows the three totals, and no "Already freed" row', () => {
    show();

    expect(screen.getByText('Safe to clean').parentElement?.textContent).toContain('149 GB');
    expect(screen.getByText('Needs your review').parentElement?.textContent).toContain('268 GB');
    expect(screen.getByText('Kept').parentElement?.textContent).toContain('142 GB');
    expect(screen.queryByText('Already freed')).toBeNull();
  });

  it('lists the biggest wins largest first, without the kept ones', () => {
    show();

    const heading = screen.getByRole('heading', { name: 'Biggest wins' });
    const rows = [...(heading.parentElement?.querySelectorAll('.recr') ?? [])].slice(3);

    expect(rows.map((row) => row.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
      'Title of c 268 GB',
      'Title of a 88 GB',
      'Title of b 61 GB',
    ]);
    expect(within(heading.parentElement as HTMLElement).queryByText('Title of d')).toBeNull();
  });

  it('shows zeros and no list of wins when there are no suggestions', () => {
    show({ suggestions: { suggestions: [], totals: { safe: 0, review: 0, keep: 0 } } });

    expect(screen.getAllByText('0 B')).toHaveLength(3);
    expect(screen.queryByRole('heading', { name: 'Biggest wins' })).toBeNull();
  });

  it('shows the logo spinner after 300 ms of waiting, and "Try again" after an error', async () => {
    vi.useFakeTimers();
    show({ suggestions: undefined, loading: true });

    expect(screen.queryByRole('status')).toBeNull();
    await vi.advanceTimersByTimeAsync(350);
    expect(screen.getByRole('status')).toBeTruthy();
  });

  it('offers "Try again" when the suggestions could not be loaded', async () => {
    const onretry = vi.fn();

    render(ReclaimableCard, {
      props: { suggestions: undefined, loading: false, error: new Error('x'), onretry, scanned: GB },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    expect(onretry).toHaveBeenCalledTimes(1);
  });
});
