/*
 * The pieces of an answer, as the backend writes them (`answer.rs`: `Segment`, `quote_name`, `render`, `cut_path`;
 * `check.rs`: `Known`, `segments_of`): text, and the paths of the scan as segments of their own. Used by the mock's
 * template (`mock-scan-question.ts`); the answer is always the template's (the model only reads the question), so no model text goes through here.
 */

/** What a path of an answer is: the row of the sheet with this index, or the folder the question was about. */
export type Place = { at: 'row'; index: number } | { at: 'scope' };

export type Segment = { kind: 'text'; text: string } | { kind: 'path'; path: string; place: Place };

/** The longest path the sheet (and so an answer) shows, in characters. */
export const SHEET_PATH_CHARS = 160;

/** The substitutes for the quotes that set a path apart, inside a path. */
const OPEN_SUBSTITUTE = '‟';
const CLOSE_SUBSTITUTE = '〃';

/**
 * `path` at most `chars` characters long: a longer one keeps its beginning and its end and has `…` in the middle (so
 * the cut form is never the path of a real folder, and the end, which tells the folders of one parent apart, stays).
 */
export function cutPath(path: string, chars: number = SHEET_PATH_CHARS): string {
  const all = Array.from(path);

  if (all.length <= chars) {
    return path;
  }

  const kept = chars - 1;
  const head = Math.ceil(kept / 2);
  const tail = kept - head;

  return `${all.slice(0, head).join('')}…${all.slice(all.length - tail).join('')}`;
}

/** A path set apart in the text; a `“`, `”` or `"` in the name is shown as `‟` or `〃`, so it cannot close the quote. */
export function quoteName(path: string): string {
  return `“${withoutQuotes(path)}”`;
}

/** `path` with every `“`, `”` or `"` shown as `‟` or `〃`: the text `quoteName` sets in quotes. */
function withoutQuotes(path: string): string {
  return Array.from(path)
    .map((c) => (c === '“' ? OPEN_SUBSTITUTE : c === '”' || c === '"' ? CLOSE_SUBSTITUTE : c))
    .join('');
}

/** The segments as one text: every path in “ ” quotes. */
export function render(segments: readonly Segment[]): string {
  return segments
    .map((segment) => (segment.kind === 'text' ? segment.text : quoteName(segment.path)))
    .join('');
}

/** Adds `text` to the segments, to the text before it when there is one. */
export function pushText(segments: Segment[], text: string): void {
  if (text === '') {
    return;
  }

  const last = segments.at(-1);

  if (last?.kind === 'text') {
    last.text += text;
  } else {
    segments.push({ kind: 'text', text });
  }
}

const INVISIBLE =
  /[\p{Cc}\u00AD\u0600-\u0605\u061C\u06DD\u070F\u08E2\u180E\u200B-\u200F\u2028\u2029\u202A-\u202E\u2060-\u206F\uFEFF\uFFF9-\uFFFB\u{110BD}\u{110CD}\u{1BCA0}-\u{1BCA3}\u{1D173}-\u{1D17A}\u{E0001}\u{E0020}-\u{E007F}]/gu;

/** The text as the check reads it: invisible characters are spaces and white space is single. */
export function plain(text: string): string {
  return text.replace(INVISIBLE, ' ').split(/\s+/u).filter(Boolean).join(' ');
}

/** A path the sheet holds, as an answer is read. */
export interface KnownPath {
  /** The path as `quoteName` shows it (its own quotes as `‟` or `〃`): two folders that read the same so are one text. */
  text: string;
  /** The path as the sheet holds it, for the path segment. */
  real: string;
  /** The ways the path can stand in a text, longest first: `text`, and the path as the sheet holds it. */
  forms: string[];
  /** The rows shown with this text (none for the scope and the context paths). */
  rows: number[];
  /** Whether it is the scope. */
  scope: boolean;
  /**
   * Whether the sheet shows this text for more than one folder: two rows whose names read the same once invisible
   * characters and white space are gone, a row and a scope or a context path that are other folders, or a real folder
   * whose name is the cut form (with `…`) of a longer path. It is bound to no row.
   */
  ambiguous: boolean;
}

/** What `Known` reads from a sheet, as it is written (`shown`). */
export interface KnownSource {
  rows: readonly { path: string }[];
  /** The folder asked about; empty for the whole scan. */
  scope: string;
  /** Paths that are no row, that an answer may still name (the user's own words, in a note). */
  contextPaths: readonly string[];
}

/** The sheet as the backend made it, before any path is cut: what makes two folders the same folder. */
export type KnownOriginal = KnownSource;

const encoder = new TextEncoder();

/** A folder behind one shown text. */
interface Folder {
  identity: string;
  row?: number;
  scope: boolean;
}

/**
 * The paths an answer may bind to a row, longest first (so a path with a longer one beginning the same way is found as
 * the longer). Not `~` or `/` alone: they name nothing in particular. A text is one folder only when one folder has it:
 * the folders are told apart by their path before any cut (the scope and a context path are the row whose path they
 * are), so a text with more than one folder behind it is `ambiguous`.
 */
export function knownPaths(original: KnownOriginal, source: KnownSource): KnownPath[] {
  const listed: { text: string; identity: string; row?: number; scope: boolean }[] = [];
  const add = (shown: string, before: string, row: number | undefined, scope: boolean): void => {
    listed.push({
      text: plain(shown.replace(/\/+$/, '')),
      identity: before.replace(/\/+$/, ''),
      row,
      scope,
    });
  };

  source.rows.forEach((row, index) => add(row.path, original.rows[index]?.path ?? row.path, index, false));

  if (source.scope !== '') {
    add(source.scope, original.scope, undefined, true);
  }

  source.contextPaths.forEach((path, index) =>
    add(path, original.contextPaths[index] ?? path, undefined, false),
  );

  const grouped = new Map<string, { real: string; forms: string[]; folders: Folder[] }>();

  for (const item of listed) {
    if (Array.from(item.text).length < 2) {
      continue;
    }

    const key = withoutQuotes(item.text);
    const group = grouped.get(key) ?? { real: item.text, forms: [key], folders: [] };
    const folders = group.folders;

    grouped.set(key, group);
    if (!group.forms.includes(item.text)) {
      group.forms.push(item.text);
    }

    // A row is a folder of its own; the scope and a context path are the folder they say.
    const same =
      item.row === undefined ? folders.find((folder) => folder.identity === item.identity) : undefined;

    if (same) {
      same.scope ||= item.scope;
    } else {
      folders.push({ identity: item.identity, row: item.row, scope: item.scope });
    }
  }

  const length = (text: string): number => encoder.encode(text).length;
  const paths: KnownPath[] = [...grouped].map(([text, { real, forms, folders }]) => ({
    text,
    real,
    forms: forms.sort((a, b) => length(b) - length(a)),
    rows: folders.flatMap((folder) => (folder.row === undefined ? [] : [folder.row])),
    scope: folders.some((folder) => folder.scope),
    ambiguous: folders.length > 1,
  }));

  const bytes = (path: KnownPath): number => Math.max(...path.forms.map(length));

  return paths.sort((a, b) => bytes(b) - bytes(a) || (a.text < b.text ? -1 : a.text > b.text ? 1 : 0));
}

/** Whether the sheet shows `path` (a path as the backend wrote it) for more than one folder. */
export function isAmbiguous(known: readonly KnownPath[], path: string): boolean {
  const text = withoutQuotes(plain(path.replace(/\/+$/, '')));

  return known.some((entry) => entry.ambiguous && entry.text === text);
}

/** What a known path is in an answer: a row, the scope, or neither (a context path, or a text shown for several folders). */
function placeOf(path: KnownPath): Place | null {
  const row = path.rows[0];

  if (path.ambiguous) {
    return null;
  }

  if (row !== undefined) {
    return { at: 'row', index: row };
  }

  return path.scope ? { at: 'scope' } : null;
}

const WORD_OPENERS = new Set(['(', '[', '{', '"', "'", '`', '“', '‘']);
const WORD_CLOSERS = new Set([',', ';', ':', '!', '?', ')', ']', '}', '"', "'", '`', '”', '’']);
const CLOSER_OF: Record<string, string> = { '“': '”', '‘': '’', '"': '"', "'": "'", '`': '`' };

const isSpace = (char: string): boolean => /^\s$/u.test(char);

function startsAWord(text: string, at: number): boolean {
  const before = text[at - 1];

  return before === undefined || isSpace(before) || WORD_OPENERS.has(before);
}

function endsAWord(text: string, end: number): boolean {
  const next = text[end];

  if (next === undefined) {
    return true;
  }

  if (next === '.') {
    const after = text[end + 1];

    return after === undefined || isSpace(after);
  }

  return isSpace(next) || WORD_CLOSERS.has(next);
}

/**
 * `text` (a note, a total's label) in pieces: plain text, and every known path of the sheet in it as a path segment
 * (a quote around it is dropped; the answer writes its own “ ” around every path). A path that is no folder of the
 * scan (a context path), or that the sheet shows for more than one folder, stays in the text, in quotes.
 */
export function segmentsOf(text: string, known: readonly KnownPath[]): Segment[] {
  const read = plain(text);
  const segments: Segment[] = [];
  let buffer = '';
  let at = 0;

  while (at < read.length) {
    // A path is found as it is, or as `quoteName` wrote it (its own quotes shown as `‟` or `〃`).
    const fits = (candidate: string): boolean =>
      read.startsWith(candidate, at) && endsAWord(read, at + candidate.length);
    const path = startsAWord(read, at) ? known.find((entry) => entry.forms.some(fits)) : undefined;
    const form = path?.forms.find(fits) ?? '';

    if (path === undefined) {
      const char = String.fromCodePoint(read.codePointAt(at) as number);

      buffer += char;
      at += char.length;

      continue;
    }

    at += form.length;

    // A path that the text put in quotes loses them.
    const closer = CLOSER_OF[buffer.at(-1) ?? ''];

    if (closer !== undefined && read[at] === closer) {
      buffer = buffer.slice(0, -1);
      at += 1;
    }

    const place = placeOf(path);

    if (place === null) {
      buffer += quoteName(path.text);
    } else {
      pushText(segments, buffer);
      buffer = '';
      segments.push({ kind: 'path', path: path.real, place });
    }
  }

  pushText(segments, buffer);

  return segments;
}
