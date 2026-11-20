import { describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type Api } from '../api/client';
import type { DiskAccess } from '../api/types';
import { createMockApi } from '../api/mock';
import { DiskAccessStore } from './access.svelte';

function setup(answers: DiskAccess[] = ['missing']) {
  const toasts: { title: string; description: string }[] = [];
  const api = {
    diskAccess: vi.fn(async (): Promise<DiskAccess> => answers.shift() ?? 'granted'),
    openDiskAccessSettings: vi.fn(async () => {}),
  };
  const store = new DiskAccessStore({
    api: api as unknown as Api,
    ui: {
      toast: (title, description) => {
        toasts.push({ title, description: description ?? '' });

        return toasts.length;
      },
    },
  });

  return { store, api, toasts };
}

describe('DiskAccessStore', () => {
  it('shows nothing until the check has answered', () => {
    const { store } = setup();

    expect(store.status).toBe('unknown');
    expect(store.shown).toBe(false);
  });

  it('shows the notice while the access is missing, and only then', async () => {
    const { store } = setup(['missing', 'granted', 'unknown']);

    await store.check();
    expect(store.shown).toBe(true);

    await store.check();
    expect(store.shown).toBe(false);

    await store.check();
    expect(store.shown).toBe(false);
  });

  it('keeps what it knew when a check fails', async () => {
    const { store, api } = setup(['missing']);

    await store.check();
    api.diskAccess.mockRejectedValueOnce(new ApiRequestError('io', 'no'));
    await store.check();

    expect(store.status).toBe('missing');
  });

  it('checks again each time the window gets the focus back, and stops when asked', async () => {
    const { store, api } = setup(['missing', 'granted']);
    const target = new EventTarget();

    const stop = store.start(target as unknown as Window);

    await vi.waitFor(() => expect(store.status).toBe('missing'));

    target.dispatchEvent(new Event('focus'));
    await vi.waitFor(() => expect(store.status).toBe('granted'));

    stop();
    target.dispatchEvent(new Event('focus'));

    expect(api.diskAccess).toHaveBeenCalledTimes(2);
  });

  it('drops an answer that a newer check overtook', async () => {
    const { store, api } = setup();
    let answerFirst: (access: DiskAccess) => void = () => {};

    api.diskAccess.mockImplementationOnce(() => new Promise((resolve) => (answerFirst = resolve)));
    api.diskAccess.mockResolvedValueOnce('granted');

    const first = store.check();
    await store.check();
    answerFirst('missing');
    await first;

    expect(store.status).toBe('granted');
  });

  it('checks again when the window is shown again', async () => {
    const { store, api } = setup(['missing', 'granted']);
    const target = new EventTarget();

    store.start(target as unknown as Window);
    await vi.waitFor(() => expect(store.status).toBe('missing'));

    target.dispatchEvent(new Event('visibilitychange'));
    await vi.waitFor(() => expect(store.status).toBe('granted'));
    expect(api.diskAccess).toHaveBeenCalledTimes(2);
  });

  it('a second start replaces the first, so one focus is one check', async () => {
    const { store, api } = setup(['missing', 'missing', 'missing']);
    const first = new EventTarget();
    const second = new EventTarget();

    store.start(first as unknown as Window);
    store.start(second as unknown as Window);
    await vi.waitFor(() => expect(api.diskAccess).toHaveBeenCalledTimes(2));

    first.dispatchEvent(new Event('focus'));
    second.dispatchEvent(new Event('focus'));

    expect(api.diskAccess).toHaveBeenCalledTimes(3);
  });

  it('"Not now" hides the notice although the access is still missing', async () => {
    const { store } = setup(['missing', 'missing']);

    await store.check();
    store.later();
    await store.check();

    expect(store.status).toBe('missing');
    expect(store.shown).toBe(false);
  });

  it('says so when System Settings cannot be opened', async () => {
    const { store, api, toasts } = setup();

    api.openDiskAccessSettings.mockRejectedValueOnce(
      new ApiRequestError('io', 'cannot open System Settings: no'),
    );
    await store.openSettings();

    expect(toasts).toEqual([
      { title: 'System Settings could not be opened', description: expect.any(String) },
    ]);
  });
});

describe('the mock Full Disk Access', () => {
  it('is granted unless asked otherwise', async () => {
    expect(await createMockApi().diskAccess()).toBe('granted');
  });

  it('is granted once the settings were opened on a Mac', async () => {
    const api = createMockApi({ os: 'macos', access: 'missing' });

    expect(await api.diskAccess()).toBe('missing');
    await api.openDiskAccessSettings();
    expect(await api.diskAccess()).toBe('granted');
  });

  it('refuses to open the settings outside macOS, as the app does', async () => {
    await expect(createMockApi({ access: 'missing' }).openDiskAccessSettings()).rejects.toMatchObject({
      code: 'invalid',
    });
  });
});
