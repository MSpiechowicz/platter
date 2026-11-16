import { describe, expect, it } from 'vitest';
import { shortenMiddle, shownPath, visibleText } from './display';

describe('visibleText', () => {
  it('leaves ordinary text, including accents and emoji, alone', () => {
    expect(visibleText('~/Projekty/zażółć gęślą/📁 build')).toBe('~/Projekty/zażółć gęślą/📁 build');
  });

  it('writes control characters, new lines and tabs as \\u{…}', () => {
    expect(visibleText('a\nb\tc\u0000d\u001b[31m')).toBe('a\\u{A}b\\u{9}c\\u{0}d\\u{1B}[31m');
  });

  it('writes bidi overrides, isolates and marks as \\u{…}, so a name cannot reorder what follows', () => {
    expect(visibleText('/tmp/‮gpj.exe')).toBe('/tmp/\\u{202E}gpj.exe');
    expect(visibleText('a⁦b⁩c‏d')).toBe('a\\u{2066}b\\u{2069}c\\u{200F}d');
  });

  it('writes zero-width characters and line and paragraph separators as \\u{…}', () => {
    expect(visibleText('a​b﻿c d e')).toBe('a\\u{200B}b\\u{FEFF}c\\u{2028}d\\u{2029}e');
  });

  it('leaves no character that is hidden in the result', () => {
    const everything = Array.from({ length: 0x2100 }, (_, code) => String.fromCodePoint(code)).join('');

    expect(visibleText(everything)).not.toMatch(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u);
  });
});

describe('shortenMiddle', () => {
  it('keeps a path that fits', () => {
    expect(shortenMiddle('/home/demo', 20)).toBe('/home/demo');
    expect(shortenMiddle('12345', 5)).toBe('12345');
  });

  it('cuts in the middle and keeps the start and the end', () => {
    const shortened = shortenMiddle('/home/demo/Projekty/platter/target/debug/deps', 21);

    expect(Array.from(shortened)).toHaveLength(21);
    expect(shortened.startsWith('/home/demo')).toBe(true);
    expect(shortened.endsWith('debug/deps')).toBe(true);
    expect(shortened).toContain('…');
  });

  it('counts characters, so a pair of code units is never split', () => {
    const shortened = shortenMiddle('📁'.repeat(30), 10);

    expect(Array.from(shortened)).toHaveLength(10);
    expect(shortened).not.toMatch(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/);
  });
});

describe('shownPath', () => {
  it('shortens first and escapes after, so an escape is never cut in half', () => {
    const shown = shownPath(`/a/${'‮'.repeat(40)}/b`, 20);

    expect(shown).toMatch(/^\/a\/(\\u\{202E\})+…(\\u\{202E\})+\/b$/);
  });
});
