import { formatCount } from '../../format';

/** The most items a search looks through: more pages than this are not loaded in the background. */
export const SEARCH_LIMIT = 5000;

/** What is searched for: the text as typed, `''` when it is empty or only spaces. */
export function normalizeQuery(text: string): string {
  return text.trim() === '' ? '' : text;
}

/** Whether `name` contains `query`, ignoring case. An empty query matches everything. */
export function matches(name: string, query: string): boolean {
  return query === '' || name.toLowerCase().includes(query.toLowerCase());
}

export interface Segment {
  text: string;
  /** This part of the name is what was searched for. */
  hit: boolean;
}

/** `name` cut into the parts that match `query` (every occurrence, ignoring case) and the parts between them. */
export function segments(name: string, query: string): Segment[] {
  if (query === '') {
    return [{ text: name, hit: false }];
  }

  const haystack = name.toLowerCase();
  const needle = query.toLowerCase();
  const parts: Segment[] = [];

  let from = 0;

  for (let at = haystack.indexOf(needle); at >= 0; at = haystack.indexOf(needle, from)) {
    if (at > from) {
      parts.push({ text: name.slice(from, at), hit: false });
    }

    parts.push({ text: name.slice(at, at + needle.length), hit: true });
    from = at + needle.length;
  }

  if (from < name.length) {
    parts.push({ text: name.slice(from), hit: false });
  }

  return parts;
}

/** "143", "999", "1.2k", "12k": a count short enough for a small label. */
export function abbreviateCount(count: number): string {
  if (count < 1000) {
    return String(Math.max(0, Math.round(count)));
  }

  const thousands = count / 1000;

  return thousands < 10 ? `${(Math.floor(thousands * 10) / 10).toString()}k` : `${Math.floor(thousands)}k`;
}

/**
 * The count in the list's header, from the longest text to the shortest: the header shows the first one that fits.
 * Without a query it is "143 items"; with one it is "12 of 143 items". The word goes first, then the numbers
 * are abbreviated.
 */
export function countLabels(total: number, matched?: number): string[] {
  const noun = total === 1 ? 'item' : 'items';

  if (matched === undefined) {
    return unique([`${formatCount(total)} ${noun}`, abbreviateCount(total)]);
  }

  return unique([
    `${formatCount(matched)} of ${formatCount(total)} ${noun}`,
    `${formatCount(matched)} of ${formatCount(total)}`,
    `${abbreviateCount(matched)} of ${abbreviateCount(total)}`,
    abbreviateCount(matched),
  ]);
}

const unique = (labels: string[]): string[] =>
  labels.filter((label, index) => labels.indexOf(label) === index);
