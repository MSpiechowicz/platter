import type { OsName } from '../api/types';

/**
 * The operating system the app runs on, for the words the interface uses (Recycle Bin, Trash or trash, Finder or
 * file manager) and the separator it writes between folder names. The backend tells it with the advisor's status, and `bootstrap`
 * sets it once that has arrived. Until then, and in the mock, it is Linux.
 */
export class PlatformStore {
  os = $state<OsName>('linux');

  get isWindows(): boolean {
    return this.os === 'windows';
  }

  get isMac(): boolean {
    return this.os === 'macos';
  }

  /** What the system calls the place that deleted files go to: "Recycle Bin" on Windows, "Trash" on macOS, "trash" on Linux. */
  get trash(): string {
    if (this.isWindows) {
      return 'Recycle Bin';
    }

    return this.isMac ? 'Trash' : 'trash';
  }

  /** The same with its article, for a sentence: "the trash", "the Recycle Bin", "the Trash". */
  get theTrash(): string {
    if (this.isWindows) {
      return 'the Recycle Bin';
    }

    return this.isMac ? 'the Trash' : 'the trash';
  }

  /** The same, to start a sentence or a label. */
  get Trash(): string {
    return this.isWindows ? 'Recycle Bin' : 'Trash';
  }

  /** The program that shows a folder on disk: "Finder" on macOS, "the file manager" elsewhere. */
  get theFileManager(): string {
    return this.isMac ? 'Finder' : 'the file manager';
  }

  /** The label of the button that shows a folder there. */
  get showInFileManager(): string {
    return this.isMac ? 'Show in Finder' : 'Show in file manager';
  }

  /** The separator between the names of a path. */
  get separator(): '/' | '\\' {
    return this.isWindows ? '\\' : '/';
  }
}

export const platform = new PlatformStore();
