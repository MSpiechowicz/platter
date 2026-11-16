import type { SizeUnits } from '../format';

export type ExplorerView = 'flat' | '3d';

export interface SettingsData {
  units: SizeUnits;
  reduceMotion: boolean;
  /** Do not descend into other mounted filesystems while scanning. */
  oneFileSystem: boolean;
  sidebarCollapsed: boolean;
  /** How the Explorer draws the focused folder. */
  explorerView: ExplorerView;
}

export const SETTINGS_KEY = 'platter.settings';

const DEFAULTS: SettingsData = {
  units: 'GB',
  reduceMotion: false,
  oneFileSystem: true,
  sidebarCollapsed: false,
  explorerView: 'flat',
};

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

/** localStorage can be missing or throw (private mode, quota); settings then live for the session only. */
function defaultStorage(): StorageLike | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

/** Keeps only values of the right type, so a hand-edited or old save cannot break the app. */
function sanitize(raw: unknown): Partial<SettingsData> {
  if (typeof raw !== 'object' || raw === null) {
    return {};
  }

  const value = raw as Record<string, unknown>;
  const clean: Partial<SettingsData> = {};

  if (value.units === 'GB' || value.units === 'GiB') {
    clean.units = value.units;
  }

  if (value.explorerView === 'flat' || value.explorerView === '3d') {
    clean.explorerView = value.explorerView;
  }

  for (const key of ['reduceMotion', 'oneFileSystem', 'sidebarCollapsed'] as const) {
    if (typeof value[key] === 'boolean') {
      clean[key] = value[key];
    }
  }

  return clean;
}

export class SettingsStore implements SettingsData {
  units = $state<SizeUnits>(DEFAULTS.units);
  reduceMotion = $state(DEFAULTS.reduceMotion);
  oneFileSystem = $state(DEFAULTS.oneFileSystem);
  sidebarCollapsed = $state(DEFAULTS.sidebarCollapsed);
  explorerView = $state<ExplorerView>(DEFAULTS.explorerView);

  readonly #storage: StorageLike | null;

  constructor(storage: StorageLike | null = defaultStorage()) {
    this.#storage = storage;
    this.#load();
  }

  /** Changes one setting and saves all of them. */
  set<K extends keyof SettingsData>(key: K, value: SettingsData[K]): void {
    (this as SettingsData)[key] = value;
    this.#save();
  }

  #load(): void {
    try {
      const saved = this.#storage?.getItem(SETTINGS_KEY);

      if (saved) {
        Object.assign(this, sanitize(JSON.parse(saved)));
      }
    } catch {
      // A corrupt save falls back to the defaults.
    }
  }

  #save(): void {
    const data: SettingsData = {
      units: this.units,
      reduceMotion: this.reduceMotion,
      oneFileSystem: this.oneFileSystem,
      sidebarCollapsed: this.sidebarCollapsed,
      explorerView: this.explorerView,
    };

    try {
      this.#storage?.setItem(SETTINGS_KEY, JSON.stringify(data));
    } catch {
      // Storage full or blocked: keep working with the in-memory value.
    }
  }
}

export const settings = new SettingsStore();
