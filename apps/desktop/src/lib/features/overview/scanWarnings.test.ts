import { describe, expect, it } from 'vitest';

import { warningLines } from './scanWarnings';

describe('warningLines', () => {
  it('says nothing when the scan went well', () => {
    expect(warningLines([])).toEqual([]);
  });

  it('explains an unreadable mount table once', () => {
    expect(warningLines(['mountTableUnavailable', 'mountTableUnavailable'])).toEqual([
      "Mounted disks could not be read, so only this disk's own files were counted",
    ]);
  });
});
