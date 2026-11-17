import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { settings } from '../../state/settings.svelte';
import { theme } from '../../state/theme.svelte';
import { ui } from '../../state/ui.svelte';
import CleanupSettingsCard from './CleanupSettingsCard.svelte';
import ExcludedPaths from './ExcludedPaths.svelte';
import SettingsPage from './SettingsPage.svelte';
import UpdatesSettingsCard from './UpdatesSettingsCard.svelte';

const { api } = vi.hoisted(() => ({
  api: { pickExcludedFolder: vi.fn(), checkUpdate: vi.fn(), openExternal: vi.fn() },
}));

vi.mock('../../api', () => ({ api }));

beforeEach(() => {
  api.pickExcludedFolder.mockReset();
  settings.set('excludedPaths', []);
  settings.set('keepPackageVersions', 2);
  settings.set('askForAdmin', true);
  settings.set('autoInstallUpdates', true);
  settings.set('theme', 'dark');
  theme.setChoice('dark');
  ui.toasts = [];
});

describe('Excluded paths', () => {
  it('adds the folder the picker returns, once, and removes it again', async () => {
    api.pickExcludedFolder.mockResolvedValue('/home/demo/Videos');
    render(ExcludedPaths);

    await fireEvent.click(screen.getByRole('button', { name: 'Add folder…' }));
    await vi.waitFor(() => expect(settings.excludedPaths).toEqual(['/home/demo/Videos']));
    await fireEvent.click(screen.getByRole('button', { name: 'Add folder…' }));
    await vi.waitFor(() => expect(api.pickExcludedFolder).toHaveBeenCalledTimes(2));
    expect(settings.excludedPaths).toEqual(['/home/demo/Videos']);
    expect(screen.getByText('/home/demo/Videos')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Stop leaving out /home/demo/Videos' }));

    expect(settings.excludedPaths).toEqual([]);
  });

  it('changes nothing when the picker is cancelled', async () => {
    api.pickExcludedFolder.mockResolvedValue(null);
    render(ExcludedPaths);

    await fireEvent.click(screen.getByRole('button', { name: 'Add folder…' }));
    await vi.waitFor(() => expect(api.pickExcludedFolder).toHaveBeenCalled());

    expect(settings.excludedPaths).toEqual([]);
    expect(ui.toasts).toEqual([]);
  });
});

describe('Cleanup card', () => {
  it('saves the kept package versions and the administrator rights', async () => {
    render(CleanupSettingsCard);

    await fireEvent.click(screen.getByRole('button', { name: /Keep package versions/ }));
    await fireEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: '3' }));
    await fireEvent.click(screen.getByRole('switch', { name: 'Ask for administrator rights when needed' }));

    expect(settings.keepPackageVersions).toBe(3);
    expect(settings.askForAdmin).toBe(false);
  });
});

describe('Install updates automatically', () => {
  it('is on by default and saves the choice', async () => {
    render(UpdatesSettingsCard);
    const toggle = screen.getByRole('switch', { name: 'Install updates automatically' }) as HTMLInputElement;

    expect(toggle.checked).toBe(true);
    await fireEvent.click(toggle);

    expect(settings.autoInstallUpdates).toBe(false);
  });
});

describe('Theme', () => {
  it('saves the choice and shows it at once', async () => {
    render(SettingsPage);

    await fireEvent.click(
      within(screen.getByRole('group', { name: 'Theme' })).getByRole('button', { name: 'Light' }),
    );
    await tick();

    expect(settings.theme).toBe('light');
    expect(theme.effective).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
  });
});
