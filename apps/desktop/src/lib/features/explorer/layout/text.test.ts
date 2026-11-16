import { afterEach, describe, expect, it } from 'vitest';
import {
  approximateMeasure,
  endTruncate,
  fitTitle,
  fits,
  joinNames,
  measure,
  setMeasurer,
  wholeTitles,
  type Measurer,
} from './text';

/** Ten pixels a character, whatever the font. */
const ten: Measurer = (text) => text.length * 10;

describe('endTruncate', () => {
  it('keeps what fits, and cuts the rest at its end with an ellipsis', () => {
    expect(endTruncate("Sid Meier's Civilization VI", 270, 'name', ten)).toBe("Sid Meier's Civilization VI");
    expect(endTruncate("Sid Meier's Civilization VI", 150, 'name', ten)).toBe("Sid Meier's Ci…");
    expect(endTruncate('Car Mechanic Simulator', 140, 'name', ten)).toBe('Car Mechanic…');
  });

  it('does not leave a space before the ellipsis', () => {
    // 13 characters fit, the 13th is the space: it is not written.
    expect(endTruncate('Car Mechanic Simulator', 140, 'name', ten)).toBe('Car Mechanic…');
    expect(endTruncate('Car Mechanic Simulator', 50, 'name', ten)).toBe('');
  });

  it('writes nothing unless four characters and the ellipsis fit', () => {
    expect(endTruncate('Steam Library', 50, 'name', ten)).toBe('Stea…');
    expect(endTruncate('Steam Library', 49, 'name', ten)).toBe('');
  });

  it('never cuts in the middle: the result is the start of the text', () => {
    const cut = endTruncate('abcdefghijklmnop', 100, 'name', ten);

    expect('abcdefghijklmnop'.startsWith(cut.slice(0, -1))).toBe(true);
    expect(cut.endsWith('…')).toBe(true);
  });
});

describe('fitTitle', () => {
  const chain = ['Total War WARHAMMER III', 'Content', 'Paks'];

  it('lists a chain whole, then by its first folder', () => {
    expect(wholeTitles(chain)).toEqual([
      'Total War WARHAMMER III / Content / Paks',
      'Total War WARHAMMER III',
    ]);
    expect(wholeTitles(['home'])).toEqual(['home']);
    expect(joinNames(['a', 'b'])).toBe('a / b');
  });

  it('writes the whole chain when it fits completely', () => {
    expect(fitTitle(chain, 400, 'name', ten)).toBe('Total War WARHAMMER III / Content / Paks');
  });

  it('writes the first folder alone when the chain does not fit', () => {
    expect(fitTitle(chain, 399, 'name', ten)).toBe('Total War WARHAMMER III');
    expect(fitTitle(chain, 230, 'name', ten)).toBe('Total War WARHAMMER III');
  });

  it('cuts the first folder at its end when even that does not fit, and never shows the tail', () => {
    expect(fitTitle(chain, 150, 'name', ten)).toBe('Total War WARH…');
    expect(fitTitle(chain, 150, 'name', ten)).not.toContain('Paks');
  });

  it('writes nothing when there is no room for four characters', () => {
    expect(fitTitle(chain, 40, 'name', ten)).toBe('');
  });

  it('cuts a plain name at its end', () => {
    expect(fitTitle(['Car Mechanic Simulator'], 140, 'name', ten)).toBe('Car Mechanic…');
  });

  it('measures what it is given: a narrow font fits more', () => {
    const narrow: Measurer = (text) => text.length * 5;

    expect(fitTitle(chain, 230, 'name', narrow)).toBe('Total War WARHAMMER III / Content / Paks');
  });
});

describe('the measurer', () => {
  afterEach(() => setMeasurer(undefined));

  it('counts characters when there is nothing to measure with, the same way every time', () => {
    expect(approximateMeasure('abcd', 'name')).toBeCloseTo(31.6, 5);
    expect(approximateMeasure('abcd', 'size')).toBeCloseTo(26.4, 5);
    // jsdom has no canvas: the default measurer is the same deterministic one.
    expect(measure('abcd', 'name')).toBeCloseTo(31.6, 5);
  });

  it('can be replaced for a test, and put back', () => {
    setMeasurer(ten);
    expect(fits('abcd', 40, 'name')).toBe(true);
    expect(fits('abcde', 40, 'name')).toBe(false);
    setMeasurer(undefined);
    expect(fits('abcd', 32, 'name')).toBe(true);
  });
});
