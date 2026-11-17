import { afterEach, describe, expect, it } from 'vitest';
import { platform } from '../../state/platform.svelte';
import {
  canRestore,
  formatDay,
  formatDaysUntil,
  kindsText,
  methodText,
  statusText,
  thinNote,
  whatText,
} from './format';

describe('formatDay', () => {
  it('writes the day and the short month', () => {
    expect(formatDay('2026-11-12')).toBe('12 Nov');
    expect(formatDay('2026-08-03')).toBe('3 Aug');
  });

  it('adds the year only when it is not the year of today', () => {
    expect(formatDay('2025-11-20', '2026-11-17')).toBe('20 Nov 2025');
    expect(formatDay('2026-01-02', '2026-11-17')).toBe('2 Jan');
  });

  it('leaves text that is not a day as it is', () => {
    expect(formatDay('soon')).toBe('soon');
    expect(formatDay('2026-13-01')).toBe('2026-13-01');
  });
});

describe('formatDaysUntil', () => {
  it('says days, weeks, months or years, rounded and in the right number', () => {
    expect(formatDaysUntil(1)).toBe('~1 day');
    expect(formatDaysUntil(9.4)).toBe('~9 days');
    expect(formatDaysUntil(21)).toBe('~3 weeks');
    expect(formatDaysUntil(154)).toBe('~5 months');
    expect(formatDaysUntil(1100)).toBe('~3 years');
  });

  it('does not promise a time that is less than a day away, or not a number', () => {
    expect(formatDaysUntil(0.4)).toBe('less than a day');
    expect(formatDaysUntil(Number.NaN)).toBe('less than a day');
  });
});

describe('what the table says', () => {
  it('names the methods once each, with the command of a tool', () => {
    expect(methodText([{ kind: 'trash' }])).toBe('trash');
    expect(methodText([{ kind: 'tool', command: 'paccache -rk2' }])).toBe('paccache -rk2');
    expect(methodText([{ kind: 'deleted' }, { kind: 'emptiedTrash' }, { kind: 'deleted' }])).toBe(
      'deleted, emptied trash',
    );
  });

  it('lists what was cleaned, and shows a dash for a run that names nothing', () => {
    expect(whatText(['Old recordings', 'Old downloads'])).toBe('Old recordings · Old downloads');
    expect(whatText([])).toBe('—');
  });

  it('words each status, and offers Restore while the items may be in the trash', () => {
    expect(statusText('inTrash')).toBe('in trash');
    expect(statusText('restored')).toBe('restored');
    expect(statusText('noLongerInTrash')).toBe('no longer in trash');
    expect(statusText('permanent')).toBe('permanent');
    expect(statusText('notChecked')).toBe('may be in trash');

    const offered = (['inTrash', 'restored', 'noLongerInTrash', 'permanent', 'notChecked'] as const).filter(
      (status) => canRestore({ status }),
    );

    expect(offered).toEqual(['inTrash', 'notChecked']);
  });
});

describe('kindsText', () => {
  it('lists one, two and three kinds as a sentence would', () => {
    expect(kindsText(['caches'])).toBe('caches');
    expect(kindsText(['build output', 'caches'])).toBe('build output and caches');
    expect(kindsText(['build output', 'caches', 'old downloads'])).toBe(
      'build output, caches and old downloads',
    );
  });
});

describe('thinNote', () => {
  it('says how the history is made, how many days there are, and when the growth shows', () => {
    expect(thinNote(3)).toBe(
      'Platter records disk usage once a day while it is open. 3 days so far. The growth rate shows after 7.',
    );
    expect(thinNote(1)).toContain('1 day so far.');
    expect(thinNote(0)).toContain('No day is recorded yet.');
  });

  it('has no semicolon', () => {
    expect(thinNote(2)).not.toContain(';');
  });
});

describe('on Windows', () => {
  afterEach(() => {
    platform.os = 'linux';
  });

  it('names the Recycle Bin where Linux says trash', () => {
    platform.os = 'windows';

    expect(methodText([{ kind: 'trash' }])).toBe('Recycle Bin');
    expect(statusText('inTrash')).toBe('in Recycle Bin');
    expect(statusText('noLongerInTrash')).toBe('no longer in Recycle Bin');
  });
});
