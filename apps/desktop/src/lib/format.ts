/** Pure formatting helpers. The size unit comes from the settings and is passed in. */

export type SizeUnits = 'GB' | 'GiB';

const DECIMAL_UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'] as const;
const BINARY_UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'] as const;

/**
 * "1.41 TB", "492 GB", "3.4 GB", "512 B".
 * Two decimals from TB up, one below 10, none from 10 on. `GB` counts in thousands, `GiB` in 1024s.
 */
export function formatSize(bytes: number, units: SizeUnits = 'GB'): string {
  const base = units === 'GiB' ? 1024 : 1000;
  const names = units === 'GiB' ? BINARY_UNITS : DECIMAL_UNITS;
  const safe = Math.max(0, bytes);

  let index = 0;
  let value = safe;

  while (value >= base && index < names.length - 1) {
    value /= base;
    index++;
  }

  if (index === 0) {
    return `${Math.round(value)} B`;
  }

  let decimals = value < 10 ? 1 : 0;
  if (index >= 4) {
    decimals = 2;
  }

  // 999.7 GB must read "1.00 TB", not "1000 GB".
  if (Number(value.toFixed(decimals)) >= base && index < names.length - 1) {
    value /= base;
    index++;
    decimals = index >= 4 ? 2 : 1;
  }

  return `${value.toFixed(decimals)} ${names[index]}`;
}

const COUNT_FORMAT = new Intl.NumberFormat('en');

/** "2,184,331" */
export function formatCount(count: number): string {
  return COUNT_FORMAT.format(Math.round(count));
}

/** Shown instead of a date or duration that cannot be shown. */
const UNKNOWN = '—';

/** "11.4 s", "2 min 5 s", "1 h 4 min"; "—" when `ms` is not a number. */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms)) {
    return UNKNOWN;
  }

  const seconds = Math.max(0, ms) / 1000;

  if (seconds < 60) {
    return `${seconds.toFixed(1)} s`;
  }

  const whole = Math.round(seconds);
  const minutes = Math.floor(whole / 60);

  if (minutes < 60) {
    return `${minutes} min ${whole % 60} s`;
  }

  return `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

/** "just now", "5 minutes ago", "2 hours ago", "3 days ago" from the time that has passed. A time in the future counts as now. */
export function formatAgo(elapsedMs: number): string {
  const minutes = Math.floor(Math.max(0, elapsedMs) / 60_000);

  if (minutes < 1) {
    return 'just now';
  }

  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  if (days >= 1) {
    return `${days} ${days === 1 ? 'day' : 'days'} ago`;
  }

  if (hours >= 1) {
    return `${hours} ${hours === 1 ? 'hour' : 'hours'} ago`;
  }

  return `${minutes} ${minutes === 1 ? 'minute' : 'minutes'} ago`;
}

const DATE_FORMAT = new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });

/**
 * "12 Mar 2025" from unix seconds. "—" for a time that is not a date: not a number, zero or negative, or beyond
 * what a `Date` holds (`Intl` throws a `RangeError` for that, and a wrong time stamp must not break a page).
 */
export function formatDate(unixSeconds: number): string {
  if (!Number.isFinite(unixSeconds) || unixSeconds <= 0) {
    return UNKNOWN;
  }

  const date = new Date(unixSeconds * 1000);

  return Number.isNaN(date.getTime()) ? UNKNOWN : DATE_FORMAT.format(date);
}

const DATE_TIME_FORMAT = new Intl.DateTimeFormat('en-GB', {
  day: 'numeric',
  month: 'short',
  year: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});

/** "12 Mar 2025, 14:05" in the local time zone, from unix seconds. "—" for a time that is not a date, as `formatDate`. */
export function formatDateTime(unixSeconds: number): string {
  if (!Number.isFinite(unixSeconds) || unixSeconds <= 0) {
    return UNKNOWN;
  }

  const date = new Date(unixSeconds * 1000);

  return Number.isNaN(date.getTime()) ? UNKNOWN : DATE_TIME_FORMAT.format(date);
}

export interface PercentOptions {
  /** One decimal under 10%, and the smallest step is 0.1%. For a share that is often small. */
  fine?: boolean;
}

/**
 * Every percentage the app shows. It never reads "100%" unless the share is exactly 1 or more, so "99.6%" reads
 * "99%", and never "0%" for a share that is not nothing: it reads "<1%" (fine: "<0.1%") under the smallest step.
 * Nothing, or a value that is not a number, reads "0%". Fine reads "99.5%" and up as one decimal, at most "99.9%".
 */
export function percentText(fraction: number, { fine = false }: PercentOptions = {}): string {
  if (!(fraction > 0)) {
    return '0%';
  }

  if (fraction >= 1) {
    return `${Math.round(fraction * 100)}%`;
  }

  const percent = fraction * 100;
  const step = fine ? 0.1 : 1;

  if (percent < step) {
    return `<${step}%`;
  }

  if (fine) {
    if (percent < 10) {
      const tenths = Math.round(percent * 10) / 10;

      // 9.96 rounds to 10.0 and reads "10%".
      return tenths < 10 ? `${tenths.toFixed(1)}%` : '10%';
    }

    if (percent >= 99.5) {
      return `${Math.min(99.9, Math.round(percent * 10) / 10).toFixed(1)}%`;
    }
  }

  return `${Math.min(99, Math.round(percent))}%`;
}

/** The last path segment: "/dev/nvme0n1p2" gives "nvme0n1p2", "C:\\Users\\me" gives "me". */
export { baseName } from './paths';
