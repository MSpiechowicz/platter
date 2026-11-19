import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ui } from '../../state/ui.svelte';
import ToastHost from './ToastHost.svelte';

describe('ToastHost', () => {
  afterEach(() => {
    for (const toast of ui.toasts) {
      ui.dismiss(toast.id);
    }
  });

  it('shows a toast without a button when it has no action', () => {
    render(ToastHost);
    ui.toast('Saved', 'All good.');

    return vi.waitFor(() => {
      expect(screen.getByText('Saved')).toBeTruthy();
      expect(screen.queryByRole('button', { name: 'Rescan' })).toBeNull();
    });
  });

  it('runs the action and closes the toast when its button is clicked', async () => {
    const run = vi.fn();
    render(ToastHost);
    ui.toast('Freed 35 GB', 'The disk changed. Rescan to clean again.', 'ok', { label: 'Rescan', run });

    const button = await screen.findByRole('button', { name: 'Rescan' });
    await fireEvent.click(button);

    expect(run).toHaveBeenCalledOnce();
    expect(ui.toasts).toHaveLength(0);
  });
});
