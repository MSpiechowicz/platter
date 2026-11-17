import { describe, expect, it } from 'vitest';
import {
  baseName,
  formatCount,
  formatDate,
  formatDateTime,
  formatDuration,
  formatPercent,
  formatShare,
  formatSize,
} from './format';

const GB = 1_000_000_000;
const GIB = 1024 ** 3;

describe('formatSize', () => {
  it('uses one decimal below 10, none from 10, two from TB', () => {
    expect(formatSize(3.4 * GB)).toBe('3.4 GB');
    expect(formatSize(36 * GB)).toBe('36 GB');
    expect(formatSize(1.41e12)).toBe('1.41 TB');
    expect(formatSize(1.9e12)).toBe('1.90 TB');
  });

  it('prints plain bytes without a decimal', () => {
    expect(formatSize(0)).toBe('0 B');
    expect(formatSize(512)).toBe('512 B');
    expect(formatSize(-5)).toBe('0 B');
  });

  it('counts GB in thousands and GiB in 1024s', () => {
    expect(formatSize(GIB, 'GB')).toBe('1.1 GB');
    expect(formatSize(GIB, 'GiB')).toBe('1.0 GiB');
    expect(formatSize(1000 ** 2, 'GB')).toBe('1.0 MB');
    expect(formatSize(1024 ** 2, 'GiB')).toBe('1.0 MiB');
  });

  it('does not print a value that rounds up to the next unit boundary', () => {
    expect(formatSize(999.7 * GB)).toBe('1.00 TB');
    expect(formatSize(999_999)).toBe('1.0 MB');
  });
});

describe('formatCount', () => {
  it('groups thousands', () => {
    expect(formatCount(2184331)).toBe('2,184,331');
    expect(formatCount(0)).toBe('0');
  });
});

describe('formatDuration', () => {
  it('shows seconds with a decimal under a minute', () => {
    expect(formatDuration(11_400)).toBe('11.4 s');
    expect(formatDuration(0)).toBe('0.0 s');
  });

  it('shows a dash when the time is not a number', () => {
    expect(formatDuration(Number.NaN)).toBe('—');
    expect(formatDuration(Infinity)).toBe('—');
  });

  it('switches to minutes and hours', () => {
    expect(formatDuration(125_000)).toBe('2 min 5 s');
    expect(formatDuration(3_840_000)).toBe('1 h 4 min');
  });
});

describe('formatDate', () => {
  it('formats unix seconds as a day, month and year', () => {
    expect(formatDate(Date.UTC(2025, 2, 12, 12) / 1000)).toBe('12 Mar 2025');
  });

  it('shows a dash for a time that is no date instead of throwing', () => {
    // The last second a `Date` can hold is 8.64e12 s; one more makes `Intl` throw a RangeError.
    for (const bad of [Number.NaN, Infinity, -Infinity, 0, -1, 8.64e12 + 1, 1e18, Number.MAX_VALUE]) {
      expect(formatDate(bad)).toBe('—');
    }

    expect(formatDate(8.64e12)).not.toBe('—');
  });
});

describe('formatDateTime', () => {
  it('formats unix seconds as a day, month, year and the local time', () => {
    expect(formatDateTime(new Date(2025, 2, 12, 14, 5, 30).getTime() / 1000)).toBe('12 Mar 2025, 14:05');
    expect(formatDateTime(new Date(2025, 2, 12, 0, 7).getTime() / 1000)).toBe('12 Mar 2025, 00:07');
  });

  it('shows a dash for a time that is no date instead of throwing', () => {
    for (const bad of [Number.NaN, Infinity, 0, -1, 8.64e12 + 1, 1e18]) {
      expect(formatDateTime(bad)).toBe('—');
    }
  });
});

describe('formatPercent', () => {
  it('rounds a fraction to a whole percent', () => {
    expect(formatPercent(0.7421)).toBe('74%');
    expect(formatPercent(1)).toBe('100%');
  });
});

describe('baseName', () => {
  it('returns the last segment', () => {
    expect(baseName('/dev/nvme0n1p2')).toBe('nvme0n1p2');
    expect(baseName('/mnt/backup/')).toBe('backup');
    expect(baseName('/')).toBe('/');
  });
});

describe('formatShare', () => {
  it('is a plain percentage, but "<1%" for a share that is not nothing', () => {
    expect(formatShare(0.46)).toBe('46%');
    expect(formatShare(0.0099)).toBe('<1%');
    expect(formatShare(0.01)).toBe('1%');
    expect(formatShare(0)).toBe('0%');
  });
});
