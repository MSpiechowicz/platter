import { describe, expect, it } from 'vitest';
import { countText, hasStatus } from './markedText';

describe('countText', () => {
  it('asks to tick folders when nothing is marked', () => {
    expect(countText(0, 0)).toBe('Tick folders in the list.');
  });

  it('counts the items, and says how many marks sit inside another one', () => {
    expect(countText(1, 0)).toBe('1 item');
    expect(countText(1234, 0)).toBe('1,234 items');
    expect(countText(2, 3)).toBe('2 items · 3 inside another');
  });
});

describe('hasStatus', () => {
  const store = (phase: string, notice?: string, outdated = false) => ({
    phase: { kind: phase } as never,
    notice,
    outdated,
  });

  it('is false only while nothing runs, nothing was said and the scan is current', () => {
    expect(hasStatus(store('idle'))).toBe(false);
    expect(hasStatus(store('planning'))).toBe(true);
    expect(hasStatus(store('running'))).toBe(true);
    expect(hasStatus(store('done'))).toBe(true);
    expect(hasStatus(store('idle', 'Not cleaned.'))).toBe(true);
    expect(hasStatus(store('idle', undefined, true))).toBe(true);
  });
});
