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
      expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull();
    });
  });

  it('runs the action and closes the toast when its button is clicked', async () => {
    const run = vi.fn();
    render(ToastHost);
    ui.toast('Freed 35 GB', 'Moved to the trash.', 'ok', { label: 'Undo', run });

    const button = await screen.findByRole('button', { name: 'Undo' });
    await fireEvent.click(button);

    expect(run).toHaveBeenCalledOnce();
    expect(ui.toasts).toHaveLength(0);
  });
});
