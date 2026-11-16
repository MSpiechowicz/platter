import { describe, expect, it } from 'vitest';
import { SETTINGS_KEY, SettingsStore } from './settings.svelte';

class MemoryStorage {
  data = new Map<string, string>();

  getItem(key: string): string | null {
    return this.data.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.data.set(key, value);
  }
}

describe('SettingsStore', () => {
  it('starts with the defaults', () => {
    const settings = new SettingsStore(new MemoryStorage());

    expect(settings.units).toBe('GB');
    expect(settings.reduceMotion).toBe(false);
    expect(settings.oneFileSystem).toBe(true);
    expect(settings.sidebarCollapsed).toBe(false);
    expect(settings.explorerView).toBe('flat');
  });

  it('saves a change and restores it in a new store', () => {
    const storage = new MemoryStorage();
    const first = new SettingsStore(storage);

    first.set('units', 'GiB');
    first.set('reduceMotion', true);
    first.set('oneFileSystem', false);
    first.set('sidebarCollapsed', true);
    first.set('explorerView', '3d');

    const second = new SettingsStore(storage);

    expect(second.units).toBe('GiB');
    expect(second.reduceMotion).toBe(true);
    expect(second.oneFileSystem).toBe(false);
    expect(second.sidebarCollapsed).toBe(true);
    expect(second.explorerView).toBe('3d');
  });

  it('ignores values of the wrong type and keeps the defaults for them', () => {
    const storage = new MemoryStorage();

    storage.setItem(
      SETTINGS_KEY,
      JSON.stringify({
        units: 'TB',
        reduceMotion: 'yes',
        oneFileSystem: false,
        explorerView: 'cube',
        extra: 1,
      }),
    );

    const settings = new SettingsStore(storage);

    expect(settings.units).toBe('GB');
    expect(settings.reduceMotion).toBe(false);
    expect(settings.oneFileSystem).toBe(false);
    expect(settings.explorerView).toBe('flat');
  });

  it('survives a corrupt save', () => {
    const storage = new MemoryStorage();

    storage.setItem(SETTINGS_KEY, '{not json');

    expect(new SettingsStore(storage).units).toBe('GB');
  });

  it('keeps working when storage is missing or throws', () => {
    const throwing = {
      getItem: (): string | null => {
        throw new Error('blocked');
      },
      setItem: (): void => {
        throw new Error('quota');
      },
    };

    for (const storage of [null, throwing]) {
      const settings = new SettingsStore(storage);

      settings.set('units', 'GiB');

      expect(settings.units).toBe('GiB');
    }
  });
});
