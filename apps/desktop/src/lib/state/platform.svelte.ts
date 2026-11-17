import type { OsName } from '../api/types';

/**
 * The operating system the app runs on, for the words the interface uses (Recycle Bin or trash, how a copy updates)
 * and the separator it writes between folder names. The backend tells it with the advisor's status, and `bootstrap`
 * sets it once that has arrived. Until then, and in the mock, it is Linux.
 */
export class PlatformStore {
  os = $state<OsName>('linux');

  get isWindows(): boolean {
    return this.os === 'windows';
  }

  /** What the system calls the place that deleted files go to: "Recycle Bin" on Windows, "trash" elsewhere. */
  get trash(): string {
    return this.isWindows ? 'Recycle Bin' : 'trash';
  }

  /** The same with its article, for a sentence: "the trash", "the Recycle Bin". */
  get theTrash(): string {
    return this.isWindows ? 'the Recycle Bin' : 'the trash';
  }

  /** The same, to start a sentence or a label. */
  get Trash(): string {
    return this.isWindows ? 'Recycle Bin' : 'Trash';
  }

  /** The separator between the names of a path. */
  get separator(): '/' | '\\' {
    return this.isWindows ? '\\' : '/';
  }
}

export const platform = new PlatformStore();
