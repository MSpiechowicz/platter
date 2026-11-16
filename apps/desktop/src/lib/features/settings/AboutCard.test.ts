import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AboutCard from './AboutCard.svelte';

const { openExternal } = vi.hoisted(() => ({ openExternal: vi.fn() }));

vi.mock('../../api', () => ({ api: { openExternal } }));

describe('AboutCard', () => {
  beforeEach(() => openExternal.mockReset());

  it('opens its three links through the backend, because target=_blank does nothing in the app window', async () => {
    render(AboutCard);

    await fireEvent.click(screen.getByRole('button', { name: 'Source code' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Report a problem' }));
    await fireEvent.click(screen.getByRole('button', { name: 'License text' }));

    expect(openExternal.mock.calls).toEqual([['source'], ['issues'], ['license']]);
    expect(document.querySelector('a[target="_blank"]')).toBeNull();
  });
});
