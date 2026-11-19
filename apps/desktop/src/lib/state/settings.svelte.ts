import {
  MAX_EXCLUDED,
  MAX_EXCLUDED_PATH_BYTES,
  type KeepPackageVersions,
  type UnloadAfter,
} from '../api/types';
import type { SizeUnits } from '../format';
import { isAbsolutePath } from '../paths';
import { isThemeChoice, type ThemeChoice } from './theme.svelte';

export type ExplorerView = 'flat' | '3d';

/** How often Platter looks for a new version in the background. `never` turns off only the automatic checks. */
export type UpdateSchedule = 'sixHours' | 'daily' | 'weekly' | 'never';

export interface SettingsData {
  units: SizeUnits;
  reduceMotion: boolean;
  /** Do not descend into other mounted filesystems while scanning. */
  oneFileSystem: boolean;
  sidebarCollapsed: boolean;
  /** Scan the folder of the last scan again at every start. Off: Platter opens the last scan as it was. */
  rescanOnOpen: boolean;
  /** How the Explorer draws the focused folder. */
  explorerView: ExplorerView;
  /** Use the local advisor model. Off until the model is downloaded; a finished download switches it on. */
  advisorEnabled: boolean;
  /** When the model leaves RAM again. The backend keeps the timer; `bootstrap` and a change send it there. */
  advisorUnloadAfter: UnloadAfter;
  /** Keep a local log of the questions and how they were read. The backend holds it in memory only, so it is sent at start. */
  advisorLogQuestions: boolean;
  /** Run the model on a graphics card when one fits it. An older save has no such value and gets the default, on. */
  advisorUseGpu: boolean;
  /** How often to check for a new version. An older save has no such value and gets the default, daily. */
  updateCheck: UpdateSchedule;
  /** A copy that can install itself (Linux AppImage) downloads and installs an update a check found by itself. It never restarts by itself. On by default (owner, 2026-11-17). */
  autoInstallUpdates: boolean;
  /** How many old versions of each pacman package stay in its cache when it is cleaned. */
  keepPackageVersions: KeepPackageVersions;
  /** Clean what needs the administrator password (system caches and logs). Off leaves those out of every plan. */
  askForAdmin: boolean;
  /** Absolute paths of folders every scan leaves out, in the order they were added, each in the style of the OS (`/` or `C:\\`). */
  excludedPaths: string[];
  /** Dark, light, or what the desktop uses. Dark by default, as before the setting existed. */
  theme: ThemeChoice;
}

const UPDATE_SCHEDULES: readonly UpdateSchedule[] = ['sixHours', 'daily', 'weekly', 'never'];

const UNLOAD_AFTER: readonly UnloadAfter[] = ['oneMinute', 'fiveMinutes', 'windowClose'];

export const SETTINGS_KEY = 'platter.settings';

const DEFAULTS: SettingsData = {
  units: 'GB',
  reduceMotion: false,
  oneFileSystem: true,
  sidebarCollapsed: false,
  rescanOnOpen: false,
  explorerView: 'flat',
  advisorEnabled: false,
  advisorUnloadAfter: 'fiveMinutes',
  advisorLogQuestions: true,
  advisorUseGpu: true,
  updateCheck: 'daily',
  autoInstallUpdates: true,
  keepPackageVersions: 2,
  askForAdmin: true,
  excludedPaths: [],
  theme: 'dark',
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

const encoder = new TextEncoder();

/** The folders to leave out as the backend takes them: absolute paths (`/home/x` or `C:\\Users\\x`), no repeats, at most 64 of at most 4096 bytes. */
export function cleanExcluded(paths: readonly unknown[]): string[] {
  const kept: string[] = [];

  for (const path of paths) {
    if (
      typeof path === 'string' &&
      isAbsolutePath(path) &&
      encoder.encode(path).length <= MAX_EXCLUDED_PATH_BYTES &&
      !kept.includes(path) &&
      kept.length < MAX_EXCLUDED
    ) {
      kept.push(path);
    }
  }

  return kept;
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

  if (UNLOAD_AFTER.includes(value.advisorUnloadAfter as UnloadAfter)) {
    clean.advisorUnloadAfter = value.advisorUnloadAfter as UnloadAfter;
  }

  if (isThemeChoice(value.theme)) {
    clean.theme = value.theme;
  }

  if (Array.isArray(value.excludedPaths)) {
    clean.excludedPaths = cleanExcluded(value.excludedPaths);
  }

  if (value.keepPackageVersions === 1 || value.keepPackageVersions === 2 || value.keepPackageVersions === 3) {
    clean.keepPackageVersions = value.keepPackageVersions;
  }

  if (UPDATE_SCHEDULES.includes(value.updateCheck as UpdateSchedule)) {
    clean.updateCheck = value.updateCheck as UpdateSchedule;
  }

  for (const key of [
    'reduceMotion',
    'oneFileSystem',
    'sidebarCollapsed',
    'rescanOnOpen',
    'advisorEnabled',
    'advisorLogQuestions',
    'advisorUseGpu',
    'autoInstallUpdates',
    'askForAdmin',
  ] as const) {
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
  rescanOnOpen = $state(DEFAULTS.rescanOnOpen);
  explorerView = $state<ExplorerView>(DEFAULTS.explorerView);
  advisorEnabled = $state(DEFAULTS.advisorEnabled);
  advisorUnloadAfter = $state<UnloadAfter>(DEFAULTS.advisorUnloadAfter);
  advisorLogQuestions = $state(DEFAULTS.advisorLogQuestions);
  advisorUseGpu = $state(DEFAULTS.advisorUseGpu);
  updateCheck = $state<UpdateSchedule>(DEFAULTS.updateCheck);
  autoInstallUpdates = $state(DEFAULTS.autoInstallUpdates);
  keepPackageVersions = $state<KeepPackageVersions>(DEFAULTS.keepPackageVersions);
  askForAdmin = $state(DEFAULTS.askForAdmin);
  excludedPaths = $state<string[]>([...DEFAULTS.excludedPaths]);
  theme = $state<ThemeChoice>(DEFAULTS.theme);

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
      rescanOnOpen: this.rescanOnOpen,
      explorerView: this.explorerView,
      advisorEnabled: this.advisorEnabled,
      advisorUnloadAfter: this.advisorUnloadAfter,
      advisorLogQuestions: this.advisorLogQuestions,
      advisorUseGpu: this.advisorUseGpu,
      updateCheck: this.updateCheck,
      autoInstallUpdates: this.autoInstallUpdates,
      keepPackageVersions: this.keepPackageVersions,
      askForAdmin: this.askForAdmin,
      excludedPaths: [...this.excludedPaths],
      theme: this.theme,
    };

    try {
      this.#storage?.setItem(SETTINGS_KEY, JSON.stringify(data));
    } catch {
      // Storage full or blocked: keep working with the in-memory value.
    }
  }
}

export const settings = new SettingsStore();
