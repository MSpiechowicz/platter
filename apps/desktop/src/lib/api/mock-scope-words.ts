import { BUILTIN_RULE_NAMES, type RuleName } from './mock-suggestions';

/*
 * How the backend reads a scope text that is no path (`by_rule`, `scope_words`, `rule_hits`, `name_words` and
 * `place_hint` in `question.rs`): the words of the text are scored against each built-in rule's id, its title and the
 * other names it goes by, and the rules with the most words win. The matches of those rules are the folders asked
 * about; `mock-scan-question.ts` finds them in the mock tree.
 */

/** Words that name no folder. */
const STOP_WORDS: readonly string[] = [
  'a',
  'all',
  'an',
  'and',
  'any',
  'are',
  'can',
  'do',
  'does',
  'for',
  'from',
  'in',
  'is',
  'it',
  'me',
  'my',
  'of',
  'on',
  'or',
  'that',
  'the',
  'these',
  'this',
  'those',
  'to',
  'you',
  'your',
];

/** Words so common in the titles of the rules that on their own they name no rule (`cache` is in most of them). */
const GENERIC_WORDS: readonly string[] = [
  'app',
  'apps',
  'build',
  'cache',
  'data',
  'dir',
  'directory',
  'file',
  'files',
  'folder',
  'folders',
  'old',
  'output',
  'package',
  'recent',
  'space',
  'system',
  'temp',
  'tmp',
];

/** Other names a rule goes by, besides the words of its id and title. */
const ALIASES: Readonly<Record<string, readonly string[]>> = {
  'docker-data': ['container', 'image', 'volume'],
  'docker-desktop-vm': ['container', 'image', 'volume'],
  'python-cache': ['pip', 'pycache', 'python'],
  trash: ['bin', 'recycle', 'wastebasket'],
  journal: ['log'],
  coredumps: ['crash', 'dump'],
  'btrfs-snapshots': ['snapper', 'rollback'],
  'steam-library': ['game'],
  'pacman-cache': ['package', 'pkg'],
};

/**
 * The lower-case words of `text` (letters and digits, so `node_modules` is `node` and `module`), a plural `s` cut off
 * so `images` and `image` are one word.
 */
export function wordsOf(text: string): string[] {
  return text
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter((word) => word !== '')
    .map((word) => {
      const stem = word.endsWith('s') ? word.slice(0, -1) : null;

      return stem !== null && Array.from(stem).length > 2 && !stem.endsWith('s') ? stem : word;
    });
}

const SKIPPED: ReadonlySet<string> = new Set([...STOP_WORDS, ...GENERIC_WORDS].flatMap(wordsOf));

/** Whether a (stemmed) word is one that names no rule: a stop word, or one nearly every rule has. */
export const isSkippedWord = (word: string): boolean => SKIPPED.has(word);

const withoutRepeats = <T>(items: readonly T[]): T[] => items.filter((item, at) => item !== items[at - 1]);

/** The words of a scope text that can name a rule: no stop word, no word every rule has, at least two characters. */
export function scopeWords(text: string): string[] {
  return withoutRepeats(wordsOf(text).filter((word) => Array.from(word).length >= 2 && !SKIPPED.has(word)));
}

/** The words of a rule's id and title. */
export const titleWordsOf = (rule: RuleName): string[] => [
  ...wordsOf(rule.id.replaceAll('-', ' ')),
  ...wordsOf(rule.title),
];

/** The words of the other names a rule goes by. */
const aliasWordsOf = (rule: RuleName): string[] => (ALIASES[rule.id] ?? []).flatMap(wordsOf);

/** How many of `words` a rule has, and how many words it has in all (in its id, its title and its other names). */
function ruleHits(rule: RuleName, words: readonly string[]): [number, number] {
  const known = new Set([...titleWordsOf(rule), ...aliasWordsOf(rule)]);

  return [words.filter((word) => known.has(word)).length, known.size];
}

/**
 * The rules that have the most of `words` among their own words (id, title, other names), and of those the ones that
 * fit closest (the share of their own words the words are). Rules of equal standing are all named. Nothing is looked up
 * in a scan; empty when no rule has a word.
 */
export function rulesForWords(words: readonly string[]): RuleName[] {
  const scored = BUILTIN_RULE_NAMES.flatMap((rule) => {
    const [hits, own] = ruleHits(rule, words);

    return hits > 0 ? [{ rule, hits, fit: hits / Math.max(own, 1) }] : [];
  });
  const most = Math.max(0, ...scored.map((entry) => entry.hits));
  const bestFit = Math.max(0, ...scored.filter((entry) => entry.hits === most).map((entry) => entry.fit));

  return scored.filter((entry) => entry.hits === most && entry.fit * 2 >= bestFit).map((entry) => entry.rule);
}

/**
 * The rules a scope text names: the rules with the most of its words win, and of those the ones that fit closest (the
 * share of their own words the text has), so `desktop` is the Desktop folder and not Docker Desktop. Rules of equal
 * standing all count (`docker` is Docker data and the Docker Desktop disk image). Empty when no rule has a word.
 */
export function rulesNamedBy(text: string): RuleName[] {
  const words = scopeWords(text);

  return words.length === 0 ? [] : rulesForWords(words);
}

/** Whether `word` is one a built-in rule goes by (`docker`, `steam`, `trash`, `node_modules`...). */
export const isScopeWord = (word: string): boolean => rulesNamedBy(word).length > 0;

/** Whether `word` is a word no rule is named by alone (`cache`, `files`...): it may still be part of a rule's name. */
export const isGenericWord = (word: string): boolean => GENERIC_WORDS.includes(word.toLowerCase());

/** Words that name no folder, as the scope of a name lookup sees them. */
export const isStopWord = (word: string): boolean => STOP_WORDS.includes(word.toLowerCase());

const NOT_A_NAME_EDGE = /^[^\p{L}\p{N}._]+|[^\p{L}\p{N}._]+$/gu;

/**
 * The words of a scope text that can name a folder, the longest first: no stop word, at least two characters, and not
 * the whole text (one word is the text itself, which was tried whole).
 */
export function nameWords(text: string): string[] {
  const pieces = text.split(/\s+/u).filter((piece) => piece !== '');

  if (pieces.length < 2) {
    return [];
  }

  const words = pieces
    .map((piece) => piece.replace(NOT_A_NAME_EDGE, ''))
    .filter((word) => Array.from(word).length >= 2 && word !== text && !isStopWord(word))
    .sort((a, b) => Array.from(b).length - Array.from(a).length);

  return withoutRepeats(words);
}

/**
 * The words of `text`, as written, that name one of `rules`: by the id or title when any word does (`docker` of
 * `docker images`), else by another name they go by. The whole text when none does.
 */
export function namedWords(text: string, rules: readonly RuleName[]): string {
  const titles = new Set(rules.flatMap(titleWordsOf));
  const aliases = new Set(rules.flatMap(aliasWordsOf));
  const written = text.split(/[^\p{L}\p{N}_]+/u).filter((word) => word !== '');
  const by = (known: ReadonlySet<string>): string[] =>
    written.filter((word) => wordsOf(word).some((stem) => known.has(stem)));
  const byTitle = by(titles);
  const named = byTitle.length > 0 ? byTitle : by(aliases);

  return named.length > 0 ? named.join(' ') : text;
}

/** Where the rule looks, when it has one place: `“Docker data” lives in /var/lib/docker; scan a folder that holds it`. */
export function placeHint(rule: RuleName): string | null {
  if (rule.place === null) {
    return null;
  }

  return rule.place.startsWith('~')
    ? `“${rule.title}” lives in ${rule.place}; scan your home folder`
    : `“${rule.title}” lives in ${rule.place}; scan a folder that holds it`;
}
