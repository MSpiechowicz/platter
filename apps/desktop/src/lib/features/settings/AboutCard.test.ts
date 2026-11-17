import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { updater } from '../../state/update.svelte';
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

  it('names both network calls: the optional model download and the update check that can be turned off', () => {
    render(AboutCard);

    expect(
      screen.getByText(
        /The only network calls are the optional model download and the update check, which you can turn off in Settings\./,
      ),
    ).toBeTruthy();
  });

  it('shows the version the update check reported, and a fallback before that', () => {
    updater.current = null;
    const { unmount } = render(AboutCard);

    expect(screen.getByText('Version 0.1.0')).toBeTruthy();
    unmount();

    updater.current = '0.3.1';
    render(AboutCard);

    expect(screen.getByText('Version 0.3.1')).toBeTruthy();
    updater.current = null;
  });
});
