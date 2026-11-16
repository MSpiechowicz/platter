import { describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type Api } from '../api/client';
import { createMockApi, MOCK_VOLUMES } from '../api/mock';
import type { ScanEvent } from '../api/types';
import { findVolume, ScanStore } from './scan.svelte';

const NOW = 1_750_000_000;
const ROOT = { root: '/', oneFileSystem: true };

function setup(durationMs = 80): { api: Api; store: ScanStore } {
  const api = createMockApi({ durationMs, tickMs: 10, now: () => NOW });

  return { api, store: new ScanStore(api) };
}

const status = (store: ScanStore) => () => store.state.status;

describe('volumes', () => {
  it('loads the volume list', async () => {
    const { store } = setup();

    expect(store.volumesStatus).toBe('idle');
    await store.loadVolumes();

    expect(store.volumesStatus).toBe('ready');
    expect(store.volumes).toEqual(MOCK_VOLUMES);
  });

  it('reports a failure to list volumes', async () => {
    const api: Api = { ...createMockApi(), listVolumes: () => Promise.reject(new Error('no udev')) };
    const store = new ScanStore(api);

    await store.loadVolumes();

    expect(store.volumesStatus).toBe('error');
    expect(store.volumes).toEqual([]);
  });

  it('matches a root to its volume, exactly or by the deepest mount point above it', () => {
    expect(findVolume(MOCK_VOLUMES, '/')).toMatchObject({ exact: true, volume: { id: 'nvme0n1p2' } });
    expect(findVolume(MOCK_VOLUMES, '/mnt/backup')).toMatchObject({ exact: true, volume: { id: 'sda1' } });
    expect(findVolume(MOCK_VOLUMES, '/mnt/backup/photos')).toMatchObject({
      exact: false,
      volume: { id: 'sda1' },
    });
    expect(findVolume(MOCK_VOLUMES, '/home/demo')).toMatchObject({
      exact: false,
      volume: { id: 'nvme0n1p2' },
    });
    expect(findVolume(MOCK_VOLUMES, '/mnt/backup-old')).toMatchObject({ volume: { id: 'nvme0n1p2' } });
    expect(findVolume([], '/home')).toBeUndefined();
  });
});

describe('scan lifecycle', () => {
  it('goes idle, scanning with progress, then ready with the scan id, root and totals', async () => {
    const { api, store } = setup();
    await store.loadVolumes();

    expect(store.state).toEqual({ status: 'idle' });

    const done = store.start(ROOT);
    expect(store.state.status).toBe('scanning');

    await vi.waitFor(() => {
      expect(store.state.status === 'scanning' && store.state.progress.files > 0).toBe(true);
    });

    await vi.waitFor(() => expect(store.state.status).toBe('ready'));
    await done;

    if (store.state.status !== 'ready') {
      throw new Error('expected ready');
    }

    expect(store.state.options).toEqual(ROOT);
    expect(store.state.totals.files).toBeGreaterThan(1000);
    expect(store.state.elapsedMs).toBeGreaterThan(0);
    expect((await api.getNode(store.state.scanId, store.state.rootId)).allocated).toBe(
      store.state.totals.allocated,
    );
    expect(store.volumeInfo).toMatchObject({ exact: true, volume: { id: 'nvme0n1p2' } });
  });

  it('accepts a finished event that carries fields it does not know, and ignores them', async () => {
    const mock = createMockApi({ durationMs: 0, now: () => NOW });
    const totals = { allocated: 740, apparent: 700, files: 2000, dirs: 300, errors: 1 };
    const api: Api = {
      ...mock,
      startScan: async (_options, onEvent) => {
        // A newer backend adds fields (such as scan warnings) that this UI has no use for yet.
        onEvent({
          type: 'finished',
          scanId: 7,
          rootId: 0,
          elapsedMs: 1234,
          totals: { ...totals, warnings: ['mountTableUnavailable'] },
          somethingNew: true,
        } as ScanEvent);

        return 7;
      },
    };
    const store = new ScanStore(api);

    await store.start(ROOT);

    expect(store.state).toMatchObject({ status: 'ready', scanId: 7, rootId: 0, elapsedMs: 1234 });
    expect(store.state.status === 'ready' && store.state.totals).toMatchObject(totals);
  });

  it('cancels a running scan and ends as cancelled', async () => {
    const { store } = setup(2000);

    await store.start(ROOT);
    await vi.waitFor(() =>
      expect(store.state.status === 'scanning' && store.state.progress.files > 0).toBe(true),
    );

    const cancelled = store.cancel();
    expect(store.state).toMatchObject({ status: 'scanning', cancelling: true });

    await cancelled;
    await vi.waitFor(() => expect(status(store)()).toBe('cancelled'));
    expect(store.state).toMatchObject({ options: ROOT });
  });

  it('honours a cancel that comes before the backend has returned the scan id', async () => {
    const { store } = setup(2000);

    const started = store.start(ROOT);
    const cancelled = store.cancel();

    await Promise.all([started, cancelled]);
    await vi.waitFor(() => expect(status(store)()).toBe('cancelled'));
  });

  it('stays ready when the scan finished while the cancel was on its way', async () => {
    const mock = createMockApi({ durationMs: 30, tickMs: 5, now: () => NOW });
    const api: Api = {
      ...mock,
      // The backend finishes the scan, then finds no such scan to cancel.
      cancelScan: async () => {
        await new Promise((resolve) => setTimeout(resolve, 80));
        throw new ApiRequestError('notFound', 'No such scan: 1');
      },
    };
    const store = new ScanStore(api);

    await store.start(ROOT);
    await store.cancel();

    expect(store.state.status).toBe('ready');
  });

  it('keeps the real end of the scan when the cancel is refused after it ended', async () => {
    let emit: ((event: ScanEvent) => void) | undefined;
    const api: Api = {
      ...createMockApi(),
      startScan: async (_options, onEvent) => {
        emit = onEvent;
        return 7;
      },
      cancelScan: async () => {
        emit?.({ type: 'failed', message: 'Disk removed' });
        throw new ApiRequestError('notFound', 'No such scan: 7');
      },
    };
    const store = new ScanStore(api);

    await store.start(ROOT);
    await store.cancel();

    expect(store.state).toMatchObject({ status: 'failed', message: 'Disk removed' });
  });

  it('shows a refused cancel as a failure while the scan is still running', async () => {
    const api: Api = {
      ...createMockApi({ durationMs: 2000, tickMs: 10 }),
      cancelScan: () => Promise.reject(new ApiRequestError('io', 'Cannot stop')),
    };
    const store = new ScanStore(api);

    await store.start(ROOT);
    await store.cancel();

    expect(store.state).toMatchObject({ status: 'failed', message: 'Cannot stop' });
  });

  it('ignores a second start while one is running', async () => {
    const { store } = setup(2000);

    await store.start(ROOT);
    await store.start({ root: '/mnt/backup', oneFileSystem: false });

    expect(store.options).toEqual(ROOT);
    await store.cancel();
    await vi.waitFor(() => expect(status(store)()).toBe('cancelled'));
  });

  it('rescans the same root and frees the previous tree', async () => {
    const { api, store } = setup(30);

    await store.start(ROOT);
    await vi.waitFor(() => expect(status(store)()).toBe('ready'));

    const first = store.state.status === 'ready' ? store.state.scanId : -1;

    await store.rescan();
    await vi.waitFor(() => expect(store.state.status === 'ready' && store.state.scanId !== first).toBe(true));

    expect(store.options).toEqual(ROOT);
    await expect(api.getNode(first, 0)).rejects.toMatchObject({ code: 'notFound' });
  });

  it('goes back to idle on reset and frees the scan, but not while scanning', async () => {
    const { api, store } = setup(30);

    await store.start(ROOT);
    await vi.waitFor(() => expect(status(store)()).toBe('ready'));

    const id = store.state.status === 'ready' ? store.state.scanId : -1;

    store.reset();

    expect(store.state).toEqual({ status: 'idle' });
    await expect(api.getNode(id, 0)).rejects.toMatchObject({ code: 'notFound' });

    const slow = setup(2000);
    await slow.store.start(ROOT);
    slow.store.reset();
    expect(slow.store.state.status).toBe('scanning');
    await slow.store.cancel();
  });

  it('fails when the backend refuses to start', async () => {
    const api: Api = {
      ...createMockApi(),
      startScan: () => Promise.reject(new ApiRequestError('io', 'Permission denied')),
    };
    const store = new ScanStore(api);

    await store.start(ROOT);

    expect(store.state).toEqual({ status: 'failed', options: ROOT, message: 'Permission denied' });
  });

  it('fails when the backend reports a failure while scanning, and frees nothing twice', async () => {
    const mock = createMockApi();
    const api: Api = {
      ...mock,
      startScan: async (_options, onEvent: (event: ScanEvent) => void) => {
        queueMicrotask(() => onEvent({ type: 'failed', message: 'Disk removed' }));
        return 41;
      },
    };
    const store = new ScanStore(api);

    await store.start(ROOT);
    await vi.waitFor(() => expect(status(store)()).toBe('failed'));

    expect(store.state).toMatchObject({ message: 'Disk removed' });
  });

  it('allows a new scan after a cancelled or failed one', async () => {
    const { store } = setup(30);

    await store.start(ROOT);
    await store.cancel();
    await vi.waitFor(() => expect(status(store)()).toBe('cancelled'));

    await store.start(ROOT);
    await vi.waitFor(() => expect(status(store)()).toBe('ready'));
  });
});
