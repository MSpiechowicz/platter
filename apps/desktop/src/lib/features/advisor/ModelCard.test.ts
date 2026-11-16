import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import type { Api } from '../../api/client';
import type { AdvisorStatusDto } from '../../api/types';
import { AdvisorStore } from '../../state/advisor.svelte';
import { SettingsStore } from '../../state/settings.svelte';
import { GIB, MODEL_BYTES, statusOf } from './fixtures';
import ModelCard from './ModelCard.svelte';

function setup(status: AdvisorStatusDto | undefined, enabled = true) {
  const api = { cancelModelDownload: vi.fn().mockResolvedValue(undefined) } as unknown as Api;
  const settings = new SettingsStore(null);

  settings.set('advisorEnabled', enabled);

  const store = new AdvisorStore(api, settings);

  render(ModelCard, { props: { status, store } });

  return { api, store };
}

const meter = (): HTMLElement | null => screen.queryByRole('meter');

describe('ModelCard', () => {
  it('shows the name, the format, the size on disk and when it unloads', () => {
    setup(statusOf('onDisk'));

    expect(screen.getByText('Qwen3.5-4B')).toBeTruthy();
    expect(screen.getByText('GGUF · Q4_K_M')).toBeTruthy();
    expect(screen.getByText('2.7 GB')).toBeTruthy();
    expect(screen.getByText('5 min idle')).toBeTruthy();
    expect(screen.getByText('—')).toBeTruthy();
    expect(meter()).toBeNull();
  });

  it('shows the speed once it is known, and the RAM bar only while loaded', () => {
    setup(statusOf('loaded', { ramBytes: 2 * GIB, tokensPerSecond: 36.5 }));

    expect(screen.getByText('37 tokens/s')).toBeTruthy();
    expect(screen.getByText('2.1 GB of 34 GB RAM in use')).toBeTruthy();
    expect(meter()?.getAttribute('aria-valuenow')).toBe('6');
  });

  it('shows the progress of a download with Cancel', async () => {
    const { api } = setup(
      statusOf('downloading', { download: { received: MODEL_BYTES / 2, total: MODEL_BYTES } }),
    );

    expect(screen.getByText('1.4 GB of 2.7 GB')).toBeTruthy();
    expect(screen.getByRole('progressbar', { name: 'Download progress' }).getAttribute('aria-valuenow')).toBe(
      '50',
    );

    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(api.cancelModelDownload).toHaveBeenCalledTimes(1);
  });

  it('says a cancelled download continues where it stopped', () => {
    setup(statusOf('notDownloaded', { download: { received: 400_000_000, total: MODEL_BYTES } }));

    expect(
      screen.getByText('400 MB of 2.7 GB received. The download continues where it stopped.'),
    ).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull();
  });

  it('says what is the matter with a model that is off, not there or not available', () => {
    setup(statusOf('onDisk'), false);
    expect(screen.getByText('The local model is switched off in Settings.')).toBeTruthy();
  });

  it('shows a problem of the backend as text', () => {
    setup(statusOf('notDownloaded', { problem: 'The file did not match its checksum <script>.' }));

    expect(screen.getByRole('alert').textContent).toContain('The file did not match its checksum <script>.');
    expect(document.querySelector('script')).toBeNull();
  });

  it('keeps a problem in the status slot, in one line with the whole text in a tooltip, so the card does not grow', () => {
    const problem = 'The file did not match its checksum. '.repeat(8).trim();
    const { store } = setup(statusOf('onDisk'));

    store.failure = undefined;
    cleanup();
    render(ModelCard, { props: { status: statusOf('onDisk', { problem }), store } });

    const slot = document.querySelector('.slot') as HTMLElement;
    const alert = screen.getByRole('alert');

    expect(slot.contains(alert)).toBe(true);
    expect(slot.children).toHaveLength(1);
    expect(alert.querySelector('[title]')?.getAttribute('title')).toBe(problem);
    expect(screen.queryByText('Not in RAM. It loads when it is needed.')).toBeNull();
  });

  it('keeps the progress and Cancel while a download runs, whatever else is wrong', () => {
    setup(
      statusOf('downloading', {
        download: { received: 100, total: MODEL_BYTES },
        problem: 'An old problem.',
      }),
    );

    expect(screen.getByRole('progressbar')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeTruthy();
  });

  it('shows what the last action failed with, in the same place', () => {
    const { store } = setup(statusOf('onDisk'));

    expect(screen.queryByRole('alert')).toBeNull();
    store.failure = 'The local model is switched off in the settings.';

    return tick().then(() => {
      expect(document.querySelector('.slot')?.contains(screen.getByRole('alert'))).toBe(true);
    });
  });

  it('always discloses that the download is the only network call', () => {
    for (const state of ['notDownloaded', 'downloading', 'onDisk', 'loaded'] as const) {
      const { unmount } = render(ModelCard, {
        props: { status: statusOf(state), store: new AdvisorStore({} as Api, new SettingsStore(null)) },
      });

      expect(screen.getByText("The model download is Platter's only network call.")).toBeTruthy();
      unmount();
    }
  });

  it('says it is checking before the backend has answered', () => {
    setup(undefined);

    expect(screen.getByText('Checking the model…')).toBeTruthy();
  });
});
