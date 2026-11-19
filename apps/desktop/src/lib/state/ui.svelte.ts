import type { NodeId, ScanId } from '../api/types';
import { scan } from './scan.svelte';

/** The item chosen in the sidebar. What is shown for it also depends on the scan (see navigation.ts). */
export type Page = 'scan' | 'overview' | 'explorer' | 'cleanup' | 'advisor' | 'history' | 'settings';
export type ToastKind = 'ok' | 'info' | 'error';

/** A button on a toast: clicking it runs `run` and closes the toast. */
export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: number;
  title: string;
  description: string;
  kind: ToastKind;
  action?: ToastAction;
}

const TOAST_LIFETIME_MS = 4200;
/** A toast with a button stays longer: there is something to do, not only to read. */
const ACTION_TOAST_LIFETIME_MS = 10_000;
const MAX_TOASTS = 3;

export class UiStore {
  page = $state<Page>('scan');
  /**
   * Where the Explorer opens: the node last asked for by `openInExplorer`, or else the folder the user last had open
   * in the Explorer (`rememberExplorer`). In memory only, and for one scan (`explorerScan`).
   */
  explorerFocus = $state<NodeId | undefined>(undefined);
  /** The colour of `explorerFocus` when it is known, so the Explorer need not look it up. */
  explorerFocusColor = $state<string | undefined>(undefined);
  /** The scan `explorerFocus` belongs to: node ids mean nothing in another scan. */
  explorerScan = $state<ScanId | undefined>(undefined);
  toasts = $state<Toast[]>([]);

  #nextToastId = 1;
  // Timer handles are bookkeeping, never rendered, so a plain Map is right.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  #timers = new Map<number, ReturnType<typeof setTimeout>>();

  go(page: Page): void {
    this.page = page;
  }

  /**
   * `color` is the folder's colour (see `folderColor.ts`), when the caller already has it. While no scan is ready
   * the id belongs to no scan, so nothing is kept (and an older place goes too) and only the page changes.
   */
  openInExplorer(nodeId: NodeId, color?: string): void {
    if (scan.state.status === 'ready') {
      this.explorerFocus = nodeId;
      this.explorerFocusColor = color;
      this.explorerScan = scan.state.scanId;
    } else {
      this.forgetExplorer();
    }

    this.page = 'explorer';
  }

  /** The Explorer tells where the user is, so that coming back from another page shows the same folder. */
  rememberExplorer(scanId: ScanId, nodeId: NodeId, color?: string): void {
    // An Explorer of a scan that is already gone must not bring its ids back.
    if (scan.state.status !== 'ready' || scan.state.scanId !== scanId) {
      return;
    }

    this.explorerFocus = nodeId;
    this.explorerFocusColor = color;
    this.explorerScan = scanId;
  }

  /** The Explorer opens on the scan root again (a new scan, or no scan). */
  forgetExplorer(): void {
    this.explorerFocus = undefined;
    this.explorerFocusColor = undefined;
    this.explorerScan = undefined;
  }

  /** Shows a toast at the bottom right, with a button when `action` is given. It closes itself after a few seconds. */
  toast(title: string, description = '', kind: ToastKind = 'ok', action?: ToastAction): number {
    const id = this.#nextToastId++;

    this.toasts = [...this.toasts, { id, title, description, kind, action }].slice(-MAX_TOASTS);
    this.#timers.set(
      id,
      setTimeout(() => this.dismiss(id), action ? ACTION_TOAST_LIFETIME_MS : TOAST_LIFETIME_MS),
    );

    return id;
  }

  dismiss(id: number): void {
    clearTimeout(this.#timers.get(id));
    this.#timers.delete(id);
    this.toasts = this.toasts.filter((toast) => toast.id !== id);
  }
}

export const ui = new UiStore();
