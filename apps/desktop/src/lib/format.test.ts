import { describe, expect, it } from 'vitest';
import {
  baseName,
  formatAgo,
  formatCount,
  formatDate,
  formatDateTime,
  formatDuration,
  formatSize,
  percentText,
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

describe('formatAgo', () => {
  const MINUTE = 60_000;

  it('says how long ago in the largest whole unit', () => {
    expect(formatAgo(30_000)).toBe('just now');
    expect(formatAgo(MINUTE)).toBe('1 minute ago');
    expect(formatAgo(5 * MINUTE)).toBe('5 minutes ago');
    expect(formatAgo(60 * MINUTE)).toBe('1 hour ago');
    expect(formatAgo(150 * MINUTE)).toBe('2 hours ago');
    expect(formatAgo(24 * 60 * MINUTE)).toBe('1 day ago');
    expect(formatAgo(3 * 24 * 60 * MINUTE)).toBe('3 days ago');
  });

  it('counts a time in the future as now', () => {
    expect(formatAgo(-5 * MINUTE)).toBe('just now');
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

describe('baseName', () => {
  it('returns the last segment', () => {
    expect(baseName('/dev/nvme0n1p2')).toBe('nvme0n1p2');
    expect(baseName('/mnt/backup/')).toBe('backup');
    expect(baseName('/')).toBe('/');
  });
});

describe('percentText', () => {
  it('rounds to a whole percent', () => {
    expect(percentText(0.7421)).toBe('74%');
    expect(percentText(0.46)).toBe('46%');
    expect(percentText(0.01)).toBe('1%');
  });

  it('reads 0% only for nothing, and for what is not a number', () => {
    expect(percentText(0)).toBe('0%');
    expect(percentText(-0.2)).toBe('0%');
    expect(percentText(Number.NaN)).toBe('0%');
  });

  it('reads "<1%" for a share that is not nothing but is under one per cent', () => {
    expect(percentText(1e-9)).toBe('<1%');
    expect(percentText(0.0049)).toBe('<1%');
    expect(percentText(0.0099)).toBe('<1%');
  });

  it('reads 100% only when the share is exactly all of it', () => {
    expect(percentText(1)).toBe('100%');
    expect(percentText(0.9999)).toBe('99%');
    expect(percentText(0.995)).toBe('99%');
    expect(percentText(0.99996)).toBe('99%');
    // More than the whole (shared blocks on btrfs) is not capped.
    expect(percentText(1.2)).toBe('120%');
  });

  describe('fine', () => {
    const fine = { fine: true };

    it('keeps one decimal under 10% and rounds from there', () => {
      expect(percentText(0.013, fine)).toBe('1.3%');
      expect(percentText(0.002, fine)).toBe('0.2%');
      expect(percentText(0.0996, fine)).toBe('10%');
      expect(percentText(0.97, fine)).toBe('97%');
      expect(percentText(0.46, fine)).toBe('46%');
    });

    it('reads "<0.1%" under a tenth of a per cent, and 0% for nothing', () => {
      expect(percentText(0.0004, fine)).toBe('<0.1%');
      expect(percentText(0.00099, fine)).toBe('<0.1%');
      expect(percentText(0.001, fine)).toBe('0.1%');
      expect(percentText(0, fine)).toBe('0%');
    });

    it('never reads 100% unless exact, and stops at 99.9%', () => {
      expect(percentText(1, fine)).toBe('100%');
      expect(percentText(0.99996, fine)).toBe('99.9%');
      expect(percentText(0.9951, fine)).toBe('99.5%');
      expect(percentText(0.994, fine)).toBe('99%');
    });
  });
});
