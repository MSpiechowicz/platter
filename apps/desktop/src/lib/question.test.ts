import { describe, expect, it } from 'vitest';
import { cleanQuestion, isBlankQuestion } from './question';

describe('isBlankQuestion', () => {
  it('counts white space and invisible characters as nothing, as the backend does', () => {
    for (const blank of ['', '   ', '​', ' ‮⁦ ', ' \t', '\u{E0041}']) {
      expect(isBlankQuestion(blank), JSON.stringify(blank)).toBe(true);
    }
    for (const asked of ['a', ' what is big? ', '​why']) {
      expect(isBlankQuestion(asked), JSON.stringify(asked)).toBe(false);
    }
  });
});

describe('cleanQuestion', () => {
  it('turns invisible characters into spaces and collapses white space, as the backend does', () => {
    expect(cleanQuestion('  what\u200Bis   big?\n')).toBe('what is big?');
    expect(cleanQuestion('\u202E')).toBe('');
  });
});

describe('the invisible characters', () => {
  it('are those of the backend: a format character it keeps is kept here too', () => {
    // U+0890 is a format character (Cf) that the backend's list does not hold: it is part of the question.
    expect(isBlankQuestion('\u0890')).toBe(false);
    expect(cleanQuestion('a\u0890b')).toBe('a\u0890b');
    expect(cleanQuestion('a\u2066b\u{E0041}c')).toBe('a b c');
  });
});
