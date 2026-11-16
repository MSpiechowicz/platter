import { describe, expect, it } from 'vitest';
import { abbreviateCount, countLabels, matches, normalizeQuery, segments } from './search';

describe('matches', () => {
  it('finds a part of a name, whatever the case', () => {
    expect(matches('Node_Modules', 'modules')).toBe(true);
    expect(matches('Node_Modules', 'NODE_')).toBe(true);
    expect(matches('node_modules', 'cache')).toBe(false);
    expect(matches('anything', '')).toBe(true);
  });

  it('takes a query of only spaces for no query', () => {
    expect(normalizeQuery('   ')).toBe('');
    expect(normalizeQuery(' a ')).toBe(' a ');
  });
});

describe('segments', () => {
  it('cuts a name into the parts that match and the parts between them', () => {
    expect(segments('Cargo.lock', 'LO')).toEqual([
      { text: 'Cargo.', hit: false },
      { text: 'lo', hit: true },
      { text: 'ck', hit: false },
    ]);
  });

  it('marks every place the query is found, and nothing for a name without it', () => {
    expect(segments('abab', 'ab')).toEqual([
      { text: 'ab', hit: true },
      { text: 'ab', hit: true },
    ]);
    expect(segments('node', 'zz')).toEqual([{ text: 'node', hit: false }]);
    expect(segments('node', '')).toEqual([{ text: 'node', hit: false }]);
  });

  it('keeps the text of the name, not the query, and so rebuilds the name', () => {
    const parts = segments('README.md', 'read');

    expect(parts[0]).toEqual({ text: 'READ', hit: true });
    expect(parts.map((part) => part.text).join('')).toBe('README.md');
  });
});

describe('abbreviateCount', () => {
  it('writes small numbers whole and thousands as 1.2k, 12k', () => {
    expect(abbreviateCount(4)).toBe('4');
    expect(abbreviateCount(143)).toBe('143');
    expect(abbreviateCount(999)).toBe('999');
    expect(abbreviateCount(1000)).toBe('1k');
    expect(abbreviateCount(1234)).toBe('1.2k');
    expect(abbreviateCount(12_345)).toBe('12k');
  });
});

describe('countLabels', () => {
  it('goes from "143 items" to "143", longest first', () => {
    expect(countLabels(143)).toEqual(['143 items', '143']);
    expect(countLabels(1)).toEqual(['1 item', '1']);
    expect(countLabels(1234)).toEqual(['1,234 items', '1.2k']);
  });

  it('says "M of N" while a query is on, and shortens it before anything else', () => {
    expect(countLabels(143, 12)).toEqual(['12 of 143 items', '12 of 143', '12']);
    expect(countLabels(1234, 1000)).toEqual(['1,000 of 1,234 items', '1,000 of 1,234', '1k of 1.2k', '1k']);
  });
});
