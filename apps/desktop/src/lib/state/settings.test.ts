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

  it('keeps the advisor off until the model is downloaded', () => {
    const settings = new SettingsStore(new MemoryStorage());

    expect(settings.advisorEnabled).toBe(false);
    expect(settings.advisorUnloadAfter).toBe('fiveMinutes');
  });

  it('keeps the question log on by default', () => {
    expect(new SettingsStore(new MemoryStorage()).advisorLogQuestions).toBe(true);
  });

  it('has the graphics card on by default, also for a save from before the option existed', () => {
    expect(new SettingsStore(new MemoryStorage()).advisorUseGpu).toBe(true);

    const storage = new MemoryStorage();

    storage.setItem(
      SETTINGS_KEY,
      JSON.stringify({ advisorEnabled: true, advisorUnloadAfter: 'oneMinute', advisorLogQuestions: false }),
    );

    const settings = new SettingsStore(storage);

    expect(settings.advisorUseGpu).toBe(true);
    expect(settings.advisorEnabled).toBe(true);
    expect(settings.advisorLogQuestions).toBe(false);

    settings.set('reduceMotion', true);

    expect(JSON.parse(storage.getItem(SETTINGS_KEY) ?? '{}')).toMatchObject({ advisorUseGpu: true });
  });

  it('saves a change and restores it in a new store', () => {
    const storage = new MemoryStorage();
    const first = new SettingsStore(storage);

    first.set('units', 'GiB');
    first.set('reduceMotion', true);
    first.set('oneFileSystem', false);
    first.set('sidebarCollapsed', true);
    first.set('explorerView', '3d');
    first.set('advisorEnabled', true);
    first.set('advisorUnloadAfter', 'windowClose');
    first.set('advisorLogQuestions', false);
    first.set('advisorUseGpu', false);

    const second = new SettingsStore(storage);

    expect(second.units).toBe('GiB');
    expect(second.reduceMotion).toBe(true);
    expect(second.oneFileSystem).toBe(false);
    expect(second.sidebarCollapsed).toBe(true);
    expect(second.explorerView).toBe('3d');
    expect(second.advisorEnabled).toBe(true);
    expect(second.advisorUnloadAfter).toBe('windowClose');
    expect(second.advisorLogQuestions).toBe(false);
    expect(second.advisorUseGpu).toBe(false);
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
        advisorEnabled: 'yes',
        advisorUnloadAfter: 'never',
        advisorLogQuestions: 'no',
        advisorUseGpu: 'no',
        extra: 1,
      }),
    );

    const settings = new SettingsStore(storage);

    expect(settings.units).toBe('GB');
    expect(settings.reduceMotion).toBe(false);
    expect(settings.oneFileSystem).toBe(false);
    expect(settings.explorerView).toBe('flat');
    expect(settings.advisorEnabled).toBe(false);
    expect(settings.advisorUnloadAfter).toBe('fiveMinutes');
    expect(settings.advisorLogQuestions).toBe(true);
    expect(settings.advisorUseGpu).toBe(true);
  });

  it('ignores the model choice an older version saved, and does not write it back', () => {
    const storage = new MemoryStorage();

    storage.setItem(
      SETTINGS_KEY,
      JSON.stringify({
        units: 'GiB',
        advisorEnabled: true,
        advisorModel: 'qwen3.5-2b',
      }),
    );

    const settings = new SettingsStore(storage);

    expect(settings.units).toBe('GiB');
    expect(settings.advisorEnabled).toBe(true);
    expect(settings).not.toHaveProperty('advisorModel');

    settings.set('reduceMotion', true);

    expect(storage.getItem(SETTINGS_KEY)).not.toContain('advisorModel');
  });

  it('ignores the download offer flag an older version saved, and does not write it back', () => {
    const storage = new MemoryStorage();

    storage.setItem(SETTINGS_KEY, JSON.stringify({ units: 'GiB', advisorOfferSeen: true }));

    const settings = new SettingsStore(storage);

    expect(settings.units).toBe('GiB');
    expect(settings).not.toHaveProperty('advisorOfferSeen');

    settings.set('reduceMotion', true);

    expect(storage.getItem(SETTINGS_KEY)).not.toContain('advisorOfferSeen');
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
