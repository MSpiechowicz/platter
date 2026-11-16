import { fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import PageBoundary from './PageBoundary.svelte';

describe('PageBoundary', () => {
  afterEach(() => vi.restoreAllMocks());

  it('shows a card instead of a blank page when the view fails, logs the error, and tries again', async () => {
    const log = vi.spyOn(console, 'error').mockImplementation(() => {});
    let broken = true;
    const view = createRawSnippet(() => ({
      render: () => {
        if (broken) {
          throw new RangeError('Invalid time value');
        }

        return '<p>The view</p>';
      },
    }));

    render(PageBoundary, { children: view });

    expect(await screen.findByText('This view could not be shown')).toBeTruthy();
    expect(screen.getByRole('alert')).toBeTruthy();
    expect(screen.queryByText('The view')).toBeNull();
    expect(log).toHaveBeenCalledWith('A view could not be shown', expect.any(RangeError));

    broken = false;
    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    expect(await screen.findByText('The view')).toBeTruthy();
    expect(screen.queryByText('This view could not be shown')).toBeNull();
  });

  it('shows the view as it is when nothing fails', () => {
    const view = createRawSnippet(() => ({ render: () => '<p>The view</p>' }));

    render(PageBoundary, { children: view });

    expect(screen.getByText('The view')).toBeTruthy();
    expect(screen.queryByRole('alert')).toBeNull();
  });
});
