import { ApiRequestError, type Api } from './client';
import type { UpdateOffer } from './types';

/*
 * The mock's stand-in for the update commands of `src-tauri/src/commands/update.rs`. By default there is no update.
 * With an `offer`, `checkUpdate` finds it and `installUpdate` "downloads" it in about two seconds.
 *
 * Still different from the backend:
 * - Nothing is downloaded or checked against a signature, and `restartApp` does nothing.
 */

const MB = 1_000_000;

const DEMO_OFFER: UpdateOffer = {
  version: '0.2.0',
  notes: [
    '3D column view in the Explorer',
    'Scanning btrfs snapshots is about 2× faster',
    'Fix: Trash size was counted twice',
  ],
  sizeBytes: 14 * MB,
  canInstall: true,
};

export interface MockUpdateOptions {
  /** The update the mock finds; fields left out take the demo values. Without it there is no update. */
  offer?: Partial<UpdateOffer>;
  /** The running version. */
  current?: string;
  /** How long `installUpdate` downloads. 0 finishes on the next tick. */
  downloadMs?: number;
  /** Time between progress events. */
  tickMs?: number;
}

export type MockUpdateApi = Pick<Api, 'checkUpdate' | 'installUpdate' | 'restartApp'>;

export function createMockUpdate(options: MockUpdateOptions = {}): MockUpdateApi {
  const current = options.current ?? '0.1.0';
  const downloadMs = options.downloadMs ?? 2000;
  const tickMs = options.tickMs ?? 50;
  const offer: UpdateOffer | null = options.offer ? { ...DEMO_OFFER, ...options.offer } : null;

  // The update the last check found, as in the backend: the caller never names what is installed.
  let pending: UpdateOffer | null = null;

  return {
    async checkUpdate() {
      pending = offer;

      return { current, update: offer && { ...offer, notes: [...offer.notes] } };
    },

    async installUpdate(onProgress) {
      if (pending && !pending.canInstall) {
        throw new ApiRequestError(
          'invalid',
          'This copy of Platter was installed by a package. Update it with your package manager.',
        );
      }

      const found = pending;

      if (!found) {
        throw new ApiRequestError('invalid', 'There is no update to install. Check for updates first.');
      }

      await download(found.sizeBytes, downloadMs, tickMs, onProgress);

      onProgress({ kind: 'installing' });
      await new Promise((resolve) => setTimeout(resolve, Math.min(downloadMs, 300)));
      pending = null;
    },

    async restartApp() {},
  };
}

/** Reports a download that takes `durationMs`, in steps of `tickMs`, ending with everything received. */
function download(
  total: number | null,
  durationMs: number,
  tickMs: number,
  onProgress: Parameters<Api['installUpdate']>[0],
): Promise<void> {
  // Without a size the mock still counts bytes, as a server that sends no length does.
  const size = total ?? DEMO_OFFER.sizeBytes ?? 0;

  return new Promise((resolve) => {
    const started = performance.now();

    const step = (): boolean => {
      const fraction = durationMs <= 0 ? 1 : Math.min(1, (performance.now() - started) / durationMs);

      onProgress({ kind: 'downloading', received: Math.round(size * fraction), total });

      return fraction >= 1;
    };

    if (step()) {
      resolve();
      return;
    }

    const timer = setInterval(() => {
      if (step()) {
        clearInterval(timer);
        resolve();
      }
    }, tickMs);
  });
}
