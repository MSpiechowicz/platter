import type { HistoryCleanupDto, HistoryMethodDto, HistoryStatus } from '../../api/types';
import { platform } from '../../state/platform.svelte';

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'] as const;

/** The `thin` rule of the backend: this many days are needed before it says how fast the disk grows. */
export const DAYS_FOR_GROWTH = 7;

/**
 * "12 Nov" from the local day `2026-11-12`. The year is added when it is not the year of `today`, because the
 * window reaches back up to 366 days. Text that is not a day comes back as it is.
 */
export function formatDay(day: string, today?: string): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  const month = match ? MONTHS[Number(match[2]) - 1] : undefined;

  if (!match || !month) {
    return day;
  }

  const text = `${Number(match[3])} ${month}`;

  return today && today.slice(0, 4) !== match[1] ? `${text} ${match[1]}` : text;
}

/** "~5 months", "~3 weeks", "~9 days": a rough time, not a promise. */
export function formatDaysUntil(days: number): string {
  if (!Number.isFinite(days) || days < 1) {
    return 'less than a day';
  }

  const plural = (count: number, unit: string): string => `~${count} ${unit}${count === 1 ? '' : 's'}`;

  if (days < 14) {
    return plural(Math.round(days), 'day');
  }

  if (days < 60) {
    return plural(Math.round(days / 7), 'week');
  }

  if (days < 730) {
    return plural(Math.round(days / 30), 'month');
  }

  return plural(Math.round(days / 365), 'year');
}

/** What the table says about how a cleanup was done: "trash", "deleted", "emptied trash", or the tool's command. "Recycle Bin" for "trash" on Windows. */
export function methodText(methods: readonly HistoryMethodDto[]): string {
  const names = methods.map((method) => {
    switch (method.kind) {
      case 'trash':
        return platform.trash;
      case 'deleted':
        return 'deleted';
      case 'emptiedTrash':
        return `emptied ${platform.trash}`;
      case 'tool':
        return method.command ?? 'tool';
    }
  });

  return [...new Set(names)].join(', ');
}

export function whatText(what: readonly string[]): string {
  return what.length > 0 ? what.join(' · ') : '—';
}

/** The words for a status that offers nothing to do. A status that can be restored has none. */
export function statusText(status: HistoryStatus): string {
  switch (status) {
    case 'inTrash':
      return `in ${platform.trash}`;
    case 'notChecked':
      return `may be in ${platform.trash}`;
    case 'restored':
      return 'restored';
    case 'noLongerInTrash':
      return `no longer in ${platform.trash}`;
    case 'permanent':
      return 'permanent';
  }
}

/** Restore is offered while the items may still be in the trash (the Recycle Bin on Windows). */
export function canRestore(cleanup: Pick<HistoryCleanupDto, 'status'>): boolean {
  return cleanup.status === 'inTrash' || cleanup.status === 'notChecked';
}

/** The muted line under the chart while there are too few days to say how fast the disk grows. */
export function thinNote(daysSoFar: number): string {
  const so =
    daysSoFar === 0 ? 'No day is recorded yet.' : `${daysSoFar} ${daysSoFar === 1 ? 'day' : 'days'} so far.`;

  return `Platter records disk usage once a day while it is open. ${so} The growth rate shows after ${DAYS_FOR_GROWTH}.`;
}

/** "build output", "build output and caches", "build output, caches and old downloads": the kinds a sentence lists. */
export function kindsText(kinds: readonly string[]): string {
  if (kinds.length <= 1) {
    return kinds[0] ?? '';
  }

  return `${kinds.slice(0, -1).join(', ')} and ${kinds[kinds.length - 1]}`;
}
