import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError } from '../../api/client';
import { platform } from '../../state/platform.svelte';
import { settings } from '../../state/settings.svelte';
import { theme } from '../../state/theme.svelte';
import { ui } from '../../state/ui.svelte';
import CleanupSettingsCard from './CleanupSettingsCard.svelte';
import ExcludedPaths from './ExcludedPaths.svelte';
import SettingsPage from './SettingsPage.svelte';
import UpdatesSettingsCard from './UpdatesSettingsCard.svelte';

const { api } = vi.hoisted(() => ({
  api: { pickExcludedFolder: vi.fn(), checkUpdate: vi.fn(), openExternal: vi.fn(), forgetLastScan: vi.fn() },
}));

vi.mock('../../api', () => ({ api }));

beforeEach(() => {
  api.pickExcludedFolder.mockReset();
  settings.set('excludedPaths', []);
  settings.set('keepPackageVersions', 2);
  settings.set('askForAdmin', true);
  settings.set('autoInstallUpdates', true);
  settings.set('theme', 'dark');
  settings.set('rescanOnOpen', false);
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

describe('Settings on Windows', () => {
  afterEach(() => {
    platform.os = 'linux';
  });

  it('leaves out the pacman row, which has nothing to set there, and keeps the automatic install switch', () => {
    platform.os = 'windows';
    render(CleanupSettingsCard);

    expect(screen.queryByRole('button', { name: /Keep package versions/ })).toBeNull();
    expect(screen.getByRole('switch', { name: 'Ask for administrator rights when needed' })).toBeTruthy();

    render(UpdatesSettingsCard);

    expect(screen.getByRole('switch', { name: 'Install updates automatically' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Check now' })).toBeTruthy();
  });
});

describe('Settings on macOS', () => {
  afterEach(() => {
    platform.os = 'linux';
  });

  it('has no Cleanup card, because cleaning only moves to the Trash and no pacman or administrator row is left', () => {
    platform.os = 'macos';
    render(SettingsPage);

    expect(screen.queryByRole('region', { name: 'Cleanup' })).toBeNull();
    expect(screen.queryByRole('button', { name: /Keep package versions/ })).toBeNull();
    expect(screen.queryByRole('switch', { name: 'Ask for administrator rights when needed' })).toBeNull();
    expect(screen.getByRole('switch', { name: 'Install updates automatically' })).toBeTruthy();
  });

  it('keeps the Cleanup card on Linux', () => {
    render(SettingsPage);

    expect(screen.getByRole('region', { name: 'Cleanup' })).toBeTruthy();
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

describe('Rescan when Platter opens', () => {
  it('is off, and the switch saves the choice', async () => {
    render(SettingsPage);

    const toggle = screen.getByRole('switch', { name: 'Rescan when Platter opens' }) as HTMLInputElement;

    expect(toggle.checked).toBe(false);
    expect(
      screen.getByText(
        'Platter opens the last scan as it was. Turn this on to scan its folder again at every start.',
      ),
    ).toBeTruthy();

    await fireEvent.click(toggle);

    expect(settings.rescanOnOpen).toBe(true);
  });
});

describe('Saved scan', () => {
  beforeEach(() => {
    api.forgetLastScan.mockReset();
  });

  it('forgets the saved scan when the backend dialog is confirmed, and says so', async () => {
    api.forgetLastScan.mockResolvedValue(undefined);
    render(SettingsPage);

    await fireEvent.click(screen.getByRole('button', { name: 'Forget' }));

    await vi.waitFor(() => expect(ui.toasts.at(-1)?.title).toBe('Saved scan forgotten'));
    expect(api.forgetLastScan).toHaveBeenCalledOnce();
  });

  it('says nothing when the user says no in the dialog', async () => {
    api.forgetLastScan.mockRejectedValue(new ApiRequestError('cancelled', 'not confirmed'));
    render(SettingsPage);

    await fireEvent.click(screen.getByRole('button', { name: 'Forget' }));
    await vi.waitFor(() => expect(api.forgetLastScan).toHaveBeenCalledOnce());
    await tick();

    expect(ui.toasts).toEqual([]);
  });

  it('says why when forgetting fails', async () => {
    api.forgetLastScan.mockRejectedValue(new ApiRequestError('io', 'permission denied'));
    render(SettingsPage);

    await fireEvent.click(screen.getByRole('button', { name: 'Forget' }));

    await vi.waitFor(() =>
      expect(ui.toasts.at(-1)).toMatchObject({ title: 'Could not forget the saved scan', kind: 'error' }),
    );
  });
});
