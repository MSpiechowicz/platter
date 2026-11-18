import { describe, expect, it } from 'vitest';
import { PlatformStore } from './platform.svelte';

describe('PlatformStore', () => {
  it('is Linux until told otherwise, with the trash and the slash', () => {
    const store = new PlatformStore();

    expect(store.os).toBe('linux');
    expect(store.isWindows).toBe(false);
    expect(store.trash).toBe('trash');
    expect(store.theTrash).toBe('the trash');
    expect(store.Trash).toBe('Trash');
    expect(store.separator).toBe('/');
  });

  it('names the Recycle Bin and writes backslashes on Windows', () => {
    const store = new PlatformStore();

    store.os = 'windows';

    expect(store.isWindows).toBe(true);
    expect(store.trash).toBe('Recycle Bin');
    expect(store.theTrash).toBe('the Recycle Bin');
    expect(store.Trash).toBe('Recycle Bin');
    expect(store.separator).toBe('\\');
  });

  it('names the Trash and Finder, and keeps the slash, on macOS', () => {
    const store = new PlatformStore();

    store.os = 'macos';

    expect(store.isMac).toBe(true);
    expect(store.isWindows).toBe(false);
    expect(store.trash).toBe('Trash');
    expect(store.theTrash).toBe('the Trash');
    expect(store.Trash).toBe('Trash');
    expect(store.theFileManager).toBe('Finder');
    expect(store.showInFileManager).toBe('Show in Finder');
    expect(store.separator).toBe('/');
  });
});
