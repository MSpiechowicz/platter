import type { Segment } from './mock-answer-segments';
import { OFFTOPIC_TEXT, OUTDATED_NOTE, templateSegments, verdictTotal } from './mock-answer-template';
import type { FactRow, FactSheet, FactTotal } from './mock-answer-template';
import { render } from './mock-answer-segments';
import {
  DEFAULT_LIMIT,
  ground,
  interpret,
  MAX_LIMIT,
  readQuestion,
  type Reading,
  type ScanQuery,
  type Subject,
} from './mock-question-reader';
import { roleOfName, type Role } from './mock-question-signals';
import { nameWords, namedWords, placeHint, rulesForWords, scopeWords } from './mock-scope-words';
import {
  advisorNodes,
  BUILTIN_RULE_NAMES,
  ruleMatches,
  shortenHome,
  suggestionsOf,
  ruleView,
  type MockLabel,
  type RuleMatch,
  type RuleName,
  type UserRuleInput,
} from './mock-suggestions';
import { largestDirs } from './mock-summary';
import { childrenOf, type MockNode, type MockTree } from './mock-tree';
import { ApiRequestError } from './client';
import { formatSize } from '../format';
import type {
  AnswerSegmentDto,
  Category,
  ScanAnswerDto,
  ScanId,
  ScanRowDto,
  SizeUnits,
  SummaryDto,
  Verdict,
} from './types';

/*
 * The mock's stand-in for `ask_scan` (docs/architecture.md, "Questions about a scan"), without a model: the question
 * is read words first (`mock-question-reader.ts`: the backend's `read`; its keyword reader stands in for the model's
 * part), the subject is resolved in the mock tree (the backend's `resolve`: a tool of the rules, a personal folder as
 * the machine has it, a path, a name; a subject that fits two things is asked back), the rows and the facts are
 * collected here as the backend's `collect` and `Lens` do (`apps/desktop/src-tauri/src/commands/advisor/question.rs`)
 * from the mock tree, the mock rules (`mock-suggestions.ts`) and the mock labels, and the answer text is the backend's
 * template (`mock-answer-template.ts`, a port of `answer.rs`) over those facts. Different from the backend:
 * - The model's part of the reading is keywords, not a model (see the reader); the rest is the backend's order of work.
 * - The built-in rules of the mock are Linux's: no rule is left out for another system.
 * - The personal folders are the English names unless the tree says otherwise (`MockTree.userDirs`, the mock's
 *   `user-dirs.dirs`).
 * - The question's two stages are one wait: a model stopped meanwhile fails the question (the backend's stage A),
 *   never gives the template (its stage B). A question the words settle has no wait at all (`mock-advisor.ts`).
 * - Rows show the path as the backend does: the home folder written as `~`. The sheet's 6000 characters in all are
 *   not applied.
 */

export { DEFAULT_LIMIT, ground, interpret, MAX_LIMIT, OFFTOPIC_TEXT, OUTDATED_NOTE };
export type { ScanQuery };

/** What a row says about a folder the guard leaves alone, and what the answer says of a scan inside a kept folder. */
export const GUARD_DETAIL = 'Left alone by the safety guard, not decided by the model.';
export const KEPT_SCAN_NOTE = 'This scan starts inside a folder you keep, so nothing in it is suggested.';
const LABELLED_BY_MODEL = 'Labelled by the local model';
/** What to do about it, when the model has labelled nothing (the answer itself says that nothing is labelled). */
const NO_LABELS_NOTE = 'Use “Label folders” in the Advisor view.';
const ENGLISH_NOTE = 'Answers are in English.';
const NOT_FROM_HERE_NOTE = 'Platter does not delete anything from here. Use Cleanup to choose.';
/** What a `pick` that matches no choice is refused with. */
export const STALE_CHOICE = 'that choice is no longer available, so ask again';
const FAMILY_BUILD_LABEL = 'All build output and dependency folders';

/** The rules each kind stands for (the ids of the mock's rule table, the same ids as the backend's). */
export const CACHE_FAMILY: ReadonlySet<string> = new Set([
  'apt-cache',
  'dnf-cache',
  'npm-cache',
  'bun-cache',
  'pnpm-store',
  'yarn-cache',
  'cargo-registry',
  'gradle-cache',
  'nuget-packages',
  'go-modules',
  'steam-shader-cache',
  'pacman-cache',
  'app-cache',
  'maven-repository',
  'python-cache',
]);

export const BUILD_FAMILY: ReadonlySet<string> = new Set([
  'rust-target',
  'rust-target-recent',
  'node-modules',
  'node-modules-recent',
]);

const MEDIA_CATEGORIES: readonly Category[] = ['video', 'image', 'audio'];

const MAX_NOTES = 6;
const NOTE_CHARS = 240;
const MAX_FOUND = 20;
/** Most choices of one question asked back. */
const MAX_CHOICES = 4;
/** A folder counts as a peer of the largest one of the same name when it is at least a quarter of it. */
const PEER_SHARE = 4;
const SECONDS_PER_DAY = 86_400;

/**
 * Finds folders of the tree: an exact path (`~/...` or `/...`), and a search by name (`find_folders`). The `entry`
 * and `searchEntries` also hold the regular files (`find_entries`).
 */
export interface FolderLookup {
  exact(path: string): MockNode | null;
  /** The folder or the regular file an exact path names. */
  entry(path: string): MockNode | null;
  search(text: string, limit: number): MockNode[];
  /** Folders and files by name, the exact name first, then names that start with the text, then names that hold it. */
  searchEntries(text: string, limit: number): MockNode[];
}

export interface ScanQuestionContext {
  scan: ScanId;
  tree: MockTree;
  user: readonly UserRuleInput[];
  /** The labels of the scan's last labelling run (empty when there is none). */
  labels: readonly MockLabel[];
  /** A cleanup ran on the scan. */
  outdated: boolean;
  units: SizeUnits;
  lookup: FolderLookup;
  /** Why the guard leaves a folder alone (the protected places, the scanned folder...), or `null` when it does not. */
  guard(node: MockNode): string | null;
  /** The summary of a folder: its shares by category and by age. */
  summary(node: MockNode): SummaryDto;
}

/** A folder of the answer before it is shown: the row, and what only the template needs of it. */
interface Row extends ScanRowDto {
  /** The words of whoever decided the folder: the rule's reason or the guard's. */
  why?: string;
  /** Platter could not read the folder, so its size is unknown. */
  unreadable: boolean;
}

/** What one intent found, before it is turned into rows and a sheet. */
interface Found {
  rows: Row[];
  /** Matches that are not among the rows. */
  more: number;
  scopeSize?: number;
  files?: number;
  totals: FactTotal[];
  notes: string[];
  contextPaths: string[];
  /** The rows are the thing the question named, not what lies inside it. */
  subject: boolean;
}

const nothingFound = (): Found => ({
  rows: [],
  more: 0,
  totals: [],
  notes: [],
  contextPaths: [],
  subject: false,
});

/** Where the question's scope points in the finished tree. */
type Scope =
  /** No scope: the whole scan. */
  | { at: 'whole' }
  | { at: 'folder'; node: MockNode }
  /** A regular file: one row, whatever the intent. */
  | { at: 'file'; node: MockNode }
  /** The folders that the rules named by the scope text matched in this scan (more than one), largest first. */
  | { at: 'rules'; title: string; nodes: MockNode[] }
  /** A scope text that is no folder of this scan. `absent` says what the rules call it and where they look for it, when a rule knows it. */
  | { at: 'missing'; text: string; absent?: Absent };

/** What a rule with a known place is called by the question, and where that place is. */
interface Absent {
  /** The words of the question that named the rule (`docker` of `docker images`). */
  named: string;
  hint: string;
}

/** The folders of one tool of the rules in this scan. */
interface Tool {
  title: string;
  nodes: MockNode[];
}

/** What the rules know of a scope text. */
type ByRule =
  /** The folders the rules matched in this scan, and what to call them; `located`: the name means a known place. */
  | { found: true; title: string; nodes: MockNode[]; located: boolean; notes: string[]; tools: Tool[] }
  /** A rule with a known place names it, and that place is not in this scan. */
  | { found: false; absent: Absent };

/** One way to read an unclear subject. */
interface Choice {
  label: string;
  path?: string;
  node?: number;
  target: Scope;
}

/** A question asked back: what the words could mean. */
interface Clarify {
  prompt: string;
  choices: Choice[];
}

/** Where a question's subject is. */
interface Resolved {
  scope: Scope;
  notes: string[];
  clarify?: Clarify;
}

type Outcome = { scope: Scope; notes: string[] } | { ask: Clarify };

const CATEGORY_NAME: Record<Category, string> = {
  video: 'Video',
  image: 'Images',
  audio: 'Audio',
  document: 'Documents',
  archive: 'Archives',
  code: 'Code',
  build: 'Build output',
  package: 'Packages',
  app: 'Applications',
  diskImage: 'Disk images',
  data: 'Data',
  other: 'Other',
};

const total = (label: string, bytes: number, count: number): FactTotal => ({
  label,
  bytes,
  count,
  kind: 'plain',
});

/** A text of the user's disk or of the user's words, set apart in “ ” quotes. */
const quoted = (text: string): string => `“${text}”`;

/** `No file or folder called “x” is in this scan`, and where the rules look for what `x` names, when they know. */
const notInScan = (text: string, absent?: Absent): string => {
  const called = absent?.named ?? text;

  return absent === undefined
    ? `No file or folder called ${quoted(called)} is in this scan`
    : `No file or folder called ${quoted(called)} is in this scan (${absent.hint})`;
};

const clip = (note: string): string => Array.from(note).slice(0, NOTE_CHARS).join('');

/** A share in whole per cent, rounded. */
export const percent = (part: number, whole: number): number =>
  whole === 0 ? 0 : Math.floor((part * 100 + Math.floor(whole / 2)) / whole);

/** The last part of a path. */
const baseName = (path: string): string => path.replace(/\/+$/, '').split('/').pop() ?? path;

const capitalised = (text: string): string => `${text.charAt(0).toUpperCase()}${text.slice(1)}`;

/** `downloads`, `documents`: the role as it is said in a sentence. */
const roleWord = (role: Role): string => role.toLowerCase();

/**
 * The lower-case names of the folders of the tree, and of the files whose name holds a dot that does not start it
 * (`old.iso`): what the keyword reader (the model's stand-in) can take for a name.
 */
export function folderNamesOf(tree: MockTree): Set<string> {
  const names = new Set<string>();

  for (const node of tree.nodes) {
    if (node.parent < 0) {
      continue;
    }

    if (node.kind === 'dir' || (node.kind === 'file' && node.name.indexOf('.') > 0)) {
      names.add(node.name.toLowerCase());
    }
  }

  return names;
}

/** Whether the words of a question settle what it asks, so that no model is needed to read it (and no wait). */
export const readsWithoutModel = (question: string): boolean => readQuestion(question, null).complete;

/**
 * Answers a question about a finished scan: what the backend's `ask_scan` returns, with the template as the text. The
 * words are read first; the model's stand-in (the keyword reader) fills what they leave open. `pick` is the id of a
 * choice of the same question asked back before; one that fits no choice is `invalid`.
 */
export function answerQuestion(context: ScanQuestionContext, question: string, pick?: number): ScanAnswerDto {
  let reading = readQuestion(question, null);

  if (!reading.complete) {
    reading = readQuestion(question, interpret(question, folderNamesOf(context.tree)));
  }

  return answerReading(context, reading, pick);
}

/** What the question comes to, collected into rows and facts and written as the answer. */
function answerReading(
  context: ScanQuestionContext,
  reading: Reading,
  pick: number | undefined,
): ScanAnswerDto {
  const { tree, lookup, units } = context;
  const view = ruleView(tree, context.user);
  const suggestions = suggestionsOf(tree, context.labels, context.user);
  const matches = ruleMatches(tree, context.user);
  const labelOf = new Map(context.labels.map((label) => [label.node, label]));
  const root = tree.nodes[0] as MockNode;
  const query = reading.query;
  // A find of a family of folders is a question about the biggest of them.
  const intent = query.intent === 'find' && query.kind !== 'any' ? 'largest' : query.intent;

  const display = (node: MockNode): string => shortenHome(view.path(node), view.home);
  const isDir = (node: MockNode | undefined): node is MockNode => node?.kind === 'dir';

  /** A folder as a row with its standing, in the backend's order: the guard, a rule that covers it, an earlier label. */
  const rowOf = (node: MockNode): Row => {
    const base = {
      node: node.id,
      path: display(node),
      allocated: node.allocated,
      unreadable: node.unreadable === true,
      ...(node.kind === 'file' ? { file: true as const } : {}),
    };
    const detail = context.guard(node);

    if (detail !== null) {
      return { ...base, verdict: 'keep', source: 'guard', note: GUARD_DETAIL, why: detail };
    }

    const cover = view.covering(node);

    if (cover) {
      return { ...base, verdict: cover.verdict, source: cover.source, note: cover.title, why: cover.reason };
    }

    const label = labelOf.get(node.id);

    return label ? { ...base, verdict: label.verdict, source: 'model', note: LABELLED_BY_MODEL } : base;
  };

  const inside = (node: MockNode, scope: MockNode): boolean => {
    for (let at: MockNode | undefined = node; at; at = tree.nodes[at.parent]) {
      if (at.id === scope.id) {
        return true;
      }
    }

    return false;
  };

  /** `nodes`, largest first, equal sizes by id. */
  const bySize = (a: MockNode, b: MockNode): number => b.allocated - a.allocated || a.id - b.id;

  const sum = (nodes: readonly MockNode[]): number =>
    nodes.reduce((bytes, node) => bytes + node.allocated, 0);

  /** The folders in order of size, without one twice. */
  const largestFirst = (nodes: readonly MockNode[]): MockNode[] => {
    const sorted = [...nodes].sort(bySize);

    return sorted.filter((node, index) => node.id !== sorted[index - 1]?.id);
  };

  /** The rows of `nodes`, in order, at most the question's limit; the rest is `more`. */
  const rowsOf = (nodes: readonly MockNode[], found: Found): void => {
    const shown = Math.min(query.limit, MAX_LIMIT);

    found.rows = nodes.slice(0, shown).map(rowOf);
    found.more = Math.max(0, nodes.length - shown);
  };

  /**
   * The summary's largest folders, each lifted to the folder a rule matched above it, when that lies inside the scope:
   * the folder the rules know (`/var/lib/docker`, `~/Projekty/app/target`), not a part of it. Largest first.
   */
  const lifted = (nodes: readonly MockNode[], scope: MockNode): MockNode[] =>
    largestFirst(
      nodes.map((node) => {
        const at = tree.nodes[view.covering(node)?.at ?? -1];

        return at && at.id !== node.id && at.id !== scope.id && inside(at, scope) && isDir(at) ? at : node;
      }),
    );

  /** The categories a `media` question is about: the one it names (`videos`), else all three. */
  const mediaCategories = (): readonly Category[] => (reading.media ? [reading.media] : MEDIA_CATEGORIES);

  /** Whether a match belongs to the question's kind; `rule` is the rule that matched, when one did. */
  const inKind = (node: MockNode, rule: string | null): boolean => {
    switch (query.kind) {
      case 'any':
        return true;
      case 'cache':
        return rule !== null && CACHE_FAMILY.has(rule);
      case 'build':
        return rule !== null && BUILD_FAMILY.has(rule);
      case 'media':
        return mediaCategories().includes(node.category);
    }
  };

  /** The matches of the rules `accept` takes, under the scope: every match, not only the ones a suggestion lists. */
  const matched = (
    accept: (match: RuleMatch) => boolean,
    scope: MockNode,
  ): { node: MockNode; rule: string; verdict: Verdict }[] =>
    matches
      .filter(accept)
      .flatMap((match) =>
        match.nodes
          .filter((node) => inside(node, scope))
          .map((node) => ({ node, rule: match.id, verdict: match.verdict })),
      );

  /** What the rules matched inside a folder (not the folder itself), by rule: the bytes of its matches, largest first. */
  const insideRules = (node: MockNode): { id: string; title: string; bytes: number; verdict: Verdict }[] => {
    const byRule = new Map<string, { id: string; title: string; bytes: number; verdict: Verdict }>();

    for (const match of matched(() => true, node)) {
      if (match.node.id === node.id) {
        continue;
      }

      const entry = byRule.get(match.rule) ?? {
        id: match.rule,
        title: matches.find((entry) => entry.id === match.rule)?.title ?? match.rule,
        bytes: 0,
        verdict: match.verdict,
      };

      byRule.set(match.rule, { ...entry, bytes: entry.bytes + match.node.allocated });
    }

    return [...byRule.values()].sort((a, b) => b.bytes - a.bytes || (a.id < b.id ? -1 : 1));
  };

  // ---- Scope ----

  /** One folder is that folder; more are the folders of a rule. */
  const scopeOf = (title: string, nodes: MockNode[]): Scope => {
    const [only] = nodes;

    return nodes.length === 1 && only ? { at: 'folder', node: only } : { at: 'rules', title, nodes };
  };

  /** The folders a built-in rule matched in this scan. */
  const matchesOf = (id: string): MockNode[] =>
    (matches.find((match) => match.id === id)?.nodes ?? []).filter(isDir);

  /** The folder at the one place a rule looks in, when the scan holds it and could not read it. */
  const unreadablePlace = (place: string | null): MockNode | null => {
    const found = place === null ? null : lookup.exact(place);

    return found?.unreadable ? found : null;
  };

  /**
   * The folders of the rules `named` matched in this scan, largest first, with what to call them; where they look, when
   * they have one place and nothing was matched. `text` is what the user said. `null` when no rule is named, or one is
   * and has neither a match nor a place to name.
   */
  const byNamedRules = (named: readonly RuleName[], text: string): ByRule | null => {
    if (named.length === 0) {
      return null;
    }

    const matching: RuleName[] = [];
    // The folders each rule matched, to tell the tools it names apart.
    const perRule: { rule: RuleName; nodes: MockNode[] }[] = [];

    for (const rule of named) {
      const folders = matchesOf(rule.id);

      if (folders.length > 0) {
        matching.push(rule);
        perRule.push({ rule, nodes: folders });
      }
    }

    // A rule's place that Platter could not read is not matched (nothing is known of it), but it is there: it is the
    // folder asked about, and the answer says it could not be read.
    for (const rule of named) {
      const node = matching.some((one) => one.id === rule.id) ? null : unreadablePlace(rule.place);

      if (node) {
        matching.push(rule);
        perRule.push({ rule, nodes: [node] });
      }
    }

    const folders = largestFirst(perRule.flatMap((entry) => entry.nodes));
    const places = named.flatMap((rule) => placeHint(rule) ?? []);

    if (folders.length === 0) {
      return places.length > 0
        ? { found: false, absent: { named: namedWords(text, named), hint: places.slice(0, 2).join('. ') } }
        : null;
    }

    // One rule's folders are called by its title; the folders of several by what the user said.
    const only = matching.length === 1 ? matching[0] : undefined;
    const title = only ? only.title : /^\p{L}+$/u.test(text) ? capitalised(text) : text;

    return {
      found: true,
      title,
      nodes: folders,
      located: places.length > 0,
      notes: outsideNotes(named),
      tools: toolsOf(perRule),
    };
  };

  /**
   * For each of the rules `named` whose one fixed place lies outside the scanned folder: where it is and how to
   * include it. Asked only when folders of the tool were found, so the answer does not look complete without it.
   */
  const outsideNotes = (named: readonly RuleName[]): string[] => {
    const scanned = view.path(root);

    return named.flatMap((rule) =>
      rule.place?.startsWith('/') &&
      !(scanned === '/' || rule.place === scanned || rule.place.startsWith(`${scanned}/`))
        ? [`${quoted(rule.place)} (${rule.title}) is outside this scan. Scan / to include it.`]
        : [],
    );
  };

  /**
   * The rules that matched, grouped by tool: the first word of a rule's id (`docker-data` and `docker-desktop-vm` are
   * Docker), each with its folders, largest first.
   */
  const toolsOf = (perRule: readonly { rule: RuleName; nodes: MockNode[] }[]): Tool[] => {
    const tools = new Map<string, Tool>();

    for (const { rule, nodes } of perRule) {
      const family = rule.id.split('-')[0] as string;
      const known = tools.get(family);

      if (known) {
        known.nodes.push(...nodes);
      } else {
        tools.set(family, { title: rule.title, nodes: [...nodes] });
      }
    }

    return [...tools.values()].map((tool) => ({ ...tool, nodes: largestFirst(tool.nodes) }));
  };

  /** What the rules know by the words of a scope text: the rules with the most of them, and the folders they matched. */
  const byRule = (text: string): ByRule | null => {
    const words = scopeWords(text);

    return words.length === 0 ? null : byNamedRules(rulesForWords(words), text);
  };

  /** What the rules call `text` and where they look for it, when this scan does not hold it. */
  const absentFor = (text: string): Absent | undefined => {
    const rules = byRule(text);

    return rules && !rules.found ? rules.absent : undefined;
  };

  const missing = (text: string, absent?: Absent): Scope => ({
    at: 'missing',
    text,
    ...(absent ? { absent } : {}),
  });

  /** A path as the tree displays it: the home written as `~`. */
  const queryOf = (path: string): string => shortenHome(path, view.home);

  /** Where the user's folder of this role is: the machine's own setting, else its usual name in the home. */
  const rolePath = (role: Role): string | null =>
    view.home === null ? null : `${view.home}/${tree.userDirs?.[role] ?? role}`;

  /** For `~/Documents`: the role of that name and where the user's own folder of it is. */
  const roleOfPath = (text: string): { role: Role; real: string } | null => {
    const name = text.startsWith('~/') ? text.slice(2).replace(/\/+$/, '') : '';
    const role = name === '' || name.includes('/') ? null : roleOfName(name);
    const path = role === null ? null : rolePath(role);

    return role !== null && path !== null ? { role, real: queryOf(path) } : null;
  };

  /** A path the user typed: that folder exactly. */
  const resolvePath = (text: string): Outcome => {
    const hit = lookup.exact(text);
    const role = roleOfPath(text);

    if (!hit) {
      const file = lookup.entry(text);

      if (file?.kind === 'file') {
        return { scope: { at: 'file', node: file }, notes: [] };
      }

      return {
        scope: missing(
          text,
          role
            ? { named: text, hint: `your ${roleWord(role.role)} folder is ${quoted(role.real)}` }
            : undefined,
        ),
        notes: [],
      };
    }

    const notes =
      role && role.real !== display(hit)
        ? [`Your ${roleWord(role.role)} folder is ${quoted(role.real)}.`]
        : [];

    return { scope: { at: 'folder', node: hit }, notes };
  };

  /**
   * The user's folder of a role. `typed` is the name the user used: an English name for a folder that has another name
   * here is said (`There is no “~/Downloads”. Your downloads folder is “~/Pobrane”.`).
   */
  const resolveRole = (role: Role, typed: string): Outcome => {
    const absent = (hint?: string): Outcome => ({
      scope: missing(role, hint === undefined ? undefined : { named: role, hint }),
      notes: [],
    });
    const path = rolePath(role);

    if (path === null) {
      return absent();
    }

    const query = queryOf(path);
    const found = lookup.exact(query);

    if (!found || display(found) !== query) {
      return absent(`your ${roleWord(role)} folder is ${quoted(query)}`);
    }

    const sameName = baseName(display(found)).toLowerCase() === role.toLowerCase();
    const notes =
      typed.toLowerCase() === role.toLowerCase() && !sameName
        ? [`There is no ${quoted(`~/${role}`)}. Your ${roleWord(role)} folder is ${quoted(display(found))}.`]
        : [];

    return { scope: { at: 'folder', node: found }, notes };
  };

  /** The folders of the tools that matched: one tool is the scope, two or more are asked back. */
  const ofTools = (
    said: string,
    title: string,
    nodes: MockNode[],
    tools: Tool[],
    notes: string[],
  ): Outcome => {
    if (tools.length < 2) {
      return { scope: scopeOf(title, nodes), notes };
    }

    const choices = tools.slice(0, MAX_CHOICES).map((tool): Choice => {
      const target = scopeOf(tool.title, tool.nodes);

      return target.at === 'folder'
        ? { label: tool.title, path: display(target.node), node: target.node.id, target }
        : { label: tool.title, target };
    });

    return {
      ask: { prompt: `${quoted(said)} could mean more than one thing. Which one do you mean?`, choices },
    };
  };

  /** A tool of the rules the question named, by the ids of its rules. */
  const resolveTools = (said: string, ids: readonly string[]): Outcome => {
    const rules = byNamedRules(
      BUILTIN_RULE_NAMES.filter((rule) => ids.includes(rule.id)),
      said,
    );

    if (rules?.found) {
      return ofTools(said, rules.title, rules.nodes, rules.tools, rules.notes);
    }

    return { scope: missing(said, rules?.absent), notes: [] };
  };

  /**
   * The folders with a name, for a subject that is a bare name: `~/<name>`, else the folders called that (or, failing
   * that, starting with it or holding it), largest first. Folders inside another of them are not separate; of the rest,
   * the largest alone when the others are much smaller (a quarter or less of it), else every one of similar standing.
   */
  const foldersNamed = (text: string): MockNode[] => {
    const exact = lookup.exact(`~/${text}`);

    if (exact && display(exact) === `~/${text}`) {
      return [exact];
    }

    const found = lookup.search(text, MAX_FOUND);
    const lowered = text.toLowerCase();
    const tier = (node: MockNode): number => {
      const name = baseName(display(node)).toLowerCase();

      return name === lowered ? 0 : name.startsWith(lowered) ? 1 : 2;
    };
    const best = Math.min(...found.map(tier));
    const nodes = found.filter((node) => tier(node) === best).sort(bySize);
    const outer = nodes.filter((node) => !nodes.some((other) => other.id !== node.id && inside(node, other)));
    const [largest] = outer;

    if (!largest) {
      return [];
    }

    const peers = outer.filter((node) => node.allocated * PEER_SHARE >= largest.allocated);

    return peers.length < 2 ? [largest] : peers;
  };

  /**
   * The files with exactly this name (case ignored): the file at `~/<name>` when there is one, else the files called
   * that anywhere, largest first. The largest alone when the others are a quarter of it or less, else every one of
   * similar standing.
   */
  const filesNamed = (text: string): MockNode[] => {
    const home = lookup.entry(`~/${text}`);

    if (home && home.kind === 'file' && display(home) === `~/${text}`) {
      return [home];
    }

    const lowered = text.toLowerCase();
    const files = tree.nodes
      .filter((node) => node.parent >= 0 && node.kind === 'file' && node.name.toLowerCase() === lowered)
      .sort(bySize);
    const [largest] = files;

    if (!largest) {
      return [];
    }

    const peers = files.filter((node) => node.allocated * PEER_SHARE >= largest.allocated);

    return peers.length < 2 ? [largest] : peers;
  };

  /** The question asked back for folders that share a name. */
  const whichFolder = (text: string, nodes: readonly MockNode[]): Clarify => ({
    prompt: `I found more than one folder called ${quoted(text)}. Which one do you mean?`,
    choices: nodes.slice(0, MAX_CHOICES).map((node) => ({
      label: display(node),
      path: display(node),
      node: node.id,
      target: { at: 'folder', node } as const,
    })),
  });

  /** The question asked back for files that share a name, or for a file and a folder that do. */
  const whichEntry = (text: string, folders: readonly MockNode[], files: readonly MockNode[]): Clarify => {
    const mixed = folders.length > 0 && files.length > 0;
    // Folders first, and room left for at least one file when there is one.
    const shownFolders = folders.slice(0, mixed ? MAX_CHOICES - 1 : MAX_CHOICES);
    const shownFiles = files.slice(0, MAX_CHOICES - shownFolders.length);
    const choice = (node: MockNode, kind: 'file' | 'folder'): Choice => ({
      label: mixed ? `${display(node)} (${kind})` : display(node),
      path: display(node),
      node: node.id,
      target: kind === 'file' ? { at: 'file', node } : { at: 'folder', node },
    });
    const what = mixed ? 'file or folder' : 'file';

    return {
      prompt: `I found more than one ${what} called ${quoted(text)}. Which one do you mean?`,
      choices: [
        ...shownFolders.map((node) => choice(node, 'folder')),
        ...shownFiles.map((node) => choice(node, 'file')),
      ],
    };
  };

  /** The question asked back for several folders, and the folder itself for one. */
  const ofFolders = (text: string, folders: readonly MockNode[]): Outcome => {
    const [folder] = folders;

    return folders.length === 1 && folder
      ? { scope: { at: 'folder', node: folder }, notes: [] }
      : { ask: whichFolder(text, folders) };
  };

  /**
   * What a bare name stands for: the folders with that name and the files with exactly that name. An exact file beats
   * folders that only start with the name or hold it, a file and a folder both exact are asked back, and several files
   * follow the rule of the folders. `null` when nothing has the name. The words of a text that is no name are looked
   * for as folders only, by `foldersOfWords`.
   */
  const entriesNamed = (text: string): Outcome | null => {
    const folders = foldersNamed(text);
    const files = filesNamed(text);
    const [folder] = folders;
    const [file] = files;
    const folderExact =
      folder !== undefined && baseName(display(folder)).toLowerCase() === text.toLowerCase();

    if (file && (folderExact || files.length > 1)) {
      return { ask: whichEntry(text, folderExact ? folders : [], files) };
    }

    if (file) {
      return { scope: { at: 'file', node: file }, notes: [] };
    }

    return folder ? ofFolders(text, folders) : null;
  };

  /** The first word of a text that is no name that has folders, and those folders. */
  const foldersOfWords = (text: string): Outcome | null => {
    for (const word of nameWords(text)) {
      const folders = foldersNamed(word);

      if (folders.length > 0) {
        return ofFolders(text, folders);
      }
    }

    return null;
  };

  /** A path or a name the model read, as the tree knows it. */
  const resolveText = (given: string): Outcome => {
    const text = given.trim();

    if (text === '') {
      return { scope: { at: 'whole' }, notes: [] };
    }

    if (text === '~' || text.startsWith('~/') || text.startsWith('/')) {
      return resolvePath(text);
    }

    const role = roleOfName(text);

    if (role !== null) {
      return resolveRole(role, text);
    }

    // A name with a slash in it is a path of folders, never what a rule is called.
    const rules = text.includes('/') ? null : byRule(text);

    if (rules?.found) {
      return ofTools(text, rules.title, rules.nodes, rules.tools, rules.notes);
    }

    if (rules) {
      return { scope: missing(text, rules.absent), notes: [] };
    }

    return entriesNamed(text) ?? foldersOfWords(text) ?? { scope: missing(text), notes: [] };
  };

  /**
   * Resolves what the question names, in the finished tree and never on disk, in this order: a typed path is that exact
   * folder; a tool the rules know is the folders its rules matched, all of them (where the rules have one place that the
   * scan does not hold, the scope is missing, and says where); a personal folder is the user's own; a bare name is
   * `~/<name>`, else the folders with that name. A subject that fits two things is asked back; `pick` takes one of them.
   */
  const resolve = (subject: Subject): Resolved => {
    let outcome: Outcome;

    switch (subject.kind) {
      case 'none':
        outcome = { scope: { at: 'whole' }, notes: [] };
        break;
      case 'path':
        outcome = resolvePath(subject.text);
        break;
      case 'role':
        outcome = resolveRole(subject.role, subject.typed);
        break;
      case 'rules':
        outcome = resolveTools(subject.said, subject.ids);
        break;
      case 'name':
        outcome = resolveText(subject.text);
        break;
    }

    if ('ask' in outcome) {
      if (pick === undefined) {
        return { scope: { at: 'whole' }, notes: [], clarify: outcome.ask };
      }

      const choice = Number.isInteger(pick) ? outcome.ask.choices[pick] : undefined;

      if (!choice) {
        throw new ApiRequestError('invalid', STALE_CHOICE);
      }

      return { scope: choice.target, notes: [] };
    }

    if (pick !== undefined) {
      throw new ApiRequestError('invalid', STALE_CHOICE);
    }

    return outcome;
  };

  // `find` looks for a name, so a name is not a folder to resolve; an overview is of the whole scan.
  const resolved: Resolved =
    intent === 'general' ||
    intent === 'offtopic' ||
    (intent === 'find' && (reading.subject.kind === 'name' || reading.subject.kind === 'none'))
      ? { scope: { at: 'whole' }, notes: [] }
      : resolve(reading.subject);

  if (resolved.clarify) {
    return clarified(context, intent, query, resolved.clarify);
  }

  let scope = resolved.scope;

  /** The scope as a node: a resolved folder, or the root for the whole scan (a scope of several folders: the largest). */
  const nodeOf = (given: Scope): MockNode =>
    given.at === 'folder' || given.at === 'file'
      ? given.node
      : given.at === 'rules'
        ? (given.nodes[0] as MockNode)
        : root;

  /** The folders the scope covers: the root for the whole scan, none for a scope that is not there. */
  const rootsOf = (given: Scope): MockNode[] => {
    switch (given.at) {
      case 'whole':
        return [root];
      case 'folder':
      case 'file':
        return [given.node];
      case 'rules':
        return given.nodes;
      case 'missing':
        return [];
    }
  };

  // ---- The intents ----

  function largest(given: Scope): Found {
    // The folders of a rule are the answer themselves, largest first, with what they take together.
    if (given.at === 'rules') {
      const found = nothingFound();

      found.totals.push(total(given.title, sum(given.nodes), given.nodes.length));
      rowsOf(given.nodes, found);

      return found;
    }

    switch (query.kind) {
      case 'any':
        return largestAny(given);
      case 'cache':
        return largestFamily(given, CACHE_FAMILY, 'All caches');
      case 'build':
        return largestFamily(given, BUILD_FAMILY, FAMILY_BUILD_LABEL);
      case 'media':
        return largestMedia(given);
    }
  }

  /** The scope's largest folders, from the summary, lifted to what a rule matched. */
  function largestAny(given: Scope): Found {
    const node = nodeOf(given);
    const found: Found = { ...nothingFound(), scopeSize: node.allocated, files: node.files };

    rowsOf(lifted(largestDirs(tree, node), node), found);

    return found;
  }

  /** The folders of one rule family under the scope, largest first, with the family's total. */
  function largestFamily(given: Scope, family: ReadonlySet<string>, label: string): Found {
    const node = nodeOf(given);
    const found = nothingFound();
    const cover = view.covering(node);

    // The scope itself lies in the family: it is the one row.
    if (cover && family.has(cover.id)) {
      found.rows.push(rowOf(node));
      found.totals.push(total(label, node.allocated, 1));

      return found;
    }

    const nodes = largestFirst(
      matched((match) => family.has(match.id), node)
        .map((match) => match.node)
        .filter(isDir),
    );

    if (nodes.length > 0) {
      found.totals.push(total(label, sum(nodes), nodes.length));
    } else if (view.insideKept) {
      found.notes.push(KEPT_SCAN_NOTE);
    }

    rowsOf(nodes, found);

    return found;
  }

  /** The first folder of each branch below the scope whose category is video, image or audio (not entered). */
  function largestMedia(given: Scope): Found {
    const wanted = mediaCategories();
    const hits: MockNode[] = [];
    const stack = childrenOf(tree, nodeOf(given)).filter(isDir).reverse();

    for (let node = stack.pop(); node; node = stack.pop()) {
      if (!wanted.includes(node.category)) {
        stack.push(...childrenOf(tree, node).filter(isDir).reverse());
      } else if (node.allocated > 0) {
        hits.push(node);
      }
    }

    hits.sort(bySize);

    const found = nothingFound();

    for (const category of wanted) {
      const of = hits.filter((hit) => hit.category === category);

      if (of.length > 0) {
        found.totals.push(total(CATEGORY_NAME[category], sum(of), of.length));
      }
    }

    rowsOf(hits, found);

    for (const row of found.rows) {
      row.note = CATEGORY_NAME[(tree.nodes[row.node] as MockNode).category];
    }

    return found;
  }

  /** Adds a verdict's bytes and count to its total. */
  const addVerdictTotal = (found: Found, verdict: Verdict, bytes: number, count: number): void => {
    const known = found.totals.find((entry) => entry.kind === 'verdict' && entry.verdict === verdict);

    if (known) {
      known.bytes = (known.bytes ?? 0) + bytes;
      known.count = (known.count ?? 0) + count;
    } else {
      found.totals.push(verdictTotal(verdict, bytes, count));
    }
  };

  /** The name most of `nodes` have (the first of the largest wins a tie), when more than one has it. */
  const commonName = (nodes: readonly MockNode[]): string | undefined => {
    const counts = new Map<string, number>();

    for (const node of nodes) {
      counts.set(node.name, (counts.get(node.name) ?? 0) + 1);
    }

    let best: [string, number] | undefined;

    for (const entry of counts) {
      if (!best || entry[1] > best[1]) {
        best = entry;
      }
    }

    return best && best[1] > 1 ? best[0] : undefined;
  };

  /** What a folder that nothing in it can be cleaned is: its size, and who decides about it. */
  function standingOf(node: MockNode, found: Found): void {
    found.scopeSize = node.allocated;
    found.files = node.files;

    if (rowOf(node).source === 'model') {
      found.notes.push('The local model labelled it for review. Deleting it is your call.');

      return;
    }

    // What its rules keep inside it.
    const kept = insideRules(node)
      .filter((entry) => entry.verdict === 'keep')
      .slice(0, 2);

    if (kept.length === 0) {
      found.notes.push('Platter has no rule for it. Deleting it is your call.');

      return;
    }

    const named = kept.map((entry) => `${quoted(entry.title)} (${formatSize(entry.bytes, units)})`);

    found.notes.push(
      `Platter keeps part of it, such as ${named.join(' and ')}. Deleting the rest is your call.`,
    );
  }

  /**
   * What can be cleaned: the matches that are not `keep` and the labels the suggestions list, `safe` first, then
   * `review`, largest first. A match counts, as a row and in the totals, with the verdict `rowOf` gives it. What the
   * guard leaves alone is neither a row nor in a total, unless it is the very thing the question named.
   */
  function cleanable(given: Scope): Found {
    const node = nodeOf(given);
    const found = nothingFound();

    if (view.insideKept) {
      found.notes.push(KEPT_SCAN_NOTE);

      return found;
    }

    // The folders of a rule are what the question names: each is a row with the verdict it has, kept and left alone by
    // the guard too, so the answer can say yes or no about every one.
    if (given.at === 'rules') {
      found.subject = true;
      rowsOf(given.nodes, found);

      return found;
    }

    // A scope (or a whole scan) that a rule covers is that rule's: the one row, and for a scope the answer says yes or
    // no about it, a kept folder and one the guard leaves alone too. The whole scan that is itself covered is only
    // said to be kept or left alone.
    const cover = view.covering(node);

    if (cover) {
      const row = rowOf(node);
      const named = given.at === 'folder';
      const detail = context.guard(node);

      found.subject = named;

      if (cover.verdict === 'keep') {
        if (named) {
          found.rows.push(row);
        } else {
          found.notes.push(`${display(node)} is kept by ${cover.title}.`);
        }
      } else if (detail !== null) {
        // A folder that could not be read says so in its row (and in the answer).
        if (!(named && row.unreadable)) {
          found.notes.push(detail);
        }

        if (named) {
          found.rows.push(row);
        }
      } else if (inKind(node, cover.id)) {
        if (row.verdict === 'safe' || row.verdict === 'review') {
          addVerdictTotal(found, row.verdict, node.allocated, 1);
        }

        found.rows.push(row);
      }

      return found;
    }

    // A scope that Platter could not read is not covered by any rule, but the answer is about it.
    if (given.at === 'folder' && node.unreadable) {
      found.subject = true;
      found.rows.push(rowOf(node));

      return found;
    }

    const candidates: { node: MockNode; verdict: 'safe' | 'review' }[] = [];
    const offer = (offered: MockNode): void => {
      const row = rowOf(offered);

      if (row.source !== 'guard' && (row.verdict === 'safe' || row.verdict === 'review')) {
        candidates.push({ node: offered, verdict: row.verdict });
      }
    };

    for (const match of matched((entry) => entry.verdict !== 'keep', node)) {
      if (inKind(match.node, match.rule)) {
        offer(match.node);
      }
    }

    // The model's labels are no rule: they only belong to the kinds that name no rule.
    if (query.kind === 'any' || query.kind === 'media') {
      for (const labelled of advisorNodes(tree, context.labels, context.user)) {
        if (inside(labelled, node) && inKind(labelled, null)) {
          offer(labelled);
        }
      }
    }

    // A match found twice counts once. Only folders are rows; matched files (old downloads, disk images) count in the
    // totals, and are counted once.
    const seen = new Set<number>();
    const ordered = candidates
      .sort((a, b) => (a.verdict === b.verdict ? 0 : a.verdict === 'safe' ? -1 : 1) || bySize(a.node, b.node))
      .filter((candidate) => !seen.has(candidate.node.id) && seen.add(candidate.node.id));
    const skipped = ordered.filter((candidate) => !isDir(candidate.node)).length;
    const byVerdict = (['safe', 'review'] as const).map((verdict) => {
      const all = ordered
        .filter((candidate) => candidate.verdict === verdict)
        .map((candidate) => candidate.node);

      if (all.length > 0) {
        addVerdictTotal(found, verdict, sum(all), all.length);
      }

      return { verdict, folders: all.filter(isDir) };
    });

    // The rows are the largest folders, the limit shared between what is safe and what needs a look, so the answer names
    // both (the biggest to review are often the ones that matter most).
    const limit = Math.min(query.limit, MAX_LIMIT);
    const safe = byVerdict[0]?.folders ?? [];
    const review = byVerdict[1]?.folders ?? [];
    let takeSafe: number;
    let takeReview: number;

    if (safe.length === 0 || review.length === 0) {
      takeSafe = Math.min(limit, safe.length);
      takeReview = Math.min(limit, review.length);
    } else {
      const reviewShare = Math.min(review.length, Math.max(Math.floor(limit / 2), 1));

      takeSafe = Math.min(safe.length, limit - Math.min(reviewShare, Math.max(limit - 1, 0)));
      takeReview = Math.min(review.length, limit - takeSafe);
    }

    const listed = [...safe.slice(0, takeSafe), ...review.slice(0, takeReview)];

    found.rows = listed.map(rowOf);
    found.more = ordered.length - listed.length;

    // What is not listed is told by a name that stands for it (`__pycache__`).
    for (const { verdict, folders } of byVerdict) {
      const example = commonName(folders.filter((folder) => !listed.includes(folder)));
      const entry = found.totals.find((one) => one.kind === 'verdict' && one.verdict === verdict);

      if (entry && example !== undefined) {
        entry.example = example;
      }
    }

    if (skipped > 0) {
      found.notes.push(
        `${skipped} matching files are counted in the totals but not listed. Only folders are listed.`,
      );
    }

    // Nothing to clean in the folder: where Platter stands on it.
    if (found.rows.length === 0 && found.totals.length === 0 && given.at === 'folder') {
      standingOf(node, found);
    }

    return found;
  }

  /** The labels the model made earlier, under the scope: its verdict only; what it said about a folder is never shown. */
  function labelled(given: Scope): Found {
    const found = nothingFound();
    const roots = rootsOf(given);
    const here = context.labels
      .flatMap((label) => {
        const node = tree.nodes[label.node];

        return isDir(node) && roots.some((at) => inside(node, at)) ? [{ label, node }] : [];
      })
      .sort((a, b) => bySize(a.node, b.node))
      .filter((entry, index, all) => all.findIndex((other) => other.node.id === entry.node.id) === index);

    // The answer says that nothing is labelled; the note says what to do about it.
    if (context.labels.length === 0) {
      found.notes.push(NO_LABELS_NOTE);
    }

    for (const verdict of ['review', 'keep'] as const) {
      const of = here.filter((entry) => entry.label.verdict === verdict);

      if (of.length > 0) {
        found.totals.push(verdictTotal(verdict, sum(of.map((entry) => entry.node)), of.length));
      }
    }

    const shown = Math.min(query.limit, MAX_LIMIT);

    found.rows = here.slice(0, shown).map(({ label, node }) => ({
      node: node.id,
      path: display(node),
      allocated: node.allocated,
      verdict: label.verdict,
      source: 'model',
      unreadable: false,
    }));
    found.more = Math.max(0, here.length - shown);

    return found;
  }

  /** Why a folder is big: what it is made of, how old it is, what the rules matched inside it and where it stands. */
  function explain(given: Scope): Found {
    const node = nodeOf(given);
    const summary = context.summary(node);
    const size = node.allocated;
    const found: Found = { ...nothingFound(), scopeSize: size, files: node.files };

    // The largest three parts, without one that rounds to nothing.
    for (const stat of summary.byCategory.slice(0, 3)) {
      if (percent(stat.allocated, size) > 0) {
        found.totals.push({
          label: CATEGORY_NAME[stat.category],
          bytes: stat.allocated,
          kind: 'category',
          percent: percent(stat.allocated, size),
        });
      }
    }

    const older = summary.byAge
      .filter((stat) => stat.bucket === 'twoYears' || stat.bucket === 'older')
      .reduce((bytes, stat) => bytes + stat.allocated, 0);

    found.totals.push({
      label: 'Older than a year',
      bytes: older,
      kind: 'older',
      percent: percent(older, size),
    });

    for (const entry of insideRules(node).slice(0, 3)) {
      found.totals.push({ label: entry.title, bytes: entry.bytes, kind: 'rule' });
    }

    if (given.at === 'folder') {
      const standing = standingNote(rowOf(node));

      if (standing !== null) {
        found.notes.push(standing);
      }
    }

    rowsOf(lifted(largestDirs(tree, node), node), found);

    return found;
  }

  /**
   * What the user is told of where the scope stands: only that the guard leaves it alone. Which rule covers it, or
   * that the model labelled it, is for the log and not for the answer.
   */
  function standingNote(row: Row): string | null {
    return row.source === 'guard' ? 'This folder is left alone by the safety guard.' : null;
  }

  /** Folders with a name, each with its standing: the folders of the rule that has a known place under that name. */
  function find(name: string): Found {
    const found = nothingFound();
    const rules = byRule(name);

    if (rules?.found && rules.located) {
      rowsOf(rules.nodes, found);

      return found;
    }

    // Files too: the mock's `find_entries`.
    rowsOf(lookup.searchEntries(name, MAX_FOUND), found);

    return found;
  }

  /** The one file a question is about, with its standing, whatever the intent. */
  function fileFound(node: MockNode): Found {
    const found = nothingFound();

    found.rows.push(rowOf(node));
    found.subject = true;

    return found;
  }

  /** The folder (or folders of a rule) a `find` is about, each with its standing. */
  function findScope(given: Scope): Found {
    const found = nothingFound();

    if (given.at === 'folder') {
      rowsOf([given.node], found);
    } else if (given.at === 'rules') {
      rowsOf(given.nodes, found);
    }

    return found;
  }

  /** The biggest folders of the whole scan, the totals of the suggestions and how many folders the model labelled. */
  function general(): Found {
    const found: Found = { ...nothingFound(), scopeSize: root.allocated, files: root.files };

    rowsOf(lifted(largestDirs(tree, root), root), found);

    // A verdict is told only when something has it.
    for (const verdict of ['safe', 'review', 'keep'] as const) {
      if (suggestions.totals[verdict] > 0) {
        found.totals.push(verdictTotal(verdict, suggestions.totals[verdict]));
      }
    }

    if (context.labels.length > 0) {
      found.totals.push({
        label: 'Folders the model labelled',
        count: context.labels.length,
        kind: 'plain',
      });
    }

    return found;
  }

  // ---- The answer ----

  /** How long before the scan a file changed, in whole days, and nothing for a file with no modification time. */
  const fileAge = (node: MockNode): { ageDays?: number } =>
    node.mtime > 0 ? { ageDays: Math.max(0, Math.floor((tree.now - node.mtime) / SECONDS_PER_DAY)) } : {};

  const notes: string[] = [];

  if (context.outdated) {
    notes.push(OUTDATED_NOTE);
  }

  // A reason is explained for one folder: of several that a rule matched, the largest.
  if (scope.at === 'rules' && intent === 'explain') {
    scope = { at: 'folder', node: scope.nodes[0] as MockNode };
  }

  let found: Found;

  if (intent === 'offtopic' || scope.at === 'missing') {
    found = nothingFound();
  } else if (scope.at === 'file') {
    found = fileFound(scope.node);
  } else if (intent === 'find') {
    found = reading.subject.kind === 'name' ? find(reading.subject.text) : findScope(scope);
  } else {
    switch (intent) {
      case 'largest':
        found = largest(scope);
        break;
      case 'cleanable':
        found = cleanable(scope);
        break;
      case 'labelled':
        found = labelled(scope);
        break;
      case 'explain':
        found = explain(scope);
        break;
      default:
        found = general();
        break;
    }
  }

  const contextPaths: string[] = [];

  if (scope.at === 'missing') {
    const shown = scope.absent?.named ?? scope.text;

    notes.push(notInScan(scope.text, scope.absent));
    // The user's own words may be a path; the answer may repeat them.
    contextPaths.push(shown);
  }

  notes.push(...found.notes, ...resolved.notes);

  if (intent === 'find' && reading.subject.kind === 'none') {
    notes.push('No folder name was given to look for.');
  } else if (intent === 'find' && found.rows.length === 0 && reading.subject.kind === 'name') {
    const absent = absentFor(query.scope);

    notes.push(notInScan(query.scope, absent));
    contextPaths.push(absent?.named ?? query.scope);
  }

  if (reading.everything && intent === 'cleanable') {
    notes.push(NOT_FROM_HERE_NOTE);
  }

  if (reading.polish && intent !== 'offtopic') {
    notes.push(ENGLISH_NOTE);
  }

  const shownNotes = notes.slice(0, MAX_NOTES).map(clip);
  const scopeNode = scope.at === 'folder' && intent !== 'find' ? scope.node : null;
  const fileNode = scope.at === 'file' ? scope.node : null;
  // A find says what it looked for, never a scope, whatever the typed path was.
  const scopedFile = intent === 'find' ? null : fileNode;
  const rows: ScanRowDto[] = found.rows.map((row) => ({
    node: row.node,
    path: row.path,
    allocated: row.allocated,
    ...(row.verdict ? { verdict: row.verdict } : {}),
    ...(row.source ? { source: row.source } : {}),
    ...(row.note ? { note: row.note } : {}),
    ...(row.file ? { file: true as const } : {}),
  }));

  const sheet: FactSheet = {
    intent,
    kind: query.kind,
    scope: scopeNode ? display(scopeNode) : '',
    ...(scope.at === 'rules' ? { set: scope.title } : {}),
    name: intent === 'find' ? query.scope : '',
    ...(query.kind === 'media' && reading.media ? { category: reading.media } : {}),
    scopeSize: found.scopeSize,
    files: found.files,
    subject: found.subject,
    rows: found.rows.map((row): FactRow => ({
      path: row.path,
      size: row.allocated,
      verdict: row.verdict,
      by: row.source,
      note: row.note,
      why: row.why,
      unreadable: row.unreadable,
    })),
    totals: found.totals,
    // The answer says an outdated scan by itself.
    notes: shownNotes.filter((note) => note !== OUTDATED_NOTE),
    outdated: context.outdated,
    more: found.more,
    contextPaths: [...contextPaths, ...found.contextPaths],
    ...(fileNode ? { file: fileAge(fileNode) } : {}),
  };

  const segments = templateSegments(sheet, units);

  /** The folder a path segment names, with the verdict and source `rowOf` gives it; none for a path left in the text. */
  const standing = (segment: Extract<Segment, { kind: 'path' }>): ScanRowDto | undefined =>
    segment.place.at === 'row' ? rows[segment.place.index] : scopeNode ? rowOf(scopeNode) : undefined;

  const answerSegments: AnswerSegmentDto[] = segments.map((segment): AnswerSegmentDto => {
    if (segment.kind === 'text') {
      return segment;
    }

    const folder = standing(segment);

    if (!folder) {
      return { kind: 'text', text: render([segment]) };
    }

    return {
      kind: 'path',
      node: folder.node,
      path: segment.path,
      ...(folder.verdict ? { verdict: folder.verdict } : {}),
      ...(folder.source ? { source: folder.source } : {}),
    };
  });

  return {
    scan: context.scan,
    understood: {
      intent,
      kind: query.kind,
      limit: query.limit,
      ...(scopeNode ? { scope: { node: scopeNode.id, path: display(scopeNode) } } : {}),
      ...(scopedFile
        ? { scope: { node: scopedFile.id, path: display(scopedFile), file: true as const } }
        : {}),
      ...(intent === 'find'
        ? query.scope !== ''
          ? { scopeText: query.scope }
          : {}
        : scope.at === 'missing'
          ? { scopeText: scope.text }
          : {}),
    },
    answer: render(segments),
    segments: answerSegments,
    rows,
    more: found.more,
    notes: shownNotes,
  };
}

/** The answer to a question that could mean more than one thing: the question asked back, and nothing that would answer it. */
function clarified(
  context: ScanQuestionContext,
  intent: ScanQuery['intent'],
  query: ScanQuery,
  clarify: Clarify,
): ScanAnswerDto {
  return {
    scan: context.scan,
    understood: {
      intent,
      kind: query.kind,
      limit: query.limit,
      ...(query.scope !== '' ? { scopeText: query.scope } : {}),
    },
    answer: clarify.prompt,
    segments: [{ kind: 'text', text: clarify.prompt }],
    rows: [],
    more: 0,
    notes: [],
    clarify: {
      prompt: clarify.prompt,
      choices: clarify.choices.map((choice, id) => ({
        id,
        label: choice.label,
        ...(choice.path === undefined ? {} : { path: choice.path }),
        ...(choice.node === undefined ? {} : { node: choice.node }),
      })),
    },
  };
}
