import { describe, expect, it } from 'vitest';
import { plain } from '../../api/mock-answer-segments';
import { readQuestion } from '../../api/mock-question-reader';
import type { EntryMatchDto } from '../../api/types';
import { accept, insertable, queryOf, wordAtCaret, type CaretWord } from './autocomplete';

const dir = (path: string): EntryMatchDto => ({ node: 1, path, allocated: 1000, kind: 'dir' });
const file = (path: string): EntryMatchDto => ({ node: 2, path, allocated: 1000, kind: 'file' });

/** The word under the caret (at the end of the text unless said), which the test expects to be there. */
function need(value: string, caret = value.length): CaretWord {
  const word = wordAtCaret(value, caret);

  if (!word) {
    throw new Error(`no word at ${caret} in "${value}"`);
  }

  return word;
}

/** The word at the end of the text. */
const atEnd = (value: string) => wordAtCaret(value, value.length);

describe('wordAtCaret', () => {
  it('finds the word the caret is in or right after, and none in white space or before a word', () => {
    const value = 'where is ~/Pobrane now';

    expect(wordAtCaret(value, 12)).toMatchObject({ start: 9, end: 18, text: '~/Pobrane' });
    expect(wordAtCaret(value, 18)).toMatchObject({ text: '~/Pobrane' });
    expect(wordAtCaret(value, 9)).toBeUndefined();
    expect(wordAtCaret(value, 19)).toBeUndefined();
    expect(wordAtCaret('~/a  ~/b', 5)).toBeUndefined();
  });

  it('takes a word that is `~`, or starts with `~/` or `/`, as a path', () => {
    expect(atEnd('~')?.mode).toBe('path');
    expect(atEnd('~/Pob')?.mode).toBe('path');
    expect(atEnd('/home/demo')?.mode).toBe('path');
    expect(atEnd('/')?.mode).toBe('path');
  });

  it('takes `~foo` as a name, not a path', () => {
    expect(atEnd('~foo')).toMatchObject({ mode: 'name', text: '~foo' });
    expect(atEnd('~fo')?.mode).toBe('name');
  });

  it('takes any other word as a name from 3 characters on, punctuation at its ends not counted', () => {
    expect(atEnd('ab')).toBeUndefined();
    expect(atEnd('ab?')).toBeUndefined();
    expect(atEnd('(ab),')).toBeUndefined();
    expect(atEnd('abc')?.mode).toBe('name');
    expect(atEnd('"abc"?')?.mode).toBe('name');
    expect(atEnd('old.iso')?.mode).toBe('name');
    expect(atEnd('...')).toBeUndefined();
  });

  it('asks for what is typed in the word: the query of a name has no punctuation around it, inner dots stay', () => {
    expect(queryOf(need('“old.iso”?'))).toBe('old.iso');
    expect(queryOf(need('Pobrane,'))).toBe('Pobrane');
    expect(queryOf(need('~/Pobr'))).toBe('~/Pobr');
  });

  it('takes a quoted path being typed, spaces included, and sends it without the opening quote', () => {
    const value = 'is "~/My Docs/Re';
    const word = need(value);

    expect(word).toMatchObject({ start: 3, end: value.length, mode: 'path' });
    expect(queryOf(word)).toBe('~/My Docs/Re');
  });

  it('sends a quoted path that is closed without its quotes', () => {
    const word = need('"~/My Docs"');

    expect(word.mode).toBe('path');
    expect(queryOf(word)).toBe('~/My Docs');
  });

  it('ends an open quote where the word under the caret ends, so the text after it is not part of it', () => {
    const value = 'is "~/My Docs/ more';
    const word = need(value, 14);

    expect(word.text).toBe('"~/My Docs/');
    expect(value.slice(word.end)).toBe(' more');
  });

  it('takes a closed quoted word with the caret before its closing quote, and sends its body', () => {
    const value = 'is "~/My Docs/" big';
    const word = need(value, 14);

    expect(word).toMatchObject({ start: 3, end: 15, text: '"~/My Docs/"', mode: 'path' });
    expect(queryOf(word)).toBe('~/My Docs/');
  });

  it('sends the body of a closed quoted word that punctuation follows, and leaves the punctuation out of it', () => {
    const value = 'where is "~/a b"?';
    const word = need(value);

    expect(word.text).toBe('"~/a b"');
    expect(queryOf(word)).toBe('~/a b');
    expect(value.slice(word.end)).toBe('?');
  });

  it('ends a word before the punctuation after it, never before a dot', () => {
    expect(need('~/Pob?').text).toBe('~/Pob');
    expect(need('old.iso?)').text).toBe('old.iso');
    expect(need('~/.').text).toBe('~/.');
  });

  it('starts a word after the `(` or `“` that opens it, and takes `(~/Doc` as a path', () => {
    expect(need('is (~/Doc')).toMatchObject({ start: 4, text: '~/Doc', mode: 'path' });
    expect(need('is “old.iso”')).toMatchObject({ start: 4, text: 'old.iso', mode: 'name' });
    expect(need('is ((~/Doc)')).toMatchObject({ start: 5, end: 10, text: '~/Doc' });
    expect(atEnd('is (')).toBeUndefined();
    expect(wordAtCaret('is (~/Doc', 4)).toBeUndefined();
    // A dot starts a name (`.cache`), and a quote that follows is not one the backend pairs.
    expect(need('is .cache')).toMatchObject({ start: 3, text: '.cache' });
    expect(need('is (".cache')).toMatchObject({ start: 3, text: '(".cache' });
  });

  it('takes a quote that is alone as no word', () => {
    expect(atEnd('"')).toBeUndefined();
  });
});

describe('accept: a path word', () => {
  it('keeps the typed prefix and puts the name of the match after it, with a slash for a folder', () => {
    const done = accept('~/Pob', need('~/Pob'), dir('~/Pobrane'));

    expect(done).toEqual({ value: '~/Pobrane/', caret: 10 });
  });

  it('keeps every level typed above the last one', () => {
    const value = '~/Projekty/pla';

    expect(accept(value, need(value), dir('~/Projekty/platter'))).toEqual({
      value: '~/Projekty/platter/',
      caret: 19,
    });
  });

  it('puts the name after a `/` that ends the word, and after `~`', () => {
    expect(accept('/', need('/'), dir('/home')).value).toBe('/home/');
    expect(accept('~', need('~'), dir('~/Pobrane')).value).toBe('~/Pobrane/');
  });

  it('adds no slash to a file, but a space after it, and the caret goes after the space', () => {
    expect(accept('~/Pobrane/ol', need('~/Pobrane/ol'), file('~/Pobrane/old.iso'))).toEqual({
      value: '~/Pobrane/old.iso ',
      caret: 18,
    });
  });

  it('keeps the text after the word, with the caret right after a folder’s slash', () => {
    const value = 'is ~/Pob big?';
    const word = need(value, 8);

    expect(accept(value, word, dir('~/Pobrane'))).toEqual({ value: 'is ~/Pobrane/ big?', caret: 13 });
  });

  it('adds no second space after a file when one is there, and moves the caret past it', () => {
    const value = 'is ~/Pob big?';
    const done = accept(value, need(value, 8), file('~/Pobrane'));

    expect(done).toEqual({ value: 'is ~/Pobrane big?', caret: 13 });
  });

  it('puts a path with white space in quotes, a folder too, with the caret inside the closing quote', () => {
    const opened = accept('~/My', need('~/My'), dir('~/My Files'));

    expect(opened).toEqual({ value: '"~/My Files/"', caret: 12 });

    const next = need(opened.value, opened.caret);

    expect(next.mode).toBe('path');
    expect(queryOf(next)).toBe('~/My Files/');
  });

  it('closes the quotes once when a folder with white space is picked and then a file in it', () => {
    const folder = accept('~/My', need('~/My'), dir('~/My Files'));
    const sub = accept(folder.value, need(folder.value, folder.caret), dir('~/My Files/Sub'));

    expect(sub).toEqual({ value: '"~/My Files/Sub/"', caret: 16 });

    const word = need(sub.value, sub.caret);

    expect(queryOf(word)).toBe('~/My Files/Sub/');

    const done = accept(sub.value, word, file('~/My Files/Sub/a.txt'));

    expect(done).toEqual({ value: '"~/My Files/Sub/a.txt" ', caret: 23 });
    expect(done.value.split('"')).toHaveLength(3);

    // What is submitted is read as that one path, not as `~/My`.
    expect(readQuestion(`how big is ${done.value}`, null).subject).toEqual({
      kind: 'path',
      text: '~/My Files/Sub/a.txt',
    });
  });

  it('does not double the quotes when a pick is made inside a quoted word, and keeps the text after it', () => {
    const value = 'is "~/My Files/Re" big';
    const word = need(value, 17);

    expect(queryOf(word)).toBe('~/My Files/Re');
    expect(accept(value, word, file('~/My Files/Report 1.pdf'))).toEqual({
      value: 'is "~/My Files/Report 1.pdf" big',
      caret: 29,
    });
    expect(accept(value, word, dir('~/My Files/Reports')).value).toBe('is "~/My Files/Reports/" big');
  });

  it('opens a folder with white space in an open quote the same way', () => {
    const value = '"~/My Files/Su';

    expect(accept(value, need(value), dir('~/My Files/Sub'))).toEqual({
      value: '"~/My Files/Sub/"',
      caret: 16,
    });
  });

  it('keeps the punctuation that followed the word', () => {
    const value = 'is ~/Pob? ok';
    const word = need(value, 8);

    expect(word.text).toBe('~/Pob');
    expect(accept(value, word, dir('~/Pobrane'))).toEqual({ value: 'is ~/Pobrane/? ok', caret: 13 });
    expect(accept(value, word, file('~/Pobrane'))).toEqual({ value: 'is ~/Pobrane? ok', caret: 14 });
  });

  it('keeps the `(` that opened the word', () => {
    const value = 'is (~/Doc) big';

    expect(accept(value, need(value, 9), dir('~/Documents'))).toEqual({
      value: 'is (~/Documents/) big',
      caret: 16,
    });
    expect(accept('is (Doc', need('is (Doc'), dir('~/Documents')).value).toBe('is (~/Documents/');
  });

  it('keeps sentence punctuation after a closed quote, and puts a space before other text glued to it', () => {
    const value = 'is "~/ab/fi"\'s size big';
    const word = need(value, 11);

    expect(word.text).toBe('"~/ab/fi"');
    expect(accept(value, word, file('~/ab/file.txt'))).toEqual({
      value: "is ~/ab/file.txt 's size big",
      caret: 17,
    });
    expect(accept(value, word, dir('~/ab/files')).value).toBe("is ~/ab/files/ 's size big");
    expect(accept('is "~/ab/fi"?, ok', need('is "~/ab/fi"?, ok', 11), file('~/ab/file.txt')).value).toBe(
      'is ~/ab/file.txt?, ok',
    );
    expect(accept('is "~/ab/fi"x', need('is "~/ab/fi"x', 11), file('~/ab/file.txt'))).toEqual({
      value: 'is ~/ab/file.txt x',
      caret: 17,
    });
    // The path no longer reads together with what was glued to it.
    expect(readQuestion(accept(value, word, file('~/ab/file.txt')).value, null).subject).toEqual({
      kind: 'path',
      text: '~/ab/file.txt',
    });
  });

  it('writes a quoted word again without a quote when the path has no white space', () => {
    const value = '"~/Pob';

    expect(accept(value, need(value), dir('~/Pobrane')).value).toBe('~/Pobrane/');
  });
});

describe('accept: a name word', () => {
  it('replaces the word with the whole path of the match', () => {
    const value = 'where is old.iso?';
    const word = need(value);

    expect(accept(value, word, file('~/Pobrane/old.iso'))).toEqual({
      value: 'where is ~/Pobrane/old.iso? ',
      caret: 28,
    });
  });

  it('keeps the punctuation after the word, for a folder too, and for a closed quote around it', () => {
    const value = 'where is cache?';

    expect(accept(value, need(value), dir('~/.cache'))).toEqual({ value: 'where is ~/.cache/?', caret: 18 });
    expect(accept('is (cache)', need('is (cache)', 9), file('~/.cache')).value).toBe('is (~/.cache) ');
    expect(accept('is "cache"?', need('is "cache"?'), file('~/.cache')).value).toBe('is ~/.cache? ');
  });

  it('ends a folder with a slash and leaves the caret after it, ready for the next level', () => {
    const value = 'size of Pobr';

    expect(accept(value, need(value), dir('~/Pobrane'))).toEqual({
      value: 'size of ~/Pobrane/',
      caret: 18,
    });
    expect(atEnd('size of ~/Pobrane/')?.mode).toBe('path');
  });

  it('quotes a path with white space', () => {
    const value = 'docs';

    expect(accept(value, need(value), file('~/My Docs/docs.txt')).value).toBe('"~/My Docs/docs.txt" ');
    expect(accept(value, need(value), dir('~/My Docs')).value).toBe('"~/My Docs/"');
  });
});

describe('accept: a path from the root', () => {
  it('quotes a path whose second character the backend would not read a bare path by, and no other', () => {
    const value = '/';

    for (const name of ['@home', '+foo', ' x', '#a']) {
      const path = `/${name}`;

      expect(accept(value, need(value), file(path)).value).toBe(`"${path}" `);
      expect(accept(value, need(value), dir(path)).value).toBe(`"${path}/"`);
    }

    for (const name of ['home', '.hidden', '_x', '-x', '9', 'ł', '日本']) {
      expect(accept(value, need(value), file(`/${name}`)).value).toBe(`/${name} `);
    }

    expect(accept('size of /h', need('size of /h'), dir('/home')).value).toBe('size of /home/');
  });

  it('reads the quoted path back as a path', () => {
    const question = accept('is /', need('is /'), file('/@home')).value;

    expect(readQuestion(question, null).subject).toEqual({ kind: 'path', text: '/@home' });
  });
});

describe('accept: a quoted pick after leading punctuation', () => {
  const picks: [string, EntryMatchDto, string][] = [
    ['a name with a space', file('~/a/report y.pdf'), '~/a/report y.pdf'],
    ['a name that ends in a bracket', file('~/a/foo)'), '~/a/foo)'],
    ['a name that ends in a dot', file('~/a/report.'), '~/a/report.'],
    ['a folder under a root the backend does not read bare', dir('/@srv'), '/@srv/'],
    ['a folder with a space', dir('~/a/b c/'), '~/a/b c/'],
  ];

  for (const lead of Array.from("?!,:;'()“”")) {
    for (const [label, match, read] of picks) {
      it(`sets ${label} apart from "${lead}" so the backend opens its quote`, () => {
        const value = `is ${lead}${match.path.startsWith('/') ? '/@s' : '~/a/re'}`;
        const word = need(value);
        const done = accept(value, word, match);
        const quoted = `"${read}"`;

        // A folder ends at its closing quote with the caret inside it, a file is followed by a space.
        expect(done.value).toBe(`is ${lead} ${quoted}${match.kind === 'dir' ? '' : ' '}`);
        expect(done.caret).toBe(done.value.length - (match.kind === 'dir' ? 1 : 0));
        expect(readQuestion(plain(done.value), null).subject).toEqual({ kind: 'path', text: read });
      });
    }
  }

  it('puts the caret inside the closing quote of a folder after the added space', () => {
    const value = 'is (~/a/b';
    const done = accept(value, need(value), dir('~/a/b c'));

    expect(done).toEqual({ value: 'is ( "~/a/b c/"', caret: 14 });
    expect(done.value[done.caret]).toBe('"');
  });

  it('adds no space to a bare pick or to a quote that follows white space', () => {
    expect(accept('is (~/a/re', need('is (~/a/re'), file('~/a/report.pdf')).value).toBe(
      'is (~/a/report.pdf ',
    );
    expect(accept('is "~/a/re', need('is "~/a/re'), file('~/a/report y.pdf')).value).toBe(
      'is "~/a/report y.pdf" ',
    );
  });
});

describe('an invisible character is white space', () => {
  // The backend turns each of these into a space before it reads the question, so `5<mark>"` is the word `5` and a
  // quote that opens a text. It closes at the next quote, which is the opening quote of a pick typed after it.
  const marks = ['​', '­', '\u0085', '⁠', '᠎', '‮'];
  const code = (mark: string) => `U+${mark.codePointAt(0)?.toString(16).toUpperCase().padStart(4, '0')}`;

  for (const mark of marks) {
    it(`pairs a stray quote after ${code(mark)} with the quote of the pick, as the backend does`, () => {
      const value = `is 5${mark}" wide, is "~/x/re`;
      const spaced = `is 5 " wide, is "~/x/re`;

      // The text the stray quote opens ends at the quote of the pick: `~/x/re` is not a quoted path.
      expect(atEnd(value)).toEqual(atEnd(spaced));
      expect(atEnd(value)?.text).not.toBe('"~/x/re');
      expect(readQuestion(plain(value), null).subject).toEqual(readQuestion(spaced, null).subject);
    });

    it(`ends a word at ${code(mark)} for the quote that follows it to open`, () => {
      const value = `see${mark}"~/a b/c`;

      expect(need(value)).toEqual({ start: 4, end: value.length, text: '"~/a b/c', mode: 'path' });

      const done = accept(value, need(value), file('~/a b/cat.txt'));

      expect(done.value).toBe(`see${mark}"~/a b/cat.txt" `);
      expect(readQuestion(plain(done.value), null).subject).toEqual({ kind: 'path', text: '~/a b/cat.txt' });
    });

    it(`keeps the text glued after a pick apart at ${code(mark)}`, () => {
      const value = `is ~/a/re${mark}now`;
      const done = accept(value, need(value, 9), file('~/a/report.pdf'));

      expect(done.value).toBe(`is ~/a/report.pdf${mark}now`);
      expect(readQuestion(plain(done.value), null).subject).toEqual({ kind: 'path', text: '~/a/report.pdf' });
    });
  }
});

describe('insertable', () => {
  it('offers a path that reads back as itself, plain spaces included', () => {
    expect(insertable('~/Downloads/old.iso')).toBe(true);
    expect(insertable('~/My Files/a b.txt')).toBe(true);
  });

  it('holds back a name the question would answer as another entry', () => {
    // A quote ends the quoted path early: the answer would be about `~/r/notes`.
    expect(insertable('~/r/notes "draft" v2.txt')).toBe(false);
    // A bidi override becomes a space, so the folder would be answered instead.
    expect(insertable('~/Downloads/\u202Efdp.exe')).toBe(false);
    // A tab and a double space are collapsed into one space: a sibling would be answered.
    expect(insertable('~/Downloads/a\tb.txt')).toBe(false);
    expect(insertable('~/Downloads/a  b.txt')).toBe(false);
    // A space at the end is trimmed, and a name that was not valid text is not itself.
    expect(insertable('~/Downloads/old ')).toBe(false);
    expect(insertable('~/Downloads/�.bin')).toBe(false);
    // An unassigned code point the backend counts as invisible (U+2065) would split the path.
    expect(insertable('~/Projekty⁥x')).toBe(false);
    // A path longer than the backend looks up is refused there, never shortened.
    expect(insertable(`~/${'ą'.repeat(520)}`)).toBe(false);
  });

  it('quotes a file path that ends in punctuation the backend would trim from a bare word', () => {
    for (const name of ['report.', 'foo..', 'foo)', 'foo?', 'foo”']) {
      const path = `~/Downloads/${name}`;
      const value = 'is ~/Downloads/rep';

      expect(accept(value, need(value), file(path)).value).toBe(`is "${path}" `);
    }
  });
});
