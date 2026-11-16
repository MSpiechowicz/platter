import type { CleanupStore } from '../../state/cleanup.svelte';

/** The line under the marked total: how many items, and how many of the marks sit inside another one. */
export function countText(items: number, inside: number): string {
  if (items === 0) {
    return 'Tick folders in the list.';
  }

  const text = items === 1 ? '1 item' : `${items.toLocaleString('en-US')} items`;

  return inside > 0 ? `${text} · ${inside} inside another` : text;
}

/** The footer says what is going on, or what just happened, instead of the total: a run, a result, a notice. */
export function hasStatus(store: Pick<CleanupStore, 'phase' | 'notice' | 'outdated'>): boolean {
  return store.phase.kind !== 'idle' || store.notice !== undefined || store.outdated;
}
