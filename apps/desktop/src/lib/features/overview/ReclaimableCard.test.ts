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
  props: Partial<{
    suggestions: SuggestionsDto | undefined;
    loading: boolean;
    error: unknown;
    freed: number;
    stale: boolean;
  }> = {},
) {
  render(ReclaimableCard, {
    props: {
      suggestions: DATA,
      loading: false,
      error: undefined,
      onretry: () => {},
      scanned: 1000 * GB,
      freed: 0,
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

  it('shows the three totals and an "Already freed" row after them, 0 before any cleanup', () => {
    show();

    expect(screen.getByText('Safe to clean').parentElement?.textContent).toContain('149 GB');
    expect(screen.getByText('Needs your review').parentElement?.textContent).toContain('268 GB');
    expect(screen.getByText('Kept').parentElement?.textContent).toContain('142 GB');

    const rows = [...document.querySelectorAll('.recr')].map((row) =>
      row.textContent?.replace(/\s+/g, ' ').trim(),
    );

    expect(rows.slice(0, 4)).toEqual([
      'Safe to clean 149 GB',
      'Needs your review 268 GB',
      'Kept 142 GB',
      'Already freed 0 B',
    ]);
  });

  it('shows what this session freed in the "Already freed" row, outside the bar', () => {
    show({ freed: 1_200_000_000 });

    expect(screen.getByText('Already freed').parentElement?.textContent).toContain('1.2 GB');
    expect(screen.getByRole('img').getAttribute('aria-label')).not.toContain('Already freed');
    expect(document.querySelectorAll('.fill i')).toHaveLength(3);
  });

  it('lists the biggest wins largest first, without the kept ones', () => {
    show();

    const heading = screen.getByRole('heading', { name: 'Biggest wins' });
    const rows = [...(heading.parentElement?.querySelectorAll('.recr') ?? [])].slice(4);

    expect(rows.map((row) => row.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
      'Title of c 268 GB',
      'Title of a 88 GB',
      'Title of b 61 GB',
    ]);
    expect(within(heading.parentElement as HTMLElement).queryByText('Title of d')).toBeNull();
  });

  it('shows zeros and no list of wins when there are no suggestions', () => {
    show({ suggestions: { suggestions: [], totals: { safe: 0, review: 0, keep: 0 } } });

    expect(screen.getAllByText('0 B')).toHaveLength(4);
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
      props: {
        suggestions: undefined,
        loading: false,
        error: new Error('x'),
        onretry,
        scanned: GB,
        freed: 0,
      },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    expect(onretry).toHaveBeenCalledTimes(1);
  });
});

describe('ReclaimableCard after a cleanup, until a rescan', () => {
  const NOTE = 'From the scan before the cleanup — rescan to update';
  const rowOf = (text: string): HTMLElement => screen.getByText(text).closest('.recr') as HTMLElement;

  beforeEach(() => {
    settings.reduceMotion = true;
  });

  afterEach(() => {
    settings.reduceMotion = false;
  });

  it('shows everything in full while the scan is current', () => {
    show();

    expect(document.querySelectorAll('.stale')).toHaveLength(0);
    expect(document.querySelector('[title]')).toBeNull();
  });

  it('greys the bar, the three rows and the biggest wins, with a tooltip, and leaves "Already freed" alone', () => {
    show({ stale: true, freed: 5 * GB });

    const greyed = [
      screen.getByRole('img'),
      rowOf('Safe to clean'),
      rowOf('Needs your review'),
      rowOf('Kept'),
      rowOf('Title of c'),
      rowOf('Title of a'),
    ];

    for (const element of greyed) {
      expect(element.classList.contains('stale')).toBe(true);
      expect(element.getAttribute('title')).toBe(NOTE);
    }

    expect(rowOf('Already freed').classList.contains('stale')).toBe(false);
    expect(rowOf('Already freed').getAttribute('title')).toBeNull();
    expect(rowOf('Already freed').textContent).toContain('5.0 GB');
    expect(screen.getByRole('heading', { name: 'Biggest wins' }).classList.contains('stale')).toBe(false);
  });

  it('keeps every figure on screen: nothing is removed or guessed', () => {
    show({ stale: true });

    expect(screen.getByText('Safe to clean').parentElement?.textContent).toContain('149 GB');
    expect(screen.getByText('Needs your review').parentElement?.textContent).toContain('268 GB');
    expect(document.querySelectorAll('.fill i')).toHaveLength(3);
  });
});
