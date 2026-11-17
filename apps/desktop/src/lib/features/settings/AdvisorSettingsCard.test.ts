import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
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
    clearGpuLog: vi.fn(),
    advisorStatus: vi.fn(),
    startModelDownload: vi.fn(),
    cancelModelDownload: vi.fn(),
    loadModel: vi.fn(),
    unloadModel: vi.fn(),
  },
}));

vi.mock('../../api', () => ({ api }));

function show(status: AdvisorStatusDto): void {
  advisor.status = status;
  render(AdvisorSettingsCard);
}

const toggle = (): HTMLInputElement =>
  screen.getByRole('switch', { name: 'Use the local model' }) as HTMLInputElement;
const gpuToggle = (): HTMLInputElement =>
  screen.getByRole('switch', { name: 'Use the graphics card' }) as HTMLInputElement;
const logToggle = (): HTMLInputElement =>
  screen.getByRole('switch', { name: 'Keep a local log of questions' }) as HTMLInputElement;

describe('AdvisorSettingsCard', () => {
  beforeEach(() => {
    api.configureAdvisor.mockReset().mockImplementation(async (options) => statusOf('onDisk', { options }));
    api.clearQuestionLog.mockReset().mockResolvedValue(undefined);
    api.clearGpuLog.mockReset().mockResolvedValue(undefined);
    api.advisorStatus.mockReset().mockResolvedValue(statusOf('onDisk'));
    api.startModelDownload.mockReset().mockResolvedValue(undefined);
    api.cancelModelDownload.mockReset().mockResolvedValue(undefined);
    advisor.status = undefined;
    advisor.failure = undefined;
    advisor.asking = false;
    settings.set('advisorEnabled', false);
    settings.set('advisorUnloadAfter', 'fiveMinutes');
    settings.set('advisorLogQuestions', true);
    settings.set('advisorUseGpu', true);
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

  it('shows a download in progress in the row and Cancel, and no download button', async () => {
    show(statusOf('downloading', { download: { received: MODEL_BYTES / 4, total: MODEL_BYTES } }));

    expect(screen.getByText('Qwen3.5-4B · Downloading 685 MB of 2.7 GB')).toBeTruthy();
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
      useGpu: true,
    });
    expect(ui.toasts.at(-1)?.title).toBe('Saved');
  });

  it('shows a loaded model with the memory it holds', () => {
    show(statusOf('loaded', { ramBytes: 2 * GIB }));

    expect(screen.getByText('Qwen3.5-4B · Loaded (2.1 GB in RAM)')).toBeTruthy();
    expect(toggle().disabled).toBe(false);
  });

  it('shows the video memory in place of RAM while the model is loaded on a graphics card', () => {
    show(
      statusOf('loaded', {
        ramBytes: 2 * GIB,
        device: { kind: 'gpu', name: 'AMD Radeon RX 7900 XTX (RADV NAVI31)' },
        videoBytes: 3_200_000_000,
      }),
    );

    expect(screen.getByText('Qwen3.5-4B · Loaded (3.2 GB video memory)')).toBeTruthy();
  });

  it('offers "Use the graphics card" on, with its hint, and tells the backend when it is switched off and on', async () => {
    show(statusOf('onDisk'));

    expect(
      screen.getByText(
        'Faster when a graphics card with enough memory is there; Platter falls back to the processor.',
      ),
    ).toBeTruthy();
    expect(gpuToggle().checked).toBe(true);
    expect(gpuToggle().disabled).toBe(false);

    await fireEvent.click(gpuToggle());

    expect(settings.advisorUseGpu).toBe(false);
    expect(api.configureAdvisor).toHaveBeenLastCalledWith({
      enabled: false,
      unloadAfter: 'fiveMinutes',
      logQuestions: true,
      useGpu: false,
    });
    expect(ui.toasts.at(-1)?.title).toBe('Saved');

    await fireEvent.click(gpuToggle());

    expect(settings.advisorUseGpu).toBe(true);
    expect(api.configureAdvisor).toHaveBeenLastCalledWith(expect.objectContaining({ useGpu: true }));
  });

  it('has no graphics card switch in a build that cannot use one', () => {
    show(statusOf('onDisk', { gpuSupported: false }));

    expect(screen.queryByRole('switch', { name: 'Use the graphics card' })).toBeNull();
    expect(screen.queryByText(/graphics card/)).toBeNull();
  });

  it('shows the fallback line of the backend as a problem', () => {
    show(
      statusOf('loaded', {
        device: { kind: 'cpu' },
        problem: 'The graphics card could not run the model; it runs on the processor.',
      }),
    );

    expect(screen.getByRole('alert').textContent).toContain(
      'The graphics card could not run the model; it runs on the processor.',
    );
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
      useGpu: true,
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
      useGpu: true,
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

  it('shows a problem in the same notice as the Model card', () => {
    show(statusOf('notDownloaded', { problem: 'The file did not match its checksum.' }));

    expect(screen.getByRole('alert').classList.contains('notice')).toBe(true);
  });

  it('loads and unloads the model from the Model row, like the Advisor page header', async () => {
    settings.advisorEnabled = true;
    show(statusOf('onDisk'));

    await fireEvent.click(screen.getByRole('button', { name: 'Load model' }));
    expect(api.loadModel).toHaveBeenCalledTimes(1);

    cleanup();
    show(statusOf('loaded'));
    await fireEvent.click(screen.getByRole('button', { name: 'Unload model' }));
    expect(api.unloadModel).toHaveBeenCalledTimes(1);
  });

  it('cannot load while the model loads or is switched off', () => {
    settings.advisorEnabled = true;
    show(statusOf('loading'));
    expect(screen.getByText('Qwen3.5-4B · Loading…')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Load model' }) as HTMLButtonElement).disabled).toBe(true);

    cleanup();
    settings.advisorEnabled = false;
    show(statusOf('onDisk'));
    expect((screen.getByRole('button', { name: 'Load model' }) as HTMLButtonElement).disabled).toBe(true);
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

  describe('the last graphics card problem', () => {
    const at = new Date(2025, 2, 12, 14, 5).getTime();
    const last = (what: 'crash' | 'hang' | 'error') => ({
      id: at,
      at,
      what,
      card: 'AMD Radeon RX 7900 XTX (RADV NAVI31)',
    });

    it('is a row with the local date, the time and what happened, and a Clear button', () => {
      const lines = { crash: 'crashed', hang: 'stopped responding', error: 'error' } as const;

      for (const what of ['crash', 'hang', 'error'] as const) {
        show(statusOf('onDisk', { lastGpuProblem: last(what) }));

        expect(screen.getByText('Last graphics card problem')).toBeTruthy();
        expect(screen.getByText(`12 Mar 2025, 14:05 \u00b7 ${lines[what]}`)).toBeTruthy();
        expect(screen.getByRole('button', { name: 'Clear the graphics card problem log' }).textContent).toBe(
          'Clear',
        );

        cleanup();
      }
    });

    it('is not there without a problem in the log, also when one waits for its answer', () => {
      show(statusOf('onDisk'));
      expect(screen.queryByText('Last graphics card problem')).toBeNull();
      expect(screen.queryByRole('button', { name: 'Clear the graphics card problem log' })).toBeNull();

      cleanup();
      show(statusOf('onDisk', { gpuProblem: last('hang') }));
      expect(screen.queryByText('Last graphics card problem')).toBeNull();
    });

    it('comes after "Use the graphics card" and before the question log', () => {
      show(statusOf('onDisk', { lastGpuProblem: last('crash') }));

      const text = document.body.textContent ?? '';

      expect(text.indexOf('Use the graphics card')).toBeLessThan(text.indexOf('Last graphics card problem'));
      expect(text.indexOf('Last graphics card problem')).toBeLessThan(
        text.indexOf('Keep a local log of questions'),
      );
    });

    it('clears the log, says so, and the row goes once the backend has none', async () => {
      show(statusOf('onDisk', { lastGpuProblem: last('crash') }));

      await fireEvent.click(screen.getByRole('button', { name: 'Clear the graphics card problem log' }));

      await vi.waitFor(() =>
        expect(ui.toasts.at(-1)?.description).toBe('The graphics card problem log was cleared.'),
      );

      expect(api.clearGpuLog).toHaveBeenCalledTimes(1);
      expect(screen.queryByText('Last graphics card problem')).toBeNull();
    });

    it('shows why the log could not be cleared as text, keeps the row, and shows no success toast', async () => {
      api.clearGpuLog.mockRejectedValue(
        new ApiRequestError('io', 'The graphics card problem log could not be removed.'),
      );
      show(statusOf('onDisk', { lastGpuProblem: last('crash') }));

      const toasts = ui.toasts.length;

      await fireEvent.click(screen.getByRole('button', { name: 'Clear the graphics card problem log' }));

      expect((await screen.findByRole('alert')).textContent?.trim()).toBe(
        'The graphics card problem log could not be removed.',
      );
      expect(ui.toasts).toHaveLength(toasts);
      expect(screen.getByText('Last graphics card problem')).toBeTruthy();
    });
  });
});
