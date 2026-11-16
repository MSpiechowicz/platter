import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiRequestError } from '../../api/client';
import type { AdvisorStatusDto } from '../../api/types';
import { advisor } from '../../state/advisor.svelte';
import { settings } from '../../state/settings.svelte';
import { ui } from '../../state/ui.svelte';
import { GIB, MODEL_BYTES, statusOf } from '../advisor/fixtures';
import AdvisorSettingsCard from './AdvisorSettingsCard.svelte';

const { api } = vi.hoisted(() => ({
  api: {
    configureAdvisor: vi.fn(),
    clearQuestionLog: vi.fn(),
    startModelDownload: vi.fn(),
    cancelModelDownload: vi.fn(),
  },
}));

vi.mock('../../api', () => ({ api }));

function show(status: AdvisorStatusDto): void {
  advisor.status = status;
  render(AdvisorSettingsCard);
}

const toggle = (): HTMLInputElement =>
  screen.getByRole('switch', { name: 'Use the local model' }) as HTMLInputElement;
const logToggle = (): HTMLInputElement =>
  screen.getByRole('switch', { name: 'Keep a local log of questions' }) as HTMLInputElement;

describe('AdvisorSettingsCard', () => {
  beforeEach(() => {
    api.configureAdvisor.mockReset().mockImplementation(async (options) => statusOf('onDisk', { options }));
    api.clearQuestionLog.mockReset().mockResolvedValue(undefined);
    api.startModelDownload.mockReset().mockResolvedValue(undefined);
    api.cancelModelDownload.mockReset().mockResolvedValue(undefined);
    advisor.status = undefined;
    advisor.failure = undefined;
    advisor.asking = false;
    settings.set('advisorEnabled', false);
    settings.set('advisorUnloadAfter', 'fiveMinutes');
    settings.set('advisorLogQuestions', true);
    settings.set('units', 'GB');
  });

  it('offers the download of a model that is not there, and keeps "Use the local model" off until it is', async () => {
    show(statusOf('notDownloaded'));

    expect(screen.getByText('Qwen3.5-4B · Not downloaded')).toBeTruthy();
    expect(toggle().disabled).toBe(true);

    await fireEvent.click(screen.getByRole('button', { name: 'Download Qwen3.5-4B (2.7 GB)' }));

    expect(api.startModelDownload).toHaveBeenCalledTimes(1);
  });

  it('shows "Use the local model" off while there is no model, even if it was saved on', () => {
    settings.set('advisorEnabled', true);
    show(statusOf('notDownloaded'));

    expect(toggle().checked).toBe(false);
    expect(toggle().disabled).toBe(true);
  });

  it('continues a cancelled download and says how much is there', () => {
    show(statusOf('notDownloaded', { download: { received: 400_000_000, total: MODEL_BYTES } }));

    expect(screen.getByText('Qwen3.5-4B · 400 MB of 2.7 GB downloaded')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Continue download (400 MB of 2.7 GB)' })).toBeTruthy();
  });

  it('shows a download in progress with a bar and Cancel, and no download button', async () => {
    show(statusOf('downloading', { download: { received: MODEL_BYTES / 4, total: MODEL_BYTES } }));

    expect(screen.getByText('Qwen3.5-4B · Downloading 685 MB of 2.7 GB')).toBeTruthy();
    expect(screen.getByRole('progressbar', { name: 'Download progress' }).getAttribute('aria-valuenow')).toBe(
      '25',
    );
    expect(screen.queryByRole('button', { name: /download \(/i })).toBeNull();
    expect(toggle().disabled).toBe(true);

    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(api.cancelModelDownload).toHaveBeenCalledTimes(1);
  });

  it('says nothing when the user declines the backend dialog', async () => {
    api.startModelDownload.mockRejectedValue(
      new ApiRequestError('cancelled', 'The download was not confirmed.'),
    );
    show(statusOf('notDownloaded'));

    await fireEvent.click(screen.getByRole('button', { name: 'Download Qwen3.5-4B (2.7 GB)' }));
    await tick();

    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows the message of a refused download as text', async () => {
    api.startModelDownload.mockRejectedValue(new ApiRequestError('invalid', 'A dialog is already open.'));
    show(statusOf('notDownloaded'));

    await fireEvent.click(screen.getByRole('button', { name: 'Download Qwen3.5-4B (2.7 GB)' }));

    expect((await screen.findByRole('alert')).textContent?.trim()).toBe('A dialog is already open.');
  });

  it('shows a model on disk, and lets the user switch it on and off, telling the backend each time', async () => {
    show(statusOf('onDisk'));

    expect(screen.getByText('Qwen3.5-4B · On disk (2.7 GB)')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /download/i })).toBeNull();
    expect(toggle().disabled).toBe(false);
    expect(toggle().checked).toBe(false);

    await fireEvent.click(toggle());

    expect(settings.advisorEnabled).toBe(true);
    expect(api.configureAdvisor).toHaveBeenLastCalledWith({
      enabled: true,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
    });
    expect(ui.toasts.at(-1)?.title).toBe('Saved');
  });

  it('shows a loaded model with the memory it holds', () => {
    show(statusOf('loaded', { ramBytes: 2 * GIB }));

    expect(screen.getByText('Qwen3.5-4B · Loaded (2.1 GB in RAM)')).toBeTruthy();
    expect(toggle().disabled).toBe(false);
  });

  it('chooses when the model leaves RAM, saves it and tells the backend', async () => {
    show(statusOf('onDisk'));

    const trigger = screen.getByRole('button', { name: /Unload from RAM after: 5 minutes idle/ });

    await fireEvent.click(trigger);

    const options = screen.getAllByRole('option').map((option) => option.textContent?.trim());

    expect(options).toEqual(['1 minute idle', '5 minutes idle', 'When the window closes']);

    await fireEvent.click(
      within(screen.getByRole('listbox')).getByRole('option', { name: /When the window closes/ }),
    );

    expect(settings.advisorUnloadAfter).toBe('windowClose');
    expect(api.configureAdvisor).toHaveBeenLastCalledWith({
      enabled: false,
      unloadAfter: 'windowClose',
      logQuestions: true,
    });
  });

  it('keeps a log of questions by default, says what it holds, and lets the user switch it off and on', async () => {
    show(statusOf('notDownloaded'));

    expect(screen.getByText(/never sent anywhere/)).toBeTruthy();
    expect(logToggle().checked).toBe(true);
    expect(logToggle().disabled).toBe(false);

    await fireEvent.click(logToggle());

    expect(settings.advisorLogQuestions).toBe(false);
    expect(api.configureAdvisor).toHaveBeenLastCalledWith({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: false,
    });
    expect(ui.toasts.at(-1)?.title).toBe('Saved');

    await fireEvent.click(logToggle());

    expect(settings.advisorLogQuestions).toBe(true);
    expect(api.configureAdvisor).toHaveBeenLastCalledWith(expect.objectContaining({ logQuestions: true }));
  });

  it('clears the question log and says so', async () => {
    show(statusOf('onDisk'));

    await fireEvent.click(screen.getByRole('button', { name: 'Clear log' }));
    await tick();

    expect(api.clearQuestionLog).toHaveBeenCalledTimes(1);
    expect(ui.toasts.at(-1)?.description).toBe('The question log was cleared.');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows why the question log could not be cleared as text, without a success toast', async () => {
    api.clearQuestionLog.mockRejectedValue(
      new ApiRequestError('io', 'The question log could not be removed.'),
    );
    show(statusOf('onDisk'));

    const toasts = ui.toasts.length;

    await fireEvent.click(screen.getByRole('button', { name: 'Clear log' }));

    expect((await screen.findByRole('alert')).textContent?.trim()).toBe(
      'The question log could not be removed.',
    );
    expect(ui.toasts).toHaveLength(toasts);
  });

  it('shows a problem of the backend as text', () => {
    show(statusOf('notDownloaded', { problem: 'The file did not match its checksum.' }));

    expect(screen.getByRole('alert').textContent?.trim()).toBe('The file did not match its checksum.');
  });

  it('shows a problem in the same notice as the Model card, and keeps the progress row it has room for', () => {
    show(statusOf('notDownloaded', { problem: 'The file did not match its checksum.' }));

    expect(screen.getByRole('alert').classList.contains('notice')).toBe(true);
    expect(document.querySelectorAll('.prog')).toHaveLength(1);
  });

  it('keeps the row of the progress bar in every state, so the card does not move when a download starts', () => {
    const { unmount } = render(AdvisorSettingsCard);

    for (const status of [
      statusOf('notDownloaded'),
      statusOf('downloading', { download: { received: MODEL_BYTES / 4, total: MODEL_BYTES } }),
      statusOf('onDisk'),
      statusOf('loaded'),
    ]) {
      advisor.status = status;
      flushSync();

      expect(document.querySelectorAll('.prog')).toHaveLength(1);
      expect(screen.queryByRole('progressbar') !== null).toBe(status.state === 'downloading');
    }

    unmount();
  });

  it('says what it is doing before the backend has answered, and where there is no runtime', () => {
    show(statusOf('unavailable'));

    expect(screen.getByText('Not available in this build.')).toBeTruthy();
    expect(toggle().disabled).toBe(true);
  });

  it('has no choice of model: one Model row with the state and the download button', () => {
    show(statusOf('notDownloaded'));

    expect(screen.getByText('Model')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /^Model: / })).toBeNull();
    expect(
      screen
        .getAllByRole('button')
        .map((button) => button.getAttribute('aria-label') ?? button.textContent?.trim()),
    ).toEqual(['Download Qwen3.5-4B (2.7 GB)', 'Unload from RAM after: 5 minutes idle', 'Clear log']);
  });
});
