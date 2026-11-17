import type { EntryMatchDto } from '../../api/types';

/**
 * The word of the question that is being completed. `path` is a path typed level by level (it is `~` or starts with
 * `~/` or `/`), `name` is any other word long enough to be looked up by its name.
 */
export interface CaretWord {
  /** Where the word starts in the text, its opening quote included. */
  start: number;
  /** Where the word ends: the text after it is not the word's. */
  end: number;
  /** The word as typed, quotes included. */
  text: string;
  mode: 'path' | 'name';
}

/** The fewest characters of a word, once punctuation is trimmed, that are looked up by name. */
export const MIN_NAME_CHARS = 3;

/** Punctuation that a sentence puts around a word and a name does not start or end with. */
const PUNCTUATION = '?!,.:;"\'()“”';
/** What a sentence puts after a word, that a pick leaves where it is: not a dot, which may be part of the name still typed. */
const TAIL_PUNCTUATION = '?!,:;"\'()“”';
/** What a sentence puts before a word, that a pick leaves where it is: not a dot (`.cache`) and not a quote. */
const LEADING_PUNCTUATION = "?!,:;'()“”";
const QUOTE = '"';
const WHITE_SPACE = /\s/;
/**
 * The characters the backend turns into spaces before it reads the question (its `is_invisible`): a control, a soft
 * hyphen, a zero width or bidirectional mark, the invisible operators, a byte order mark and the like.
 */
const INVISIBLE =
  /[\p{Cc}\u00AD\u0600-\u0605\u061C\u06DD\u070F\u08E2\u180E\u200B-\u200F\u2028\u2029\u202A-\u202E\u2060-\u206F\uFEFF\uFFF9-\uFFFB\u{110BD}\u{110CD}\u{1BCA0}-\u{1BCA3}\u{1D173}-\u{1D17A}\u{E0001}\u{E0020}-\u{E007F}]/u;

/** White space as the backend reads the question: an invisible character is white space too, so it ends a word. */
function isSpace(character: string | undefined): boolean {
  return character !== undefined && (WHITE_SPACE.test(character) || INVISIBLE.test(character));
}

/** The text without the punctuation at its ends: inner dots stay ("old.iso"). */
function trimPunctuation(text: string): string {
  let from = 0;
  let to = text.length;

  while (from < to && PUNCTUATION.includes(text[from] as string)) {
    from++;
  }

  while (to > from && PUNCTUATION.includes(text[to - 1] as string)) {
    to--;
  }

  return text.slice(from, to);
}

/** The text between the quotes of a word, or of a word that is still open: what follows a closing quote is not in it. */
function unquoted(text: string): string {
  if (!text.startsWith(QUOTE)) {
    return text;
  }

  const closing = text.indexOf(QUOTE, 1);

  return closing === -1 ? text.slice(1) : text.slice(1, closing);
}

/** A path is `~` alone, or starts with `~/` or `/`: a word like `~foo` is a name. */
function isPath(body: string): boolean {
  return body === '~' || body.startsWith('~/') || body.startsWith('/');
}

/** A word of the text: `core` is where it ends without the punctuation a sentence puts after it. */
interface Span {
  start: number;
  end: number;
  core: number;
  open: boolean;
}

/**
 * The words of the text, split on white space. A word that starts with a quote goes on through white space up to its
 * closing quote (a path with a space in it), or to the end of the text when nothing closes it, and then it is `open`.
 * The core of a closed quoted word ends at its closing quote, that of any other word before its trailing punctuation.
 */
function wordsOf(value: string): Span[] {
  const words: Span[] = [];
  let index = 0;

  while (index < value.length) {
    if (isSpace(value[index])) {
      index++;

      continue;
    }

    const start = index;
    let open = false;
    let closed: number | undefined;

    if (value[index] === QUOTE) {
      const closing = value.indexOf(QUOTE, index + 1);

      if (closing === -1) {
        open = true;
        index = value.length;
      } else {
        closed = closing + 1;
        index = closing + 1;
      }
    }

    while (index < value.length && !isSpace(value[index])) {
      index++;
    }

    let core = closed ?? index;

    while (closed === undefined && core > start && TAIL_PUNCTUATION.includes(value[core - 1] as string)) {
      core--;
    }

    words.push({ start, end: index, core, open });
  }

  return words;
}

/**
 * The word the caret is in or right after, or undefined when there is none to complete: the caret is in white space or at
 * the start of a word, or the word is neither a path nor a name of 3 characters. The word ends before the punctuation a
 * sentence puts after it (`old.iso?`, `"~/a b"?`), so a pick keeps that. A word that is open (a quote that is not
 * closed) ends where the word under the caret does, so what follows the caret on the line is not part of it.
 */
export function wordAtCaret(value: string, caret: number): CaretWord | undefined {
  const found = wordsOf(value).find((word) => caret > word.start && caret <= word.end);

  if (!found) {
    return undefined;
  }

  let start = found.start;
  let end = found.core;

  if (found.open) {
    end = caret;

    while (end < value.length && !isSpace(value[end])) {
      end++;
    }
  }

  // A word that opens with `(` or `“` starts after it: a pick keeps it, and `(~/Doc` is the path `~/Doc`. Nothing is
  // skipped before a quote, which would be one the backend does not pair.
  if (value[start] !== QUOTE) {
    let lead = start;

    while (lead < end && LEADING_PUNCTUATION.includes(value[lead] as string)) {
      lead++;
    }

    if (value[lead] !== QUOTE) {
      start = lead;
    }
  }

  if (caret <= start) {
    return undefined;
  }

  const text = value.slice(start, end);
  const body = unquoted(text);

  if (isPath(body)) {
    return { start, end, text, mode: 'path' };
  }

  if (trimPunctuation(body).length >= MIN_NAME_CHARS) {
    return { start, end, text, mode: 'name' };
  }

  return undefined;
}

/** What is sent to the backend for the word: a path as typed, a name without the punctuation around it. */
export function queryOf(word: CaretWord): string {
  const body = unquoted(word.text);

  return word.mode === 'path' ? body : trimPunctuation(body);
}

/** The last part of a path, without a slash at its end. */
function baseName(path: string): string {
  const trimmed = path.endsWith('/') ? path.slice(0, -1) : path;

  return trimmed.slice(trimmed.lastIndexOf('/') + 1);
}

/**
 * A character the question would lose or change: a control, an invisible format mark, an unassigned code point (the
 * backend treats U+2065 as invisible), a line break, U+FFFD.
 */
const ALTERED = /[\p{Cc}\p{Cf}\p{Cn}\p{Zl}\p{Zp}�]/u;

/** The longest path the backend looks up (in UTF-8 bytes): a longer one is refused, never shortened. */
const MAX_PATH_BYTES = 1024;

/** What the backend trims from the end of a path written without quotes. */
const TRIMMED_AT_END = /[?,;:!'()“”.]$/;

const encoder = new TextEncoder();

/**
 * The path reads back as itself once it is in the question. The backend turns invisible characters into spaces,
 * collapses runs of white space, trims a quoted path and ends it at the next `"`, so a name with any of those would
 * be answered as another entry (its folder, a sibling, a decoy). Such a match is not offered.
 */
export function insertable(path: string): boolean {
  if (path.includes(QUOTE) || ALTERED.test(path) || path !== path.trim()) {
    return false;
  }

  if (encoder.encode(path).length > MAX_PATH_BYTES) {
    return false;
  }

  // White space other than single plain spaces.
  return !/[^\S ]| {2}/.test(path);
}

/** A bare path that starts with `/` is read by the backend only when its second character is one of these. */
const ROOTED_SECOND = /[\p{L}\p{N}._-]/u;

/** Whether the backend would not read the path as a path when it is written bare: `/@home`, `/+foo`. */
function unreadBare(path: string): boolean {
  const second = Array.from(path)[1];

  return path.startsWith('/') && second !== undefined && !ROOTED_SECOND.test(second);
}

/**
 * Writes a path into the question. A path with white space is put in straight quotes, a folder too: its closing quote
 * is there for the next level to be typed inside it. So is a path that ends in punctuation the backend trims from a
 * bare word (`report.`, `foo)`), as only white space is trimmed inside quotes, and a path from `/` whose second
 * character is not a letter, a digit or one of `._-`, which a bare word is not read as a path.
 */
function written(path: string): string {
  const quoted = WHITE_SPACE.test(path) || TRIMMED_AT_END.test(path) || unreadBare(path);

  return quoted ? `${QUOTE}${path}${QUOTE}` : path;
}

/**
 * The question with the match put in place of the word, and where the caret goes.
 *
 * A path word keeps what was typed up to and including its last `/` and takes the name of the match after it, so a
 * choice made at one level does not rewrite the levels above. Any other word is replaced with the whole path of the
 * match. A folder ends with `/` and the caret stays right after it (inside the closing quote of a path with white
 * space), so the next level is the word under the caret. A file ends the word: a space follows it (one is added when
 * none is there) and the caret goes after the space. The punctuation that followed the word and the text after it are
 * kept. Text glued to the closing quote of a quoted word (`"~/a"'s`) is not punctuation: it is kept too, with a space
 * put before it, so the path is not read as one word with it (`~/a/file's`).
 */
export function accept(
  value: string,
  word: CaretWord,
  match: EntryMatchDto,
): { value: string; caret: number } {
  const folder = match.kind === 'dir';
  let path: string;

  if (word.mode === 'path') {
    // A bare `~` is the home folder, so its children are listed after `~/`.
    const typed = queryOf(word) === '~' ? '~/' : queryOf(word);

    path = typed.slice(0, typed.lastIndexOf('/') + 1) + baseName(match.path);
  } else {
    path = match.path;
  }

  const inserted = written(folder ? `${path}/` : path);
  const rest = value.slice(word.end);
  let gluedEnd = 0;

  while (gluedEnd < rest.length && !isSpace(rest[gluedEnd])) {
    gluedEnd++;
  }

  const glued = rest.slice(0, gluedEnd);
  // The backend opens a quote only at the start of the text or after white space, so a quoted pick that follows the
  // punctuation kept before the word (`(`) is set apart from it by a space.
  const apart = inserted.startsWith(QUOTE) && word.start > 0 && !isSpace(value[word.start - 1]) ? ' ' : '';
  const before = value.slice(0, word.start) + apart;
  // Only sentence punctuation stays against the path. Any other text glued to it moves behind a space.
  const tail = Array.from(glued).every((character) => TAIL_PUNCTUATION.includes(character)) ? glued : '';
  const after = rest.slice(tail.length);

  if (folder) {
    const inside = inserted.endsWith(QUOTE) ? 1 : 0;
    const split = after !== '' && !isSpace(after[0]) ? ' ' : '';

    return {
      value: before + inserted + tail + split + after,
      caret: before.length + inserted.length - inside,
    };
  }

  const gap = isSpace(after[0]) ? '' : ' ';

  return {
    value: before + inserted + tail + gap + after,
    caret: before.length + inserted.length + tail.length + 1,
  };
}
