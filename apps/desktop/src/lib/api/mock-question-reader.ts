import {
  quotedPath,
  roleOfName,
  signals,
  wordsOfQuestion,
  type MediaKind,
  type Role,
} from './mock-question-signals';
import {
  isGenericWord,
  isScopeWord,
  isSkippedWord,
  rulesForWords,
  titleWordsOf,
  wordsOf,
} from './mock-scope-words';
import { BUILTIN_RULE_NAMES, type RuleName } from './mock-suggestions';
import type { ScanIntent, ScanKind } from './types';

/*
 * The mock's reading of a question, in the backend's order (`read` in `question/reading.rs`): the words come first
 * (`readQuestion`, over `mock-question-signals.ts`: an intent word, a kind, a media word, a typed path, a personal
 * folder, a tool of the rules, a number of rows), and the model only fills what they leave open. The mock has no model:
 * its keyword reader (`interpret`) stands in for the model's part. It is not a language model: enough for each intent, a
 * scope that is a path (`~/...`, `/...`), a name after "in", "of", "under", "called"... or a word that is the name of a
 * folder of the tree or of a built-in rule (with the words of its name that follow it: "docker images", "Steam shader
 * cache"), a kind word and a number for the limit; anything with no disk word in it is off topic. What it reads is
 * grounded as the backend does (`ground`): a scope the question does not hold as whole words is dropped. A question the
 * words settle (`complete`) is answered without it, and at once.
 */

/** How many rows a question gets when it names no number, and the bounds of the number it names. */
export const DEFAULT_LIMIT = 5;
export const MAX_LIMIT = 10;

/** What the keyword reading makes of a question: the backend's `ScanQuestion`. */
export interface ScanQuery {
  intent: ScanIntent;
  kind: ScanKind;
  limit: number;
  /** A path, a folder name, or (for `find`) the name searched; empty means the whole scan. */
  scope: string;
}

const TOPIC_WORDS =
  /\b(overview|summary|summari[sz]e|scan|disk|drive|storage|space|full|usage|used|status|everything|folders?|directory|directories|files?|keep|kept|protected|safe|old|gb|gib|mb|tb)\b/;
const LABELLED_WORDS = /\blabel(?:l?ed|s)?\b/;
const EXPLAIN_WORDS = /\bwhy\b|\bexplain\b|\bwhat(?:'s| is) (?:in|inside)\b|\bso (?:big|large|huge)\b/;
const FIND_WORDS = /\b(find|locate|look for|search for)\b/;
const CLEAN_WORDS =
  /\bclean|\bdelete|\bremove|\bfree up\b|\bwipe\b|\bget rid\b|\bsafe to\b|\btrash\b|\busu[nń]|\bwyczy[sś]/;
const LARGEST_WORDS =
  /\b(big|bigger|biggest|large|larger|largest|huge|heaviest|most|top|how much|take|takes|taking|size|sizes)\b/;
const BIGNESS_WORDS = /\b(big|biggest|large|largest|most)\b/;

const CACHE_WORDS = /\bcach(?:e|es)\b/;
const BUILD_WORDS = /\b(builds?|artifacts?|dependencies|target)\b/;
const MEDIA_WORDS = /\b(videos?|movies?|films?|photos?|pictures?|images?|music|songs?|audio|media)\b/;

/** Words that follow "in", "of"... but name no folder. */
const NOT_A_FOLDER: ReadonlySet<string> = new Set([
  'a',
  'all',
  'an',
  'any',
  'big',
  'amount',
  'biggest',
  'cache',
  'caches',
  'clean',
  'data',
  'disk',
  'drive',
  'entire',
  'everything',
  'file',
  'files',
  'folder',
  'folders',
  'general',
  'it',
  'largest',
  'me',
  'model',
  'most',
  'my',
  'order',
  'overview',
  'scan',
  'size',
  'space',
  'stuff',
  'the',
  'them',
  'there',
  'things',
  'this',
  'total',
  'whole',
]);

/** Words of a kind: a scope never starts from these (`.cache` is a name, `cache` is a kind). */
const KIND_WORDS: ReadonlySet<string> = new Set([
  'build',
  'builds',
  'artifact',
  'artifacts',
  'dependencies',
  'target',
  'video',
  'videos',
  'movie',
  'movies',
  'film',
  'films',
  'photo',
  'photos',
  'picture',
  'pictures',
  'image',
  'images',
  'music',
  'song',
  'songs',
  'audio',
  'media',
]);

const TOKEN = /[^\s?,;!"'()]+/g;
const SCOPE_PHRASE =
  /\b(?:in|inside|under|within|of|from|for|called|named)\s+(?:the\s+|my\s+|this\s+)?(?:folder\s+|directory\s+|dir\s+)?([^\s?,;!"'()]+)/gi;
const FIND_PHRASE =
  /(?:where\s+(?:is|are|does|do|can\s+i\s+find|did\s+i\s+put)|find|locate|look\s+for|search\s+for)\s+(?:me\s+)?(?:all\s+)?(?:my\s+|the\s+|any\s+)?(?:folders?\s+(?:named|called)\s+)?([^\s?,;!"'()]+)/i;

const clean = (word: string): string => word.replace(/[.:]+$/, '');

/** The first path in the question (`~`, `~/...`, `/...`) with no trailing punctuation. */
function pathIn(text: string): string | null {
  // A path with white space in it is typed in straight quotes: one path, the quotes left out.
  const quoted = quotedPath(text);

  if (quoted !== null) {
    return quoted.path;
  }

  for (const match of text.matchAll(/(?:^|\s)(~(?:\/[^\s?,;!"'()]*)?|\/[^\s?,;!"'()]*)/g)) {
    const found = clean(match[1] as string);

    if (found !== '') {
      return found;
    }
  }

  return null;
}

function limitIn(lower: string): number {
  const match =
    /\b(?:top|first)\s+(\d+)\b/.exec(lower) ??
    /\b(\d+)\s+(?:biggest|largest|folders|caches|items|things|files|videos|largest)\b/.exec(lower);
  const wanted = match ? Number(match[1]) : DEFAULT_LIMIT;

  return Math.min(MAX_LIMIT, Math.max(1, wanted));
}

/** The kind the words of the question stand for; the scope (`~/.cache`, "Steam shader cache") is a folder, not a kind. */
function kindIn(lower: string, scope: string): ScanKind {
  // "docker images" and "disk images" name a thing, not the pictures of the media kind.
  const words = (scope === '' ? lower : lower.replace(scope.toLowerCase(), ' ')).replace(
    /\b(?:docker|container|disk|vm|virtual machine)\s+images?\b/g,
    ' ',
  );

  if (CACHE_WORDS.test(words)) {
    return 'cache';
  }

  if (BUILD_WORDS.test(words)) {
    return 'build';
  }

  return MEDIA_WORDS.test(words) ? 'media' : 'any';
}

/** The name `find` looks for, or `null` when the question names none. */
function findNameIn(text: string): string | null {
  const name = clean(FIND_PHRASE.exec(text)?.[1] ?? '');
  const lower = name.toLowerCase();

  return name === '' || NOT_A_FOLDER.has(lower) ? null : name;
}

/**
 * The name of a tool or place the rules know, as the question writes it: the word at `at` and the words right after
 * it that are in the name of a rule too (`docker images`, `Steam shader cache`, `steam files`).
 */
function nameAt(
  text: string,
  tokens: readonly { word: string; start: number; end: number }[],
  at: number,
): string {
  const first = tokens[at] as (typeof tokens)[number];
  let last = first;

  for (const next of tokens.slice(at + 1, at + 4)) {
    const lower = next.word.toLowerCase();

    if (!/^\s+$/.test(text.slice(last.end, next.start)) || !(isScopeWord(lower) || isGenericWord(lower))) {
      break;
    }

    last = next;
  }

  return clean(text.slice(first.start, last.end));
}

/** The folder the question is about: a path, a name after "in"/"of"/..., a folder name of the tree, or home. */
function scopeIn(text: string, folderNames: ReadonlySet<string>): string {
  const path = pathIn(text);

  if (path !== null) {
    return path;
  }

  for (const match of text.matchAll(SCOPE_PHRASE)) {
    const word = clean(match[1] as string);
    const lower = word.toLowerCase();

    if (lower === 'home') {
      return '~';
    }

    if (word !== '' && !NOT_A_FOLDER.has(lower) && !KIND_WORDS.has(lower)) {
      return word;
    }
  }

  const tokens = Array.from(text.matchAll(TOKEN), (match) => ({
    word: clean(match[0]),
    start: match.index,
    end: match.index + match[0].length,
  }));

  for (const [at, token] of tokens.entries()) {
    const lower = token.word.toLowerCase();

    if (
      (folderNames.has(lower) || isScopeWord(lower)) &&
      !NOT_A_FOLDER.has(lower) &&
      !KIND_WORDS.has(lower)
    ) {
      return isScopeWord(lower) ? nameAt(text, tokens, at) : token.word;
    }
  }

  return /\bhome\b/i.test(text) ? '~' : '';
}

/**
 * Reads a question by keywords into what the backend's stage A and grounding make of it. The first intent whose
 * words are in the question wins: labelled, explain, find, cleanable, largest, general; a question with none of the
 * disk words is off topic. `folderNames` are the lower-case folder names of the tree (a bare name is only a scope
 * when it is one of them or follows "in", "of", "under"...).
 */
export function interpret(question: string, folderNames: ReadonlySet<string>): ScanQuery {
  const text = question.trim();
  const lower = text.toLowerCase();
  const limit = limitIn(lower);
  const answer = (intent: ScanIntent, scope: string, kind: ScanKind = 'any'): ScanQuery => ({
    intent,
    kind: intent === 'largest' || intent === 'cleanable' ? kind : 'any',
    limit,
    scope,
  });

  if (LABELLED_WORDS.test(lower)) {
    return answer('labelled', scopeIn(text, folderNames));
  }

  if (EXPLAIN_WORDS.test(lower)) {
    return answer('explain', scopeIn(text, folderNames));
  }

  if (/\bwhere\b/.test(lower) || (FIND_WORDS.test(lower) && !BIGNESS_WORDS.test(lower))) {
    const name = findNameIn(text);

    if (name !== null) {
      return answer('find', name);
    }
  }

  if (CLEAN_WORDS.test(lower) || LARGEST_WORDS.test(lower)) {
    const scope = scopeIn(text, folderNames);

    return answer(CLEAN_WORDS.test(lower) ? 'cleanable' : 'largest', scope, kindIn(lower, scope));
  }

  if (TOPIC_WORDS.test(lower)) {
    return answer('general', scopeIn(text, folderNames));
  }

  // A path is about the scan, whatever else the question says.
  const path = pathIn(text);

  return path === null ? answer('offtopic', '') : answer('general', path);
}

/** Words that name no folder, so a scope that is only one of them is no scope (`STOP_WORDS` in `platter-ai`'s `question.rs`). */
const GROUNDING_STOP_WORDS: ReadonlySet<string> = new Set([
  'a',
  'all',
  'an',
  'and',
  'any',
  'are',
  'as',
  'at',
  'big',
  'can',
  'do',
  'does',
  'file',
  'files',
  'folder',
  'folders',
  'for',
  'from',
  'how',
  'i',
  'if',
  'in',
  'is',
  'it',
  'me',
  'most',
  'much',
  'my',
  'of',
  'on',
  'or',
  'show',
  'space',
  'take',
  'takes',
  'that',
  'the',
  'them',
  'there',
  'these',
  'this',
  'those',
  'to',
  'up',
  'us',
  'what',
  'where',
  'which',
  'why',
  'with',
  'you',
  'your',
]);

const WORD_CHARACTER = /[\p{L}\p{N}_]/u;

/** Whether `needle` is in `text` and neither begins nor ends in the middle of a word. */
function containsWhole(text: string, needle: string): boolean {
  for (let at = text.indexOf(needle); at !== -1; at = text.indexOf(needle, at + 1)) {
    const before = Array.from(text.slice(0, at)).at(-1);
    const after = Array.from(text.slice(at + needle.length))[0];

    if (
      !(before !== undefined && WORD_CHARACTER.test(before)) &&
      !(after !== undefined && WORD_CHARACTER.test(after))
    ) {
      return true;
    }
  }

  return false;
}

function scopeIsBacked(question: string, scope: string): boolean {
  const text = scope.trim();

  if (text === '') {
    return false;
  }

  const lower = question.toLowerCase();

  if (text === '/') {
    return lower.includes('/');
  }

  const bare = (text.startsWith('~/') ? text.slice(2) : text).replace(/\/+$/, '');

  if (bare === '' || bare === '~') {
    return lower.includes('home') || lower.includes('~');
  }

  const wanted = bare.toLowerCase();

  return Array.from(wanted).length >= 2 && !GROUNDING_STOP_WORDS.has(wanted) && containsWhole(lower, wanted);
}

/**
 * The backend's grounding (`ground` in `question.rs`): the scope stays only when the question contains it as whole
 * words (ignoring case, a leading `~/` and a trailing `/`), at least 2 characters and not a stop word; `~` when the
 * question says `home` or `~`, `/` when it has a `/`. The kind only means something for `largest`, `cleanable` and `find`.
 */
export function ground(question: string, asked: ScanQuery): ScanQuery {
  const kindCounts = asked.intent === 'largest' || asked.intent === 'cleanable' || asked.intent === 'find';

  return {
    ...asked,
    kind: kindCounts ? asked.kind : 'any',
    scope: scopeIsBacked(question, asked.scope) ? asked.scope : '',
  };
}

// ---- The words first ----

/** What a question names. */
export type Subject =
  /** Nothing: the whole scan. */
  | { kind: 'none' }
  /** A path the user typed (`~/Projekty`, `/var`, `~`). */
  | { kind: 'path'; text: string }
  /** A personal folder by its role, in English or Polish (`Downloads`, `Pobrane`), and the name used. */
  | { kind: 'role'; role: Role; typed: string }
  /** A tool the rules know (`docker`, `steam`, `node_modules`): the rules it names and the words that named them. */
  | { kind: 'rules'; said: string; ids: string[] }
  /** Words read as a folder name, that no rule, path or personal folder explains. */
  | { kind: 'name'; text: string };

/** What the question comes to, once the words and the model are put together. */
export interface Reading {
  query: ScanQuery;
  subject: Subject;
  /** A media category, for the `media` kind. */
  media: MediaKind | null;
  polish: boolean;
  everything: boolean;
  /** The words alone decided the intent and the subject, so the model was not needed. */
  complete: boolean;
}

/**
 * The tools of the rules by the words people use for them (and Polish ones): the rule's id, and the words that name it
 * without a doubt. A word every rule has (`cache`) or one that names many things (`image`) is not here.
 */
const TOOLS: readonly (readonly [string, readonly string[]])[] = [
  ['apt-cache', ['apt']],
  ['dnf-cache', ['dnf']],
  ['rust-target', ['rust']],
  ['rust-target-recent', ['rust']],
  ['node-modules', ['node', 'nodejs']],
  ['node-modules-recent', ['node', 'nodejs']],
  ['python-cache', ['python', 'pycache', 'pip']],
  ['npm-cache', ['npm']],
  ['bun-cache', ['bun']],
  ['pnpm-store', ['pnpm']],
  ['yarn-cache', ['yarn']],
  ['cargo-registry', ['cargo']],
  ['gradle-cache', ['gradle']],
  ['nuget-packages', ['nuget']],
  ['go-modules', ['golang']],
  ['trash', ['trash', 'kosz', 'recycle', 'wastebasket']],
  ['coredumps', ['coredump', 'coredumps']],
  ['steam-shader-cache', ['steam', 'shader']],
  ['steam-library', ['steam', 'game']],
  ['pacman-cache', ['pacman']],
  ['journal', ['journal', 'journald']],
  ['maven-repository', ['maven']],
  ['docker-data', ['docker', 'container', 'kontener']],
  ['docker-desktop-vm', ['docker', 'container', 'kontener']],
  ['btrfs-snapshots', ['btrfs', 'snapper', 'snapshot']],
];

/** Whether a word of the question is `tool`: the same word, or (for a tool of five letters or more) with a short ending. */
const isTool = (token: string, tool: string): boolean =>
  token === tool || (tool.length >= 5 && token.startsWith(tool) && token.length <= tool.length + 3);

const NOT_A_WORD_EDGE = /^[^\p{L}\p{N}_.]+|[^\p{L}\p{N}_.]+$/gu;

/**
 * The rules the tools of the question name: the tools it says by name, then every rule that has them among its own
 * words (`steam library` is the Steam library and not the shader cache), with the words that said it.
 */
function toolRules(question: string, tokens: readonly string[]): { said: string; named: RuleName[] } | null {
  const known = new Set(BUILTIN_RULE_NAMES.map((rule) => rule.id));
  const strong: string[] = [];
  const said: string[] = [];

  for (const token of tokens) {
    const stem = wordsOf(token).pop() ?? token;

    for (const [id, tools] of TOOLS) {
      const tool = tools.find((candidate) => isTool(stem, candidate));

      if (tool === undefined || !known.has(id)) {
        continue;
      }

      // The tool as the rule's own words have it (`kosz` is `trash`) and as it was said.
      for (const word of [tool, tools[0] as string]) {
        if (!strong.includes(word)) {
          strong.push(word);
        }
      }

      if (!said.includes(token)) {
        said.push(token);
      }
    }
  }

  if (strong.length === 0) {
    return null;
  }

  // The other words of the question that are words of those rules' titles narrow the choice.
  const words = [...strong];

  for (const token of tokens) {
    for (const stem of wordsOf(token)) {
      const isRole = roleOfName(token) !== null;
      const isTitleWord = BUILTIN_RULE_NAMES.some((rule) => titleWordsOf(rule).includes(stem));

      if (
        !isRole &&
        isTitleWord &&
        Array.from(stem).length >= 2 &&
        !isSkippedWord(stem) &&
        !words.includes(stem)
      ) {
        words.push(stem);
      }
    }
  }

  const named = rulesForWords(words);

  if (named.length === 0) {
    return null;
  }

  // The words as they were written (`node_modules`, `dockera`), not as they were split.
  const written = question
    .split(/\s+/u)
    .map((word) => word.replace(NOT_A_WORD_EDGE, ''))
    .filter((word) => word !== '' && wordsOfQuestion(word).some((token) => said.includes(token)));

  return { said: written.length === 0 ? said.join(' ') : written.join(' '), named };
}

const isPath = (scope: string): boolean => scope === '~' || scope.startsWith('~/') || scope.startsWith('/');

/**
 * What the question names and means, from its words and the model's reading of it (`null`: no model).
 *
 * The words win: a clear intent word, a kind word, a media word, a path, a personal folder or a tool of the rules
 * replace what the model read. The model fills the intent when no word says it, a folder name that nothing else
 * explains, a number of rows, and the `media` kind when the words say nothing of the kind. Without any word of disks or
 * cleaning (and no path) the question is off topic.
 */
export function readQuestion(question: string, model: ScanQuery | null): Reading {
  const words = signals(question);
  const read = model === null ? null : ground(question, model);
  const tools = toolRules(question, words.tokens);

  const onTopic = words.aboutDisk || tools !== null;
  const modelOfftopic = read?.intent === 'offtopic';

  if (!onTopic || (modelOfftopic && words.intent === null)) {
    return {
      query: { intent: 'offtopic', scope: '', kind: 'any', limit: DEFAULT_LIMIT },
      subject: { kind: 'none' },
      media: null,
      polish: words.polish,
      everything: false,
      complete: true,
    };
  }

  const modelScope = read && read.scope !== '' ? read.scope : null;
  const firstRole = words.roles[0];
  let subject: Subject;

  if (words.path !== null) {
    subject = { kind: 'path', text: words.path };
  } else if (tools !== null) {
    subject = { kind: 'rules', said: tools.said, ids: tools.named.map((rule) => rule.id) };
  } else if (firstRole !== undefined) {
    const typed = words.tokens.find((token) => roleOfName(token) === firstRole);

    subject = { kind: 'role', role: firstRole, typed: typed ?? firstRole };
  } else if (words.hidden !== null) {
    subject = { kind: 'name', text: words.hidden };
  } else if (modelScope === null) {
    subject = { kind: 'none' };
  } else if (isPath(modelScope)) {
    subject = { kind: 'path', text: modelScope };
  } else {
    const role = roleOfName(modelScope.replace(/\/+$/, ''));

    subject = role === null ? { kind: 'name', text: modelScope } : { kind: 'role', role, typed: modelScope };
  }

  const modelIntent = read && read.intent !== 'offtopic' ? read.intent : null;
  let intent: ScanIntent = words.intent ?? modelIntent ?? 'general';

  // A kind or a media category is a question about a family of folders, never a search for a name.
  const media = subject.kind === 'none' ? words.media : null;
  let kind: ScanKind = 'any';

  if (media !== null) {
    kind = 'media';
  } else if (subject.kind === 'rules') {
    kind = 'any';
  } else if (words.kind !== null) {
    kind = words.kind;
  } else if (read?.kind === 'media' && subject.kind === 'none' && intent === 'largest') {
    kind = 'media';
  }

  if (intent === 'find' && subject.kind === 'none' && kind !== 'any') {
    intent = 'largest';
  }

  const limit = words.limit ?? read?.limit ?? DEFAULT_LIMIT;
  let scope = '';

  switch (subject.kind) {
    case 'none':
      break;
    case 'path':
    case 'name':
      scope = subject.text;
      break;
    case 'role':
      scope = subject.role;
      break;
    case 'rules':
      scope = subject.said;
      break;
  }

  return {
    query: { intent, scope, kind, limit },
    subject,
    media,
    polish: words.polish,
    everything: words.everything,
    complete: words.intent !== null && subject.kind !== 'none' && subject.kind !== 'name',
  };
}
