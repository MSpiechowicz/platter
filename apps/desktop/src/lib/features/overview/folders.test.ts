import { describe, expect, it } from 'vitest';
import { fewFoldersNote } from './folders';

describe('fewFoldersNote', () => {
  it('says how few folders there are, up to two', () => {
    expect(fewFoldersNote(0)).toBe('No folders here');
    expect(fewFoldersNote(1)).toBe('Only 1 folder here');
    expect(fewFoldersNote(2)).toBe('Only 2 folders here');
  });

  it('stays silent when there are enough folders to fill the card', () => {
    expect(fewFoldersNote(3)).toBeUndefined();
    expect(fewFoldersNote(8)).toBeUndefined();
  });
});
