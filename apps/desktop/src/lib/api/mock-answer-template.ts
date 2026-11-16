import { formatCount, formatSize } from '../format';
import {
  cutPath,
  isAmbiguous,
  knownPaths,
  pushText,
  quoteName,
  segmentsOf,
  type KnownPath,
  type Place,
  type Segment,
} from './mock-answer-segments';
import type { ScanIntent, ScanKind, ScanRowDto, SizeUnits, Verdict } from './types';

/*
 * The answer written from the facts alone: a port of `template_segments` (`crates/platter-ai/src/answer.rs`), sentence
 * for sentence. The mock collects the rows and the `FactSheet` (`mock-scan-question.ts`), and every word of the answer
 * is written here, so nothing a model says can reach the reader. The only text that is not Platter's is a name of the
 * user's disk (a path, a rule title the user wrote, a label's reason the model gave earlier); each is set apart in “ ”
 * quotes. Not mirrored: the unit tests of the writer itself (`answer.rs` has its own), and a name whose characters the
 * backend reads lossily (a path that is no UTF-8 does not exist in the mock tree).
 */

/** What the question is told when it is not about the scan; the same text as `OFFTOPIC_TEXT` in `answer.rs`. */
export const OFFTOPIC_TEXT =
  'I can only answer questions about this scan: what takes space, what can be cleaned, why a folder is big, and what the model labelled.';

/** The note of a scan that is from before a cleanup; the same text as `OUTDATED_NOTE` in `answer.rs`. */
export const OUTDATED_NOTE = 'This scan is from before a cleanup; rescan for current figures.';

/** Most rows in a sheet, most notes in a sheet, longest note in characters. */
export const SHEET_ROWS = 10;
export const SHEET_NOTES = 6;
export const NOTE_CHARS = 240;
/** Most rows an answer names in one sentence; the others are counted. */
export const NAMED_ROWS = 5;

type RowSource = NonNullable<ScanRowDto['source']>;

/** One row of the sheet: a folder, with what the backend knows about it. */
export interface FactRow {
  /** The folder's display path (home written as `~`). */
  path: string;
  /** Bytes on disk (0 for a folder that could not be read). */
  size: number;
  verdict?: Verdict;
  by?: RowSource;
  /** By who made the verdict: a rule's title, the guard's detail or the model's reason. */
  note?: string;
  /** The rule's own words on the folder, or the guard's reason. */
  why?: string;
  /** Whether Platter could not read the folder, so its size is unknown. */
  unreadable: boolean;
}

/** What a figure over many folders is. */
export type TotalKind = 'plain' | 'verdict' | 'category' | 'older' | 'rule';

/** A figure over many folders: a verdict's total, a category's share, a family's size. */
export interface FactTotal {
  label: string;
  bytes?: number;
  count?: number;
  kind: TotalKind;
  /** The verdict a `verdict` total is of. */
  verdict?: Verdict;
  /** A share of the folder's size, in whole per cent (`category` and `older`). */
  percent?: number;
  /** A name that stands for the many small items of a verdict's total that are not listed (`__pycache__`). */
  example?: string;
}

/** Everything an answer may say, in the form it is written (`FactSheet` in `answer.rs`). */
export interface FactSheet {
  intent: ScanIntent;
  /** The family of folders asked about (`largest` and `cleanable`). */
  kind: ScanKind;
  /** The folder asked about, as a display path; empty for the whole scan. */
  scope: string;
  /** The title of the rule whose folders (more than one) the question is about. */
  set?: string;
  /** The name looked for (`find`). */
  name: string;
  /** The media category asked about (`video`, `image`, `audio`), when the question is about one. */
  category?: string;
  scopeSize?: number;
  files?: number;
  /** The rows are the very thing the question named, not what lies inside a folder: the answer says yes or no. */
  subject: boolean;
  rows: readonly FactRow[];
  totals: readonly FactTotal[];
  /** What else the reader should know, each a sentence of its own, written into the answer as it is. */
  notes: readonly string[];
  /** The scan is from before a cleanup: the answer says `OUTDATED_NOTE`. */
  outdated: boolean;
  /** Folders that matched but are not among the rows. */
  more: number;
  /** Paths that are not rows, that the notes may still speak of. */
  contextPaths: readonly string[];
}

const VERDICT_LABEL: Record<Verdict, string> = {
  safe: 'Safe to clean',
  review: 'For you to review',
  keep: 'Kept',
};

/** `FactTotal` of what a verdict totals. */
export const verdictTotal = (verdict: Verdict, bytes: number, count?: number): FactTotal => ({
  label: VERDICT_LABEL[verdict],
  bytes,
  count,
  kind: 'verdict',
  verdict,
});

/** How a family of folders is called, in the singular and the plural. */
const FAMILY: Record<ScanKind, [string, string]> = {
  any: ['folder', 'folders'],
  cache: ['cache', 'caches'],
  build: ['build output or dependency folder', 'build output or dependency folders'],
  media: ['media folder', 'media folders'],
};

/** A row of the sheet with its index. */
interface Indexed {
  index: number;
  row: FactRow;
}

const items = (count: number): string => `${formatCount(count)} ${count === 1 ? 'item' : 'items'}`;

/** The sheet as it is written: at most 10 rows and 6 notes, paths cut to 160 characters and notes to 240. */
function shown(sheet: FactSheet): FactSheet {
  return {
    ...sheet,
    more: sheet.more + Math.max(0, sheet.rows.length - SHEET_ROWS),
    rows: sheet.rows.slice(0, SHEET_ROWS).map((row) => ({ ...row, path: cutPath(row.path) })),
    notes: sheet.notes.slice(0, SHEET_NOTES).map((note) => Array.from(note).slice(0, NOTE_CHARS).join('')),
    scope: cutPath(sheet.scope),
    contextPaths: sheet.contextPaths.map((path) => cutPath(path)),
  };
}

/** The last part of a path as the answer shows it. */
const baseName = (path: string): string => path.replace(/\/+$/, '').split('/').pop() ?? path;

/** One sentence, built piece by piece. */
class Line {
  readonly parts: Segment[] = [];
  /** A row that could not be read says `(could not be read)` where its size would be. */
  markUnreadable = true;

  constructor(
    private readonly known: readonly KnownPath[],
    private readonly units: SizeUnits,
  ) {}

  text(text: string): void {
    pushText(this.parts, text);
  }

  /** A text of the user's disk or of the model (a rule title, a reason), set apart in quotes. */
  quoted(text: string): void {
    pushText(this.parts, quoteName(text));
  }

  /**
   * A folder's path: a path segment bound to its place, or, when the sheet shows that text for more than one folder,
   * text in quotes that is bound to none of them.
   */
  path(path: string, place: Place): void {
    if (isAmbiguous(this.known, path)) {
      this.quoted(path);

      return;
    }

    this.parts.push({ kind: 'path', path, place });
  }

  size(bytes: number): void {
    this.text(formatSize(bytes, this.units));
  }

  /** Text of the sheet (a note) with the paths it holds as segments, so that a path inside it is shown like any other. */
  sheetText(text: string): void {
    for (const segment of segmentsOf(text, this.known)) {
      if (segment.kind === 'text') {
        this.text(segment.text);
      } else {
        this.parts.push(segment);
      }
    }
  }

  /** A row as the answer names it: its path, then its size in brackets, or `(could not be read)` where it is unknown. */
  row({ index, row }: Indexed): void {
    this.path(row.path, { at: 'row', index });

    if (row.unreadable) {
      if (this.markUnreadable) {
        this.text(' (could not be read)');
      }

      return;
    }

    this.text(' (');
    this.size(row.size);
    this.text(')');
  }

  /** The first rows as a list, `a, b and c`, then how many more there are (`more` are not rows). */
  rows(rows: readonly Indexed[], more: number): void {
    const named = Math.min(rows.length, NAMED_ROWS);
    const left = rows.length - named + more;
    const count = named + (left > 0 ? 1 : 0);

    rows.slice(0, named).forEach((entry, at) => {
      this.separator(at, count);
      this.row(entry);
    });

    if (left > 0) {
      this.separator(named, count);
      this.text(`${formatCount(left)} more`);
    }
  }

  /** What goes before the item `at` of `count`: nothing, a comma or `and`. */
  separator(at: number, count: number): void {
    if (at === 0) {
      return;
    }

    this.text(at + 1 === count ? ' and ' : ', ');
  }
}

/** What a verdict says of a folder the question named. */
type Group = 'safe' | 'review' | 'kept';

/** The written form of the one intent the sheet is for. */
class Writer {
  constructor(
    private readonly sheet: FactSheet,
    private readonly known: readonly KnownPath[],
    private readonly units: SizeUnits,
  ) {}

  line(): Line {
    return new Line(this.known, this.units);
  }

  /** A sentence that is only text. */
  said(text: string): Segment[] {
    const line = this.line();

    line.text(text);

    return line.parts;
  }

  /** A note of the sheet, as it is. */
  note(note: string): Segment[] {
    const text = note.trim();
    const line = this.line();

    line.sheetText(text);

    if (!/[.!?]$/.test(text)) {
      line.text('.');
    }

    return line.parts;
  }

  private rows(): Indexed[] {
    return this.sheet.rows.map((row, index) => ({ index, row }));
  }

  private rowsWith(verdict: Verdict): Indexed[] {
    return this.rows().filter(({ row }) => row.verdict === verdict);
  }

  private totalOf(kind: TotalKind, verdict?: Verdict): FactTotal | undefined {
    return this.sheet.totals.find(
      (total) => total.kind === kind && (verdict === undefined || total.verdict === verdict),
    );
  }

  /** A path segment bound to the row with the scope's path when there is one, to the scope otherwise. */
  private scopePlace(): Place {
    const index = this.sheet.rows.findIndex((row) => row.path === this.sheet.scope);

    return index >= 0 ? { at: 'row', index } : { at: 'scope' };
  }

  /** Where the question was about: ` in “~/Projekty”`, ` in this scan` or ` among the “Docker data” folders`. */
  private inScope(line: Line): void {
    if (this.sheet.scope !== '') {
      line.text(' in ');
      line.path(this.sheet.scope, this.scopePlace());
    } else if (this.sheet.set !== undefined) {
      line.text(' among the ');
      line.quoted(this.sheet.set);
      line.text(' folders');
    } else {
      line.text(' in this scan');
    }
  }

  /** ` in “scope”` when the question named a scope; nothing for the whole scan (the lists say it all). */
  private inScopeNamed(line: Line): void {
    if (this.sheet.scope !== '' || this.sheet.set !== undefined) {
      this.inScope(line);
    }
  }

  /** The folder asked about, as the subject of a sentence: `This scan` or `“~/Projekty”`. */
  private subject(line: Line): void {
    if (this.sheet.scope === '') {
      line.text('This scan');
    } else {
      line.path(this.sheet.scope, this.scopePlace());
    }
  }

  /** `Subject holds 31 GB in 120,345 files.` */
  private holds(): Segment[] | null {
    const { scopeSize, files } = this.sheet;

    if (scopeSize === undefined) {
      return null;
    }

    const line = this.line();

    this.subject(line);
    line.text(' holds ');
    line.size(scopeSize);

    if (files !== undefined) {
      line.text(` in ${formatCount(files)} ${files === 1 ? 'file' : 'files'}`);
    }

    line.text('.');

    return line.parts;
  }

  /** `Label: 12 GB in 4 items.` */
  private total(total: FactTotal): Segment[] | null {
    const line = this.line();

    line.text(total.label);
    line.text(': ');

    // A total can count files too (a matched file is counted though it is not listed): "items", never "folders".
    if (total.bytes !== undefined && total.count !== undefined) {
      line.size(total.bytes);
      line.text(` in ${items(total.count)}`);
    } else if (total.bytes !== undefined) {
      line.size(total.bytes);
    } else if (total.count !== undefined) {
      line.text(formatCount(total.count));
    } else {
      return null;
    }

    line.text('.');

    return line.parts;
  }

  body(): Segment[][] {
    switch (this.sheet.intent) {
      case 'largest':
        return this.largest();
      case 'cleanable':
        return this.cleanable();
      case 'labelled':
        return this.labelled();
      case 'explain':
        return this.explain();
      case 'find':
        return this.find();
      case 'general':
        return this.general();
      case 'offtopic':
        return [];
    }
  }

  /** The sentence for a question that found nothing and has no note to say why. */
  nothing(): Segment[] {
    const line = this.line();

    switch (this.sheet.intent) {
      case 'largest':
        line.text(
          this.sheet.category === undefined
            ? `No ${FAMILY[this.sheet.kind][1]} were found`
            : `No ${this.sheet.category} folders were found`,
        );
        this.inScope(line);
        break;
      case 'cleanable':
        line.text('Nothing to clean was found');
        this.inScope(line);
        break;
      case 'labelled':
        line.text('The model has not labelled any folder');
        this.inScope(line);
        break;
      case 'explain':
        line.text('There is nothing to report');
        this.inScope(line);
        break;
      case 'find':
        line.text('No matching folder was found');
        break;
      case 'general':
      case 'offtopic':
        line.text('No folders were found in this scan');
        break;
    }

    line.text('.');

    return line.parts;
  }

  // ---- The intents ----

  /**
   * `The biggest folder in “~/Projekty” is “a” (7.9 GB), then “b” (3.3 GB) and “c” (1 GB).`
   *
   * About the whole scan the biggest folders come first and the size of the scan follows. About one thing (a folder, the
   * folders of a rule, a media category) its size comes first, because a question about it (`how big is my docker
   * data?`) is a question about that.
   */
  private largest(): Segment[][] {
    const rows = this.rows();
    const aboutOneThing =
      this.sheet.scope !== '' || this.sheet.set !== undefined || this.sheet.category !== undefined;
    const sentences: Segment[][] = [];
    const sizes: Segment[][] = [];

    if (this.sheet.category !== undefined) {
      const media = this.mediaTotal(this.sheet.category);

      if (media) {
        sizes.push(media);
      }
    } else {
      sizes.push(...this.figures());
    }

    if (aboutOneThing) {
      sentences.push(...sizes.splice(0));
    }

    const [first, ...rest] = rows;

    if (first) {
      const line = this.line();

      line.text(`The biggest ${this.familyName()[0]}`);
      this.inScope(line);
      line.text(' is ');
      line.row(first);

      if (rest.length > 0) {
        // The first row is named on its own, so one less is named after it.
        const named = Math.min(rest.length, NAMED_ROWS - 1);

        line.text(', then ');
        line.rows(rest.slice(0, named), rest.length - named);
      }

      line.text('.');
      sentences.push(line.parts);
    }

    return [...sentences, ...sizes];
  }

  /** The family of folders asked about, in the singular and the plural: a media category when there is one. */
  private familyName(): [string, string] {
    const { category } = this.sheet;

    return category === undefined ? FAMILY[this.sheet.kind] : [`${category} folder`, `${category} folders`];
  }

  /** `Video folders in this scan take 2.9 GB in total (33 folders).` */
  private mediaTotal(category: string): Segment[] | null {
    const total = this.sheet.totals[0];

    if (total?.bytes === undefined) {
      return null;
    }

    const line = this.line();

    line.text(`${category.charAt(0).toUpperCase()}${category.slice(1)} folders`);
    this.inScope(line);
    line.text(' take ');
    line.size(total.bytes);
    line.text(' in total');

    if (total.count !== undefined) {
      line.text(` (${formatCount(total.count)} ${total.count === 1 ? 'folder' : 'folders'})`);
    }

    line.text('.');

    return line.parts;
  }

  /** The size of the scan, then every total of the sheet. */
  private figures(): Segment[][] {
    const sentences: Segment[][] = [];
    const holds = this.holds();

    if (holds) {
      sentences.push(holds);
    }

    for (const total of this.sheet.totals) {
      const sentence = this.total(total);

      if (sentence) {
        sentences.push(sentence);
      }
    }

    return sentences;
  }

  private cleanable(): Segment[][] {
    if (this.sheet.subject && this.sheet.rows.length > 0) {
      return this.verdictOfSubject();
    }

    const sentences: Segment[][] = [];
    let first = true;

    for (const verdict of ['safe', 'review', 'keep'] as const) {
      const ofVerdict = this.rowsWith(verdict);
      const total = this.totalOf('verdict', verdict);

      if (ofVerdict.length === 0 && total === undefined) {
        continue;
      }

      const line = this.line();

      line.text(VERDICT_LABEL[verdict]);

      if (first) {
        this.inScopeNamed(line);
        first = false;
      }

      line.text(':');
      this.listed(line, ofVerdict, total);
      sentences.push(line.parts);
    }

    // Nothing to clean in a folder: what it is, so the answer is about the folder and not about nothing. The notes say
    // where Platter stands on it.
    if (sentences.length === 0 && this.sheet.scope !== '') {
      const holds = this.holds();

      if (holds) {
        sentences.push(holds);
      }
    }

    return sentences;
  }

  /**
   * ` 6.4 GB in 3 items, the largest being “a” (4 GB) and “b” (2 GB).`
   *
   * The rows that are a small part of the total are not named one by one: they are counted with the items that are not
   * rows (`and 4,674 smaller items such as “__pycache__”`).
   */
  private listed(line: Line, rows: readonly Indexed[], total: FactTotal | undefined): void {
    const counted =
      total?.bytes !== undefined && total.count !== undefined
        ? { bytes: total.bytes, count: total.count }
        : null;

    if (counted && counted.count === 1 && rows.length === 1) {
      // A folder that is the whole total needs no total.
      line.text(' ');
      line.rows(rows, 0);
    } else if (counted && rows.length > 0) {
      line.text(' ');
      line.size(counted.bytes);
      line.text(` in ${items(counted.count)}`);

      // A row is named when it is at least 2% of the total; the first one always is.
      const named = rows
        .filter((entry, at) => at === 0 || entry.row.size * 50 >= counted.bytes)
        .slice(0, NAMED_ROWS);
      const left = Math.max(0, counted.count - named.length);
      const leftBytes = Math.max(
        0,
        counted.bytes - named.reduce((bytes, entry) => bytes + entry.row.size, 0),
      );
      // What is left is small when the average of it is under 2% of the total.
      const small = left > 0 && leftBytes * 50 < counted.bytes * left;

      line.text(left > 0 ? ', the largest being ' : ', namely ');

      const listed = named.length + (left > 0 ? 1 : 0);

      named.forEach((entry, at) => {
        line.separator(at, listed);
        line.row(entry);
      });

      if (left > 0) {
        line.separator(named.length, listed);
        line.text(`${formatCount(left)} ${small ? 'smaller' : 'more'} ${left === 1 ? 'item' : 'items'}`);

        const unnamed = rows.find((entry) => !named.some((one) => one.index === entry.index));
        const example = total?.example ?? (unnamed ? baseName(unnamed.row.path) : undefined);

        if (small && example !== undefined && example !== '') {
          line.text(' such as ');
          line.quoted(example);
        }
      }
    } else if (counted) {
      line.text(' ');
      line.size(counted.bytes);
      line.text(` in ${items(counted.count)}`);
    } else {
      line.text(' ');
      line.rows(rows, 0);
    }

    line.text('.');
  }

  /** The answer to a question about a thing itself: `Yes: “a” (4 GB) is safe to clean (Rust build output).` */
  private verdictOfSubject(): Segment[][] {
    const rows = this.rows();
    // A row with no verdict is no one's to call safe: it is for a look.
    const groupOf = (row: FactRow): Group =>
      row.verdict === 'safe' ? 'safe' : row.verdict === 'keep' ? 'kept' : 'review';
    const groups = (['safe', 'review', 'kept'] as const)
      .map((group) => ({ group, rows: rows.filter(({ row }) => groupOf(row) === group) }))
      .filter((entry) => entry.rows.length > 0);

    const line = this.line();
    const only = groups.length === 1 ? groups[0]?.group : undefined;

    line.markUnreadable = false;

    line.text(
      only === 'safe'
        ? 'Yes: '
        : only === 'review'
          ? 'No, not without a look: '
          : only === 'kept'
            ? 'No: '
            : 'Partly: ',
    );

    groups.forEach((entry, at) => {
      line.separator(at, groups.length);
      this.clause(line, entry.group, entry.rows);
    });
    line.text('.');

    const sentences = [line.parts];

    // Only the largest are named; the others are said to exist.
    if (this.sheet.more > 0) {
      sentences.push(
        this.said(
          `Only the largest ${rows.length === 1 ? 'one is' : `${rows.length} are`} named here, and ${formatCount(this.sheet.more)} more match.`,
        ),
      );
    }

    return [...sentences, ...this.reasons(rows)];
  }

  /**
   * `“a” (4 GB) is safe to clean`, `“b” is kept`, `“c” is left alone by Platter`, with the rule's title when the clause
   * is about one folder that a rule decided.
   */
  private clause(line: Line, group: Group, rows: readonly Indexed[]): void {
    const one = rows.length === 1;
    const guarded = rows.every(({ row }) => row.by === 'guard');

    line.rows(rows, 0);

    if (group === 'safe') {
      line.text(one ? ' is safe to clean' : ' are safe to clean');
    } else if (group === 'review') {
      line.text(one ? ' needs your review' : ' need your review');
    } else if (guarded) {
      line.text(one ? ' is left alone by Platter' : ' are left alone by Platter');
    } else {
      line.text(one ? ' is kept' : ' are kept');
    }

    this.titled(line, rows);
  }

  /** ` (Rust build output)` after a clause about one folder that a rule decided. */
  private titled(line: Line, rows: readonly Indexed[]): void {
    const [only] = rows;

    if (rows.length !== 1 || !only || (only.row.by !== 'rule' && only.row.by !== 'user')) {
      return;
    }

    const title = only.row.note;

    if (!title) {
      return;
    }

    line.text(' (');
    line.quoted(title);
    line.text(')');
  }

  /** What more to say about the folder the question was about: the rule's own words, why the guard leaves it alone. */
  private reasons(rows: readonly Indexed[]): Segment[][] {
    const sentences: Segment[][] = [];
    const [only] = rows;

    if (rows.length === 1 && only) {
      // The words of a rule or of the guard, unless a note says just that, or the folder is only unreadable (said
      // below), or it is safe (the rule's reason for it is no warning).
      const why = only.row.why;
      const said = (text: string): boolean => this.sheet.notes.some((note) => note.trim() === text.trim());
      const skipped = only.row.verdict === 'safe' || (only.row.by === 'guard' && only.row.unreadable);

      if (why && !skipped && !said(why)) {
        sentences.push(this.note(why));
      }
    }

    const unreadable = rows.filter(({ row }) => row.unreadable);

    if (unreadable.length > 0) {
      if (unreadable.length === 1 && rows.length === 1) {
        sentences.push(this.said('Platter could not read it, so its size is unknown.'));
      } else if (unreadable.length === rows.length) {
        sentences.push(this.said('Platter could not read them, so their sizes are unknown.'));
      } else {
        // Some of them: said by name, so it is clear which.
        const line = this.line();
        const named = unreadable.slice(0, NAMED_ROWS);

        line.text('Platter could not read ');
        named.forEach(({ index, row }, at) => {
          line.separator(at, named.length);
          line.path(row.path, { at: 'row', index });
        });
        line.text(unreadable.length === 1 ? ', so its size is unknown.' : ', so their sizes are unknown.');
        sentences.push(line.parts);
      }
    }

    return sentences;
  }

  /** The model's labels: how many and for what, then which way it marked each (never what it said about it). */
  private labelled(): Segment[][] {
    const sentences: Segment[][] = [];
    const totals = (
      [
        ['review', 'to review'],
        ['keep', 'to keep'],
      ] as const
    ).flatMap(([verdict, words]) => {
      const total = this.totalOf('verdict', verdict);

      return total ? [{ total, words }] : [];
    });
    const count = totals.reduce((sum, entry) => sum + (entry.total.count ?? 0), 0);

    // One folder is said by its own sentence.
    if (count > 1) {
      const line = this.line();

      line.text(`The model labelled ${formatCount(count)} folders`);
      this.inScopeNamed(line);
      line.text(': ');

      totals.forEach(({ total, words }, at) => {
        line.separator(at, totals.length);
        line.text(`${formatCount(total.count ?? 0)} ${words}`);

        if (total.bytes !== undefined) {
          line.text(' (');
          line.size(total.bytes);
          line.text(')');
        }
      });

      line.text('.');
      sentences.push(line.parts);
    }

    for (const entry of this.rows().slice(0, NAMED_ROWS)) {
      const line = this.line();

      line.text('The model marked ');
      line.row(entry);
      line.text(entry.row.verdict === 'keep' ? ' to keep.' : ' for review.');
      sentences.push(line.parts);
    }

    const unnamed = Math.max(0, this.sheet.rows.length - NAMED_ROWS) + this.sheet.more;

    if (unnamed > 0) {
      sentences.push(
        this.said(
          `${formatCount(unnamed)} more labelled ${unnamed === 1 ? 'folder is' : 'folders are'} not listed.`,
        ),
      );
    }

    return sentences;
  }

  /** Size, then what the folder is made of, how old it is, what the rules matched in it and its largest parts. */
  private explain(): Segment[][] {
    const sentences: Segment[][] = [];
    const holds = this.holds();

    if (holds) {
      sentences.push(holds);
    }

    const categories = this.sheet.totals.filter((total) => total.kind === 'category');
    // One type that is nearly all of it says nothing: the folder is what it is.
    const oneType = (categories[0]?.percent ?? 0) >= 95;

    if (categories.length > 0 && !oneType) {
      const line = this.line();

      line.text(categories.length === 1 ? 'The largest part by type is ' : 'The largest parts by type are ');

      categories.forEach((total, at) => {
        line.separator(at, categories.length);
        line.text(total.label);
        line.text(' (');

        if (total.percent !== undefined) {
          line.text(`${total.percent}%`);
        }

        if (total.bytes !== undefined) {
          line.text(total.percent !== undefined ? ', ' : '');
          line.size(total.bytes);
        }

        line.text(')');
      });

      line.text('.');
      sentences.push(line.parts);
    }

    const older = this.totalOf('older');

    if (older) {
      const line = this.line();

      if (older.bytes === 0) {
        line.text('None of it is older than a year.');
      } else if (older.bytes !== undefined && older.percent === 0) {
        line.text('Less than 1% of it (');
        line.size(older.bytes);
        line.text(') is older than a year.');
      } else if (older.bytes !== undefined && older.percent !== undefined) {
        line.text(`${older.percent}% of it (`);
        line.size(older.bytes);
        line.text(') is older than a year.');
      } else {
        line.text('Some of it is older than a year.');
      }

      sentences.push(line.parts);
    }

    const rules = this.sheet.totals.filter((total) => total.kind === 'rule');

    if (rules.length > 0) {
      const line = this.line();

      line.text("Platter's rules match ");

      rules.forEach((total, at) => {
        line.separator(at, rules.length);
        line.quoted(total.label);

        if (total.bytes !== undefined) {
          line.text(' (');
          line.size(total.bytes);
          line.text(')');
        }
      });

      line.text(' inside it.');
      sentences.push(line.parts);
    }

    const rows = this.rows();

    if (rows.length > 0) {
      const line = this.line();

      line.text(rows.length === 1 ? 'The biggest part is ' : 'The biggest parts are ');
      line.rows(rows, 0);
      line.text('.');
      sentences.push(line.parts);
    }

    return sentences;
  }

  /** `Matches for “node_modules”: “a” (1 GB), “b” (2 GB) and 3 more.` */
  private find(): Segment[][] {
    const rows = this.rows();

    if (rows.length === 0) {
      return [];
    }

    const line = this.line();

    if (this.sheet.name === '') {
      line.text('Folders found: ');
    } else {
      line.text('Matches for ');
      line.quoted(this.sheet.name);
      line.text(': ');
    }

    line.rows(rows, this.sheet.more);
    line.text('.');

    return [line.parts];
  }

  /** The biggest folders, the size of the scan and what the suggestions total. */
  private general(): Segment[][] {
    const sentences: Segment[][] = [];
    const rows = this.rows();

    if (rows.length > 0) {
      const line = this.line();

      line.text(rows.length === 1 ? 'The biggest folder is ' : 'The biggest folders are ');
      line.rows(rows, 0);
      line.text('.');
      sentences.push(line.parts);
    }

    return [...sentences, ...this.figures()];
  }
}

/**
 * The answer written from the sheet alone, in pieces. The first sentence answers the question (see the writer of each
 * intent), then come the figures the sheet holds and its notes, each written as it is; an outdated scan says so last.
 */
export function templateSegments(original: FactSheet, units: SizeUnits): Segment[] {
  if (original.intent === 'offtopic') {
    return [{ kind: 'text', text: OFFTOPIC_TEXT }];
  }

  const sheet = shown(original);
  const known = knownPaths(original, sheet);
  const writer = new Writer(sheet, known, units);
  const sentences = writer.body();

  // A note may be the reason there is nothing to list; then it says so by itself. What the model has not labelled is said
  // here, and its note only says what to do about it.
  if (sentences.length === 0 && (sheet.notes.length === 0 || sheet.intent === 'labelled')) {
    sentences.push(writer.nothing());
  }

  for (const note of sheet.notes) {
    sentences.push(writer.note(note));
  }

  if (sheet.outdated) {
    sentences.push(writer.note(OUTDATED_NOTE));
  }

  const segments: Segment[] = [];

  sentences.forEach((sentence, at) => {
    if (at > 0) {
      pushText(segments, ' ');
    }

    for (const part of sentence) {
      if (part.kind === 'text') {
        pushText(segments, part.text);
      } else {
        segments.push(part);
      }
    }
  });

  return segments;
}
