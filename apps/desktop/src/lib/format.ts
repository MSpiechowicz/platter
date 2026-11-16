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

/** "74%" from a fraction. */
export function formatPercent(fraction: number): string {
  return `${Math.round(fraction * 100)}%`;
}

/** "74%", or "<1%" for a share that is not nothing but rounds down to it. */
export function formatShare(fraction: number): string {
  return fraction > 0 && fraction < 0.01 ? '<1%' : formatPercent(fraction);
}

/** The last path segment: "/dev/nvme0n1p2" gives "nvme0n1p2". */
export function baseName(path: string): string {
  const trimmed = path.replace(/\/+$/, '');

  return trimmed.slice(trimmed.lastIndexOf('/') + 1) || trimmed || '/';
}
