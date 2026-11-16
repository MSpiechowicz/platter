export interface UpdateInfo {
  current: string;
  next: string;
  size: string;
  /** What is new, one short line each. */
  notes: string[];
  /** "2 hours ago" */
  checked: string;
}

export type UpdatePhase = 'available' | 'downloading' | 'ready';

/** Downloads and installs an update, reporting progress from 0 to 1. The real one arrives with the updater milestone. */
export type UpdateDownloader = (info: UpdateInfo, onProgress: (fraction: number) => void) => Promise<void>;

/** State of the sidebar update notice. `info` stays null until an update check finds something. */
export class UpdaterStore {
  info = $state<UpdateInfo | null>(null);
  phase = $state<UpdatePhase>('available');
  progress = $state(0);

  downloader: UpdateDownloader | null = null;

  async install(): Promise<void> {
    const info = this.info;

    if (!info || !this.downloader || this.phase !== 'available') {
      return;
    }

    this.phase = 'downloading';
    this.progress = 0;

    try {
      await this.downloader(info, (fraction) => (this.progress = fraction));
      this.phase = 'ready';
    } catch {
      this.phase = 'available';
    }
  }

  /** Hides the notice until the next check. */
  later(): void {
    this.info = null;
    this.phase = 'available';
  }
}

export const updater = new UpdaterStore();

/** For the `?update` test hook: a made-up update that "downloads" in about two seconds. */
export function showDemoUpdate(store: UpdaterStore = updater): void {
  store.info = {
    current: '0.1.0',
    next: '0.2.0',
    size: '14 MB',
    notes: [
      '3D column view in the Explorer',
      'Scanning btrfs snapshots is about 2× faster',
      'Fix: Trash size was counted twice',
    ],
    checked: '2 hours ago',
  };
  store.downloader = (_info, onProgress) =>
    new Promise((resolve) => {
      const started = performance.now();
      const timer = setInterval(() => {
        const fraction = Math.min(1, (performance.now() - started) / 2200);

        onProgress(fraction);

        if (fraction >= 1) {
          clearInterval(timer);
          resolve();
        }
      }, 50);
    });
}
