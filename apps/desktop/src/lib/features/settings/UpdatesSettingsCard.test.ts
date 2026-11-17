import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError } from '../../api/client';
import type { UpdateCheck } from '../../api/types';
import { settings } from '../../state/settings.svelte';
import { ui } from '../../state/ui.svelte';
import { updater } from '../../state/update.svelte';
import UpdatesSettingsCard from './UpdatesSettingsCard.svelte';

const { api } = vi.hoisted(() => ({ api: { checkUpdate: vi.fn(), openExternal: vi.fn() } }));

vi.mock('../../api', () => ({ api }));

const OFFER: UpdateCheck = {
  current: '0.1.0',
  update: { version: '0.2.0', notes: [], sizeBytes: 14_000_000, canInstall: true },
};

const checkNow = (): HTMLButtonElement =>
  screen.getByRole('button', { name: 'Check now' }) as HTMLButtonElement;

describe('UpdatesSettingsCard', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    api.checkUpdate.mockReset().mockResolvedValue({ current: '0.1.0', update: null });
    updater.info = null;
    updater.phase = 'available';
    updater.result = null;
    updater.current = null;
    updater.canInstall = null;
    updater.checkedAt = null;
    updater.checking = false;
    settings.set('updateCheck', 'daily');
    ui.toasts = [];
  });

  afterEach(() => vi.useRealTimers());

  it('says nothing was checked before the first check', () => {
    render(UpdatesSettingsCard);

    expect(screen.getByText('Platter')).toBeTruthy();
    expect(screen.getByText('Not checked yet.')).toBeTruthy();
  });

  it('says when the last check of an earlier session ran', () => {
    updater.checkedAt = Date.now() - 3 * 3_600_000;
    render(UpdatesSettingsCard);

    expect(screen.getByText('Last checked 3 hours ago.')).toBeTruthy();
  });

  it('shows a spinner for at least a second, and only then the result', async () => {
    api.checkUpdate.mockImplementation(
      () => new Promise((resolve) => setTimeout(() => resolve({ current: '0.1.0', update: null }), 200)),
    );
    render(UpdatesSettingsCard);

    await fireEvent.click(checkNow());
    await vi.advanceTimersByTimeAsync(10);

    expect(screen.getByRole('status', { name: 'Checking for updates' })).toBeTruthy();
    expect(screen.getByText('Checking for updates…')).toBeTruthy();
    const button = screen.getByRole('button', { name: /Checking…/ }) as HTMLButtonElement;
    expect(within(button).getByRole('status', { name: 'Checking for updates' })).toBeTruthy();
    expect(button.disabled).toBe(true);
    expect(api.checkUpdate).toHaveBeenCalledTimes(1);

    // The answer is in after 200 ms, and the spinner stays.
    await vi.advanceTimersByTimeAsync(500);

    expect(screen.queryByText('Platter is up to date.')).toBeNull();
    expect(screen.getByRole('status', { name: 'Checking for updates' })).toBeTruthy();

    await vi.advanceTimersByTimeAsync(600);
    await tick();

    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.getByText('Platter is up to date.')).toBeTruthy();
    expect(screen.getByText('Platter 0.1.0')).toBeTruthy();
    expect(checkNow().disabled).toBe(false);
  });

  it('turns the button into a spinner for at least a second, also when the answer is at once', async () => {
    render(UpdatesSettingsCard);

    await fireEvent.click(checkNow());
    await tick();

    expect(screen.getByRole('button', { name: /Checking…/ })).toBeTruthy();

    await vi.advanceTimersByTimeAsync(900);
    expect(screen.queryByText('Platter is up to date.')).toBeNull();

    await vi.advanceTimersByTimeAsync(200);
    await tick();

    expect(checkNow().disabled).toBe(false);
    expect(screen.getByText('Platter is up to date.')).toBeTruthy();
  });

  it('says which version is available', async () => {
    api.checkUpdate.mockResolvedValue(OFFER);
    render(UpdatesSettingsCard);

    await fireEvent.click(checkNow());
    await vi.advanceTimersByTimeAsync(1500);

    expect(screen.getByText('Version 0.2.0 is available.')).toBeTruthy();
  });

  it('shows the error of a check as text, and lets the user try again', async () => {
    api.checkUpdate.mockRejectedValueOnce(new ApiRequestError('io', 'Updates could not be checked: offline'));
    render(UpdatesSettingsCard);

    await fireEvent.click(checkNow());
    await vi.advanceTimersByTimeAsync(1500);

    expect(screen.getByText('Updates could not be checked: offline')).toBeTruthy();
    expect(checkNow().disabled).toBe(false);

    await fireEvent.click(checkNow());
    await vi.advanceTimersByTimeAsync(1500);

    expect(screen.getByText('Platter is up to date.')).toBeTruthy();
  });

  it('offers every 6 hours, daily, weekly and never, and saves the choice', async () => {
    render(UpdatesSettingsCard);

    await fireEvent.click(screen.getByRole('button', { name: /Check for updates: Daily/ }));

    expect(screen.getAllByRole('option').map((option) => option.textContent?.trim())).toEqual([
      'Every 6 hours',
      'Daily',
      'Weekly',
      'Never',
    ]);

    await fireEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Never' }));

    expect(settings.updateCheck).toBe('never');
    expect(ui.toasts.at(-1)?.title).toBe('Saved');
  });

  it('says that a package manager installs updates only when the last found update says so', () => {
    const { unmount } = render(UpdatesSettingsCard);

    expect(screen.queryByText(/package manager/)).toBeNull();
    unmount();

    updater.canInstall = true;
    render(UpdatesSettingsCard);
    expect(screen.queryByText(/package manager/)).toBeNull();
    cleanupCard();

    updater.canInstall = false;
    render(UpdatesSettingsCard);

    expect(
      screen.getByText(
        'A package manager installs updates for this copy of Platter. Platter tells you when one is out.',
      ),
    ).toBeTruthy();
  });
});

function cleanupCard(): void {
  document.body.innerHTML = '';
}
