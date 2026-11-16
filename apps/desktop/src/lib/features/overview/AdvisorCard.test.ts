import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SuggestionDto, SuggestionsDto, Verdict } from '../../api/types';
import { settings } from '../../state/settings.svelte';
import AdvisorCard from './AdvisorCard.svelte';

const GB = 1_000_000_000;

function suggestion(id: string, verdict: Verdict, gb: number, nodes: number[]): SuggestionDto {
  return {
    id,
    title: `Title of ${id}`,
    path: `~/${id}`,
    verdict,
    reason: `Reason of ${id}.`,
    allocated: gb * GB,
    items: nodes.length,
    nodes,
  };
}

const DATA: SuggestionsDto = {
  suggestions: [
    suggestion('target', 'safe', 88, [11, 12]),
    suggestion('trash', 'safe', 17, [13]),
    suggestion('snapshots', 'review', 268, [14]),
    suggestion('steam', 'keep', 142, [15]),
  ],
  totals: { safe: 105 * GB, review: 268 * GB, keep: 142 * GB },
};

function show(
  props: Partial<{
    suggestions: SuggestionsDto | undefined;
    loading: boolean;
    error: unknown;
    onretry: () => void;
    onopen: (id: number) => void;
  }> = {},
): void {
  render(AdvisorCard, {
    props: {
      suggestions: DATA,
      loading: false,
      error: undefined,
      onretry: () => {},
      onopen: () => {},
      ...props,
    },
  });
}

describe('AdvisorCard', () => {
  beforeEach(() => {
    settings.reduceMotion = true;
  });

  afterEach(() => {
    settings.reduceMotion = false;
    vi.useRealTimers();
  });

  it('says it works by rules and counts the suggestions', () => {
    show();

    expect(screen.getByRole('heading', { name: /^Advisor/ }).textContent).toContain('RULES');
    expect(screen.queryByText(/local model/i)).toBeNull();
    expect(screen.getByText('4 suggestions')).toBeTruthy();
  });

  it('groups the rows as safe, review, kept, each header with its item count and size', () => {
    show();

    // The card itself is a region too; the groups are the ones inside its list.
    const groups = screen.getAllByRole('region').filter((region) => region.closest('.list'));

    expect(groups.map((group) => group.getAttribute('aria-label'))).toEqual([
      'Safe to clean',
      'Needs your review',
      'Kept',
    ]);

    const safe = within(groups[0] as HTMLElement);

    expect(safe.getByRole('heading').textContent).toContain('Safe to clean');
    expect(safe.getByText('2 items · 105 GB')).toBeTruthy();
    expect(within(groups[1] as HTMLElement).getByText('1 item · 268 GB')).toBeTruthy();
    expect(within(groups[2] as HTMLElement).getByText('1 item · 142 GB')).toBeTruthy();
  });

  it('keeps every group header inside the scrolling list, moving with its rows', () => {
    show();

    const headers = screen.getAllByRole('heading', { level: 4 });

    expect(headers).toHaveLength(3);

    for (const header of headers) {
      expect(header.classList.contains('gh')).toBe(true);
      expect(header.closest('.list')).toBeTruthy();
    }
  });

  it('shows the title, the path, the reason and the size of every suggestion', () => {
    show();

    const row = screen.getByRole('button', { name: /Title of snapshots/ });

    expect(within(row).getByText('Title of snapshots')).toBeTruthy();
    expect(within(row).getByText('~/snapshots')).toBeTruthy();
    expect(within(row).getByText('Reason of snapshots.')).toBeTruthy();
    expect(within(row).getByText('268 GB')).toBeTruthy();
  });

  it('has nothing to tick, select or clean: no checkbox, and no button besides the rows', () => {
    show();

    expect(screen.queryAllByRole('checkbox')).toEqual([]);
    expect(document.querySelector('input')).toBeNull();
    expect(screen.queryByText('Select')).toBeNull();
    expect(screen.queryByRole('button', { name: /^(clean|select|all|none)/i })).toBeNull();

    const buttons = screen.getAllByRole('button');

    expect(buttons).toHaveLength(DATA.suggestions.length);
    expect(buttons.map((button) => button.textContent)).toEqual(
      expect.arrayContaining([expect.stringContaining('Title of target')]),
    );
  });

  it('shows the safe total and says cleaning is not here yet', () => {
    show();

    expect(screen.getByText('SAFE TO CLEAN')).toBeTruthy();
    expect(screen.getByText('105 GB')).toBeTruthy();
    expect(screen.getByText('Cleaning arrives in the next update.')).toBeTruthy();
    expect(screen.getByText('Nothing is changed for now.')).toBeTruthy();
  });

  it('opens the Explorer on the largest matched node when a row is clicked', async () => {
    const onopen = vi.fn();

    show({ onopen });

    await fireEvent.click(screen.getByRole('button', { name: /Title of target/ }));
    await fireEvent.click(screen.getByRole('button', { name: /Title of steam/ }));

    expect(onopen.mock.calls).toEqual([[11], [15]]);
  });

  it('says so in one sentence when there is nothing to suggest, and still shows the zero total', () => {
    show({ suggestions: { suggestions: [], totals: { safe: 0, review: 0, keep: 0 } } });

    expect(screen.getByText('No suggestions')).toBeTruthy();
    expect(screen.getByText(/^Nothing to suggest here/)).toBeTruthy();
    expect(screen.queryAllByRole('button')).toEqual([]);
    expect(screen.getByText('SAFE TO CLEAN')).toBeTruthy();
  });

  it('shows the logo spinner only after 300 ms of waiting', async () => {
    vi.useFakeTimers();
    show({ suggestions: undefined, loading: true });

    expect(screen.queryByRole('status')).toBeNull();

    await vi.advanceTimersByTimeAsync(350);

    expect(screen.getByRole('status').textContent).toContain('Looking for things to clean');
  });

  it('offers "Try again" when the suggestions could not be loaded', async () => {
    const onretry = vi.fn();

    show({ suggestions: undefined, error: new Error('boom'), onretry });

    expect(screen.getByRole('alert').textContent).toContain('could not be loaded');

    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    expect(onretry).toHaveBeenCalledTimes(1);
  });
});
