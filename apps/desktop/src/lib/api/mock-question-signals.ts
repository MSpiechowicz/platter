import type { ScanIntent, ScanKind } from './types';

/*
 * What the words of a question already say, without any model: a port of `signals` (`crates/platter-ai/src/reading.rs`),
 * English and Polish, letters folded to plain ASCII (`usunąć` and `usunac` are one word). The mock puts these words
 * before its keyword reader (`mock-question-reader.ts`), as the backend puts them before the model: a clear intent word,
 * a kind, a media category, a typed path, a personal folder by its name and a number of rows each decide what they can.
 * Which folder a bare name means, and what the rules call a tool, need the scan and are done by the reader and the
 * mock's resolution.
 */

export type MediaKind = 'video' | 'image' | 'audio';

/** A personal folder with a well-known role, whatever it is called on this machine. */
export type Role =
  'Desktop' | 'Documents' | 'Downloads' | 'Music' | 'Pictures' | 'Public' | 'Templates' | 'Videos';

/** The letters the Polish alphabet adds, written without their marks. */
const FOLDED: Record<string, string> = {
  ą: 'a',
  ć: 'c',
  ę: 'e',
  ł: 'l',
  ń: 'n',
  ó: 'o',
  ś: 's',
  ź: 'z',
  ż: 'z',
};

/** `text` in lower case with the Polish letters written without their marks. */
export function fold(text: string): string {
  return Array.from(text.toLowerCase(), (char) => FOLDED[char] ?? char).join('');
}

/** The role a folder name has, in English or in Polish, whatever the case and the accents (`pobrane`, `Downloads`). */
export function roleOfName(name: string): Role | null {
  switch (fold(name)) {
    case 'desktop':
    case 'pulpit':
      return 'Desktop';
    case 'documents':
    case 'dokumenty':
      return 'Documents';
    case 'downloads':
    case 'pobrane':
      return 'Downloads';
    case 'music':
    case 'muzyka':
      return 'Music';
    case 'pictures':
    case 'obrazy':
      return 'Pictures';
    case 'public':
    case 'publiczny':
      return 'Public';
    case 'templates':
    case 'szablony':
      return 'Templates';
    case 'videos':
    case 'wideo':
      return 'Videos';
    default:
      return null;
  }
}

/** Words a typo is read as: one letter wrong, missing, extra or swapped (five letters or more, so a short word is never taken for another). */
const REPAIRABLE: readonly string[] = [
  'delete',
  'biggest',
  'largest',
  'docker',
  'storage',
  'directory',
  'dlaczego',
  'usunac',
  'wyczysc',
  'najwieksze',
  'najwiekszy',
  'cleanup',
];

/** Whether `a` and `b` differ by one letter put in, left out, changed or swapped with its neighbour. */
function withinOneEdit(a: string, b: string): boolean {
  const left = Array.from(a);
  const right = Array.from(b);

  if (Math.abs(left.length - right.length) > 1) {
    return false;
  }

  let at = 0;

  while (at < left.length && at < right.length && left[at] === right[at]) {
    at++;
  }

  if (at === left.length && at === right.length) {
    return true;
  }

  const ra = left.slice(at);
  const rb = right.slice(at);
  const same = (x: readonly string[], y: readonly string[]): boolean =>
    x.length === y.length && x.every((char, index) => char === y[index]);

  if (left.length === right.length) {
    return (
      same(ra.slice(1), rb.slice(1)) ||
      (ra.length >= 2 && ra[0] === rb[1] && ra[1] === rb[0] && same(ra.slice(2), rb.slice(2)))
    );
  }

  return left.length < right.length ? same(ra, rb.slice(1)) : same(ra.slice(1), rb);
}

function repaired(word: string): string | null {
  if (Array.from(word).length < 5 || REPAIRABLE.includes(word)) {
    return null;
  }

  return REPAIRABLE.find((known) => withinOneEdit(word, known)) ?? null;
}

/** The folded words of a question: letters and digits, everything else separates; a typo of a few words is repaired. */
export function wordsOfQuestion(question: string): string[] {
  return fold(question)
    .split(/[^\p{L}\p{N}]+/u)
    .filter((word) => word !== '')
    .map((word) => repaired(word) ?? word);
}

const hasStem = (tokens: readonly string[], stems: readonly string[]): boolean =>
  tokens.some((token) => stems.some((stem) => token.startsWith(stem)));

const hasWord = (tokens: readonly string[], words: readonly string[]): boolean =>
  tokens.some((token) => words.includes(token));

const hasSequence = (tokens: readonly string[], sequence: readonly string[]): boolean =>
  tokens.some((_, at) => sequence.every((word, index) => tokens[at + index] === word));

/** Words that make a question one about a disk. A stem matches the words that begin with it. */
const DISK_STEMS = [
  'disk',
  'drive',
  'dysk',
  'space',
  'storag',
  'folder',
  'director',
  'katalog',
  'file',
  'plik',
  'size',
  'rozmiar',
  'big',
  'larg',
  'najwiek',
  'wielk',
  'cach',
  'docker',
  'steam',
  'trash',
  'kosz',
  'junk',
  'clean',
  'delet',
  'remov',
  'wipe',
  'purg',
  'empt',
  'usun',
  'wyczys',
  'skasow',
  'skasuj',
  'label',
  'scan',
  'miejsc',
  'zajmuj',
  'takes',
  'taking',
  'usage',
  'eating',
  'hog',
  'overview',
  'summary',
  'podsumow',
  'pamiec',
  'smiec',
  'node',
  'build',
  'etykiet',
  'oznacz',
  'gigabyte',
  'megabyte',
  'terabyte',
  'heav',
];
const DISK_WORDS = ['gb', 'mb', 'tb', 'kb'];

const EXPLAIN_STEMS = ['dlaczego', 'czemu', 'wyjasni'];
const FIND_WORDS = ['where', 'gdzie', 'locate', 'find', 'znajdz', 'lokalizacja', 'lokalizacje'];
const CLEAN_STEMS = [
  'delet',
  'remov',
  'clean',
  'wipe',
  'purg',
  'empt',
  'erase',
  'discard',
  'uninstall',
  'usun',
  'wyczys',
  'skasow',
  'skasuj',
  'kasuj',
  'oprozn',
  'safe',
  'bezpiecz',
  'junk',
  'smiec',
  'zwolni',
  'czyst',
  'wyrzu',
  'pozb',
];
const SIZE_STEMS = [
  'big',
  'larg',
  'najwiek',
  'zajmuj',
  'miejsc',
  'heav',
  'hog',
  'eating',
  'consum',
  'usage',
  'rozmiar',
  'wielk',
];
const SIZE_WORDS = [
  'most',
  'take',
  'takes',
  'taking',
  'use',
  'uses',
  'using',
  'space',
  'much',
  'size',
  'ile',
  'top',
];

/** The first intent word found, in this order: why, label, where, delete, size, overview. */
function intentOf(tokens: readonly string[]): ScanIntent | null {
  const explains =
    hasWord(tokens, ['why', 'explain']) ||
    hasStem(tokens, EXPLAIN_STEMS) ||
    hasSequence(tokens, ['what', 's', 'in']) ||
    hasSequence(tokens, ['what', 'is', 'in']) ||
    hasSequence(tokens, ['whats', 'in']) ||
    hasSequence(tokens, ['what', 'is', 'inside']) ||
    hasSequence(tokens, ['what', 's', 'inside']) ||
    hasSequence(tokens, ['co', 'jest', 'w']) ||
    hasSequence(tokens, ['co', 'siedzi']) ||
    hasSequence(tokens, ['z', 'czego']);

  if (explains) {
    return 'explain';
  }

  if (hasStem(tokens, ['label', 'etykiet', 'oznacz'])) {
    return 'labelled';
  }

  if (hasWord(tokens, FIND_WORDS)) {
    return 'find';
  }

  // "free" alone is not deleting ("free space"): "free up" and "to free" are.
  const frees =
    hasSequence(tokens, ['free', 'up']) ||
    hasSequence(tokens, ['to', 'free']) ||
    hasSequence(tokens, ['get', 'rid']) ||
    hasSequence(tokens, ['throw', 'away']);

  if (hasStem(tokens, CLEAN_STEMS) || frees) {
    return 'cleanable';
  }

  if (hasStem(tokens, SIZE_STEMS) || hasWord(tokens, SIZE_WORDS)) {
    return 'largest';
  }

  if (hasStem(tokens, ['overview', 'summary', 'podsumow', 'przeglad'])) {
    return 'general';
  }

  return null;
}

const MEDIA_ONLY: readonly (readonly [string, MediaKind])[] = [
  ['movie', 'video'],
  ['movies', 'video'],
  ['film', 'video'],
  ['films', 'video'],
  ['filmy', 'video'],
  ['filmow', 'video'],
  ['filmach', 'video'],
  ['photo', 'image'],
  ['photos', 'image'],
  ['zdjecia', 'image'],
  ['zdjec', 'image'],
  ['zdjeciach', 'image'],
  ['fotki', 'image'],
  ['fotografie', 'image'],
  ['song', 'audio'],
  ['songs', 'audio'],
  ['piosenki', 'audio'],
  ['audio', 'audio'],
  ['mp3', 'audio'],
  ['clips', 'video'],
];

/** A folder name that is also a media category (`videos`). */
const MEDIA_OR_FOLDER: readonly (readonly [string, MediaKind, Role])[] = [
  ['videos', 'video', 'Videos'],
  ['wideo', 'video', 'Videos'],
  ['pictures', 'image', 'Pictures'],
  ['obrazy', 'image', 'Pictures'],
  ['music', 'audio', 'Music'],
  ['muzyka', 'audio', 'Music'],
];

const FOLDER_CUES = ['folder', 'folders', 'folderze', 'directory', 'dir', 'katalog', 'katalogu', 'foldery'];

/** The words that name one personal folder and no media category. */
function roleOfToken(token: string): Role | null {
  switch (token) {
    case 'desktop':
    case 'pulpit':
    case 'pulpicie':
      return 'Desktop';
    case 'documents':
    case 'dokumenty':
    case 'dokumentach':
      return 'Documents';
    case 'downloads':
    case 'download':
    case 'pobrane':
    case 'pobranych':
      return 'Downloads';
    case 'templates':
    case 'szablony':
      return 'Templates';
    case 'publiczny':
    case 'publiczne':
      return 'Public';
    default:
      return null;
  }
}

const trimmed = (raw: string): string =>
  raw.replace(/^[?,;:!"'()“”]+|[?,;:!"'()“”]+$/g, '').replace(/\.+$/, '');

/** A word that is a hidden folder by itself (`.cache`), with the punctuation around it cut off. */
function hiddenToken(raw: string): string | null {
  const token = trimmed(raw);
  const second = Array.from(token)[1];

  return token.startsWith('.') && second !== undefined && /\p{L}/u.test(second) && !token.includes('/')
    ? token
    : null;
}

/** A word that is a path (`~/Projekty`, `/var`, `~`). */
function pathToken(raw: string): string | null {
  const token = trimmed(raw);

  if (token === '~' || token.startsWith('~/')) {
    return token;
  }

  const second = Array.from(token)[1];
  const rooted = token.startsWith('/') && second !== undefined && /[\p{L}\p{N}._-]/u.test(second);

  return rooted || token === '/' ? token : null;
}

const LIMIT_WORDS = [
  'top',
  'biggest',
  'largest',
  'folders',
  'folder',
  'caches',
  'items',
  'najwiekszych',
  'foldery',
  'najwieksze',
  'pierwszych',
];

/** A number of rows asked for (`top 5`, `3 biggest`), 1 to 10. */
function limitOf(tokens: readonly string[]): number | null {
  for (const [at, token] of tokens.entries()) {
    const number = /^\d+$/.test(token) ? Number(token) : 0;

    if (number < 1 || number > 10) {
      continue;
    }

    const next = tokens[at + 1];
    const before = tokens[at - 1];

    if ((next !== undefined && LIMIT_WORDS.includes(next)) || before === 'top') {
      return number;
    }
  }

  return null;
}

const POLISH_WORDS = [
  'czy',
  'co',
  'ile',
  'jak',
  'jakie',
  'jaki',
  'gdzie',
  'dlaczego',
  'czemu',
  'ktore',
  'mozna',
  'moge',
  'mam',
  'moje',
  'moj',
  'mojego',
  'miejsca',
  'dysku',
  'zajmuje',
  'zajmuja',
  'usunac',
  'usun',
  'wyczysc',
  'najwieksze',
  'najwiekszy',
  'foldery',
  'pliki',
  'pokaz',
  'kosz',
  'wszystko',
];

/** `my home folder`, `home directory`, `katalog domowy`: the home folder, said in words. */
function homeWord(tokens: readonly string[]): boolean {
  return tokens.some((token, at) => {
    const next = tokens[at + 1];
    const before = tokens[at - 1];

    if (token === 'home') {
      return (
        (next !== undefined && ['folder', 'directory', 'dir', 'drive'].includes(next)) ||
        (before !== undefined && ['my', 'in', 'moj', 'mojego'].includes(before))
      );
    }

    if (token === 'domowy' || token === 'domowym' || token === 'domowego') {
      return before !== undefined && ['katalog', 'katalogu', 'folder'].includes(before);
    }

    return false;
  });
}

/** What the words of a question say by themselves. */
export interface Signals {
  /** The question has a word about disks, folders, files, sizes or cleaning, or a path. */
  aboutDisk: boolean;
  /** A clear intent word. */
  intent: ScanIntent | null;
  /** `cache` or `build`, by a word. Media is `media`. */
  kind: ScanKind | null;
  /** A media category, when the question asks about media and not about a folder with that name. */
  media: MediaKind | null;
  /** Personal folders named by their English or Polish names (never those read as media). */
  roles: Role[];
  /** A path the user typed (`~/Projekty`, `/var`, the home as `~`), as typed. */
  path: string | null;
  /** A hidden folder named by itself (`.cache`, `.local`). */
  hidden: string | null;
  /** A number of rows asked for, 1 to 10. */
  limit: number | null;
  polish: boolean;
  /** `everything` / `wszystko` with a word of deleting. */
  everything: boolean;
  /** The folded words of the question. */
  tokens: string[];
}

/** Reads what the words of `question` say by themselves. */
export function signals(question: string): Signals {
  const rawWords = question.split(/\s+/u).filter((word) => word !== '');
  // A typed path is a path, not words: `/var/cache` is no word about caches.
  const spoken = rawWords.filter((word) => pathToken(word) === null && hiddenToken(word) === null);
  const tokens = wordsOfQuestion(spoken.join(' '));

  const typedPath = rawWords.map(pathToken).find((token) => token !== null) ?? null;
  const path = typedPath ?? (homeWord(tokens) ? '~' : null);
  const hidden = rawWords.map(hiddenToken).find((token) => token !== null) ?? null;
  const intent = intentOf(tokens);

  let kind: ScanKind | null = null;

  if (
    hasStem(tokens, ['cach']) ||
    hasSequence(tokens, ['pamiec', 'podreczna']) ||
    hasSequence(tokens, ['pamieci', 'podrecznej'])
  ) {
    kind = 'cache';
  } else if (hasStem(tokens, ['build', 'dependenc', 'kompilac', 'artifact'])) {
    kind = 'build';
  }

  const roles: Role[] = [];

  for (const token of tokens) {
    const role = roleOfToken(token);

    if (role !== null && !roles.includes(role)) {
      roles.push(role);
    }
  }

  let media: MediaKind | null = null;

  if (path === null) {
    media = MEDIA_ONLY.find(([word]) => tokens.includes(word))?.[1] ?? null;
  }

  for (const [word, category, role] of MEDIA_OR_FOLDER) {
    const at = tokens.indexOf(word);

    if (at < 0) {
      continue;
    }

    const after = tokens[at + 1];
    const before = tokens[at - 1];
    const namesAFolder =
      (after !== undefined && FOLDER_CUES.includes(after)) ||
      (before !== undefined && ['in', 'w', 'we', 'inside', 'under'].includes(before)) ||
      path !== null ||
      intent !== 'largest';

    if (namesAFolder) {
      if (!roles.includes(role)) {
        roles.push(role);
      }
    } else if (media === null) {
      media = category;
    }
  }

  const aboutDisk =
    path !== null ||
    hidden !== null ||
    media !== null ||
    roles.length > 0 ||
    hasStem(tokens, DISK_STEMS) ||
    hasWord(tokens, DISK_WORDS);

  return {
    aboutDisk,
    intent,
    kind,
    media,
    roles,
    path,
    hidden,
    limit: limitOf(tokens),
    polish: /[ąćęłńóśźżĄĆĘŁŃÓŚŹŻ]/u.test(question) || hasWord(tokens, POLISH_WORDS),
    everything: hasWord(tokens, ['everything', 'wszystko']) && intent === 'cleanable',
    tokens,
  };
}
