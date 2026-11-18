import type { HistoryCleanupDto } from '../../api/types';

export interface CleanedMostRow {
  /** The item as the history names it. */
  name: string;
  /** In how many cleanups it was. */
  count: number;
  /** The local day of the latest of them, `YYYY-MM-DD`. */
  last: string;
  /** `count` against the most any item has: the width of its bar, from 0 to 1. */
  share: number;
}

/**
 * The items of the cleanups ranked by how many cleanups had them, most first. An item listed twice by one cleanup counts
 * once. A tie goes to the item cleaned last, then to the name. The history keeps no size for a single item, so a count
 * is all there is to rank by.
 */
export function cleanedMost(cleanups: readonly HistoryCleanupDto[]): CleanedMostRow[] {
  const items = new Map<string, { count: number; at: number; last: string }>();

  for (const cleanup of cleanups) {
    for (const name of new Set(cleanup.what)) {
      const known = items.get(name);

      if (!known) {
        items.set(name, { count: 1, at: cleanup.at, last: cleanup.day });
      } else if (cleanup.at > known.at) {
        items.set(name, { count: known.count + 1, at: cleanup.at, last: cleanup.day });
      } else {
        known.count += 1;
      }
    }
  }

  const most = Math.max(0, ...[...items.values()].map((item) => item.count));

  return [...items]
    .sort(
      ([nameA, a], [nameB, b]) =>
        b.count - a.count || b.at - a.at || (nameA < nameB ? -1 : nameA > nameB ? 1 : 0),
    )
    .map(([name, item]) => ({ name, count: item.count, last: item.last, share: item.count / most }));
}

/** "once", "twice", "3 times". */
export function timesText(count: number): string {
  if (count === 1) {
    return 'once';
  }

  return count === 2 ? 'twice' : `${count} times`;
}
