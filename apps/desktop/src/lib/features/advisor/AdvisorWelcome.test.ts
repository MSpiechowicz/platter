import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Api } from '../../api/client';
import type { AdvisorStatusDto } from '../../api/types';
import { AdvisorStore } from '../../state/advisor.svelte';
import { SettingsStore, settings } from '../../state/settings.svelte';
import { ui } from '../../state/ui.svelte';
import { MODEL_BYTES, statusOf } from './fixtures';
import AdvisorWelcome from './AdvisorWelcome.svelte';

function setup(status: AdvisorStatusDto, failure?: string) {
  const api = {
    startModelDownload: vi.fn().mockResolvedValue(undefined),
    cancelModelDownload: vi.fn().mockResolvedValue(undefined),
  } as unknown as Api;
  const store = new AdvisorStore(api, new SettingsStore(null));

  // The download only starts from a status that is not downloaded.
  store.status = status;
  store.failure = failure;

  render(AdvisorWelcome, { props: { status, store } });

  return { api, store };
}

const illustration = (): Element => screen.getByRole('img', { name: /stack of disks/ });

describe('AdvisorWelcome', () => {
  beforeEach(() => {
    settings.set('units', 'GB');
    settings.set('reduceMotion', false);
    ui.go('scan');
  });

  it('says what the Advisor is, what it costs and what stays on the machine', () => {
    setup(statusOf('notDownloaded'));

    expect(screen.getByRole('heading', { name: 'Download the local model to use the Advisor' })).toBeTruthy();
    expect(screen.getByText(/needs about 4\.8 GB of free RAM while in use/)).toBeTruthy();

    for (const text of [
      'Offline',
      'Runs on this machine',
      '2.7 GB',
      'Qwen3.5-4B',
      'Your files stay here',
      'Nothing is uploaded',
    ]) {
      expect(screen.getByText(text)).toBeTruthy();
    }

    expect(screen.getByText(/The download is Platter's only network call\./)).toBeTruthy();
  });

  it('writes the sizes in the unit of the settings', () => {
    settings.set('units', 'GiB');
    setup(statusOf('notDownloaded'));

    expect(screen.getByRole('button', { name: /^Download Qwen3\.5-4B \(2\.\d GiB\)$/ })).toBeTruthy();
    expect(screen.getByText(/^2\.\d GiB$/)).toBeTruthy();
  });

  it('asks for the download once per press, with no web dialog of its own', async () => {
    const { api } = setup(statusOf('notDownloaded'));

    await fireEvent.click(screen.getByRole('button', { name: 'Download Qwen3.5-4B (2.7 GB)' }));

    expect(api.startModelDownload).toHaveBeenCalledTimes(1);
  });

  it('offers to continue a partial download, with what has arrived', () => {
    setup(statusOf('notDownloaded', { download: { received: MODEL_BYTES * 0.4, total: MODEL_BYTES } }));

    expect(screen.getByRole('button', { name: 'Continue download (1.1 GB of 2.7 GB)' })).toBeTruthy();
  });

  it('opens Settings from the text link', async () => {
    setup(statusOf('notDownloaded'));

    await fireEvent.click(screen.getByRole('button', { name: 'Open Settings' }));

    expect(ui.page).toBe('settings');
  });

  it('shows the progress with a Cancel button in place of the download button', async () => {
    const { api } = setup(
      statusOf('downloading', { download: { received: MODEL_BYTES * 0.4, total: MODEL_BYTES } }),
    );

    expect(screen.getByRole('heading', { name: 'Downloading the local model for the Advisor' })).toBeTruthy();
    expect(screen.getByText('1.1 GB of 2.7 GB · 40%')).toBeTruthy();
    expect(screen.getByRole('progressbar', { name: 'Download progress' }).getAttribute('aria-valuenow')).toBe(
      '40',
    );
    expect(screen.queryByRole('button', { name: /^Download|^Continue/ })).toBeNull();

    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(api.cancelModelDownload).toHaveBeenCalledTimes(1);
  });

  it('says a download is starting while no bytes have arrived', () => {
    setup(statusOf('downloading'));

    expect(screen.getByText('Starting the download…')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeTruthy();
  });

  it('shows a problem above the button, and the button continues the partial download', () => {
    setup(
      statusOf('notDownloaded', {
        download: { received: MODEL_BYTES * 0.4, total: MODEL_BYTES },
        problem: 'The connection dropped.',
      }),
    );

    expect(screen.getByRole('alert').textContent).toContain('The connection dropped.');
    expect(screen.getByRole('button', { name: /^Continue download/ })).toBeTruthy();
  });

  it('shows a problem with the normal label when nothing was received', () => {
    setup(statusOf('notDownloaded'), 'No space left on the disk.');

    expect(screen.getByRole('alert').textContent).toContain('No space left on the disk.');
    expect(screen.getByRole('button', { name: 'Download Qwen3.5-4B (2.7 GB)' })).toBeTruthy();
  });

  it('makes hidden characters of a problem visible', () => {
    setup(statusOf('notDownloaded', { problem: 'bad‮text\u0007' }));

    expect(screen.getByRole('alert').textContent).toContain('bad\\u{202E}text\\u{7}');
  });

  it('plays the illustration slowly while the model waits to be downloaded', () => {
    setup(statusOf('notDownloaded'));

    expect(illustration().classList.contains('run')).toBe(true);
    expect(illustration().classList.contains('fast')).toBe(false);
  });

  it('runs the flow faster while downloading', () => {
    setup(statusOf('downloading', { download: { received: MODEL_BYTES / 2, total: MODEL_BYTES } }));

    expect(illustration().classList.contains('run')).toBe(true);
    expect(illustration().classList.contains('fast')).toBe(true);
  });

  it('drives the badge ring from the same share as the progress bar', () => {
    setup(statusOf('downloading', { download: { received: MODEL_BYTES / 2, total: MODEL_BYTES } }));

    const ring = illustration().querySelector('.ring') as SVGElement;
    const circumference = 2 * Math.PI * 15;

    expect(parseFloat(ring.style.strokeDasharray)).toBeCloseTo(circumference / 2, 1);
  });

  it('stands still on a failed download: no looping motion, and a warning mark in the badge', () => {
    setup(statusOf('notDownloaded', { problem: 'It failed.' }));

    expect(illustration().classList.contains('run')).toBe(false);
    expect(illustration().querySelector('.badge-warn')).toBeTruthy();
    expect(illustration().querySelector('.mark')).toBeTruthy();
  });

  it('draws no animation at all when motion is reduced', () => {
    settings.set('reduceMotion', true);
    setup(statusOf('notDownloaded'));

    const classes = ['enter', 'run', 'fast'];

    for (const name of classes) {
      expect(illustration().classList.contains(name)).toBe(false);
    }

    expect(document.querySelector('.enter-text')).toBeNull();
  });

  it('shows the download only in the badge: the disk layers do not change with it', () => {
    setup(statusOf('downloading', { download: { received: MODEL_BYTES * 0.5, total: MODEL_BYTES } }));
    const atHalf = illustration().querySelector('g.disk')?.outerHTML;

    cleanup();
    setup(statusOf('downloading', { download: { received: MODEL_BYTES * 0.9, total: MODEL_BYTES } }));

    expect(illustration().querySelector('g.disk')?.outerHTML).toBe(atHalf);
  });
});
