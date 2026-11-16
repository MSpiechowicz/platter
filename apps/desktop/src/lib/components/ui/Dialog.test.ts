import { createRawSnippet } from 'svelte';
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import Dialog from './Dialog.svelte';

const actions = createRawSnippet(() => ({
  render: () => '<button data-autofocus>Cancel</button>',
}));

describe('Dialog', () => {
  it('renders nothing while closed', () => {
    render(Dialog, { props: { open: false, title: 'Remove?', actions } });

    expect(screen.queryByText('Remove?')).toBeNull();
  });

  it('opens as a modal dialog with a title, description and the safe button focused', async () => {
    render(Dialog, {
      props: { open: true, title: 'Remove?', description: 'This cannot be undone.', actions },
    });

    const dialog = document.querySelector('dialog') as HTMLDialogElement;
    await Promise.resolve();

    expect(dialog.hasAttribute('open')).toBe(true);
    expect(dialog.getAttribute('aria-labelledby')).toBe(screen.getByText('Remove?').id);
    expect(screen.getByText('This cannot be undone.')).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Cancel' }));
  });

  it('reports closing, from Escape or the native close', async () => {
    const onclose = vi.fn();

    render(Dialog, { props: { open: true, title: 'Remove?', actions, onclose } });

    const dialog = document.querySelector('dialog') as HTMLDialogElement;

    // What Escape does in a browser: the dialog closes itself and then fires `close`.
    dialog.close();
    await Promise.resolve();

    expect(onclose).toHaveBeenCalledOnce();
  });

  it('closes when the backdrop is clicked, but not when the content is', async () => {
    render(Dialog, { props: { open: true, title: 'Remove?', actions } });

    const dialog = document.querySelector('dialog') as HTMLDialogElement;

    await fireEvent.click(screen.getByText('Remove?'));
    expect(dialog.hasAttribute('open')).toBe(true);

    await fireEvent.click(dialog);
    expect(dialog.hasAttribute('open')).toBe(false);
  });
});
