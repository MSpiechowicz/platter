import { REAL_ROOT, SKEW_ROOT } from './mock-layouts';
import { childrenOf, DAY, type MockNode, type MockTree } from './mock-tree';
import type { Category, NodeId, SuggestionDto, SuggestionsDto, Verdict } from './types';

// The mock's stand-in for `get_suggestions`: the rule table of `crates/platter-rules/src/table.rs` (same ids,
// titles, verdicts and reasons) evaluated over the demo tree the way `evaluate.rs` does.

/** The most nodes one suggestion lists, like the real backend. */
export const MAX_SUGGESTION_NODES = 20;

const GIB = 2 ** 30;

/** Older than (`idle`) or younger than (`recent`) this many days. */
interface Age {
  kind: 'idle' | 'recent';
  days: number;
}

/** A directory with one of these names, anywhere in the scan, optionally with conditions. */
interface Named {
  names: string[];
  /** The parent holds a regular file with this name. */
  sibling?: string;
  /** The parent holds a regular file with one of these names. */
  siblingAny?: string[];
  /** No folder on the way down to the directory has one of these names. */
  notInside?: string[];
  /** Not below one of these absolute paths. */
  notBelow?: string[];
  age?: Age;
}

/** Individual files below a folder of the home directory, selected by what they are. */
interface FilesMatcher {
  /** Folder relative to the home directory; empty means the whole home directory. */
  home: string;
  age?: Age;
  category?: Category;
  minBytes?: number;
}

type Matcher =
  | { type: 'fixed'; path: string }
  | { type: 'home'; relative: string }
  | { type: 'userDir'; name: string }
  | { type: 'named'; named: Named }
  | { type: 'files'; files: FilesMatcher };

interface Rule {
  id: string;
  title: string;
  verdict: Verdict;
  matcher: Matcher;
  /** A template: `{n:noun}`, `{n:one|many}`, `{days}` and `{size}` are filled in from the matches. */
  reason: string;
}

const rule = (id: string, title: string, verdict: Verdict, matcher: Matcher, reason: string): Rule => ({
  id,
  title,
  verdict,
  matcher,
  reason,
});

const fixed = (path: string): Matcher => ({ type: 'fixed', path });
const home = (relative: string): Matcher => ({ type: 'home', relative });
const userDir = (name: string): Matcher => ({ type: 'userDir', name });
const named = (value: Named): Matcher => ({ type: 'named', named: value });
const files = (value: FilesMatcher): Matcher => ({ type: 'files', files: value });

const NODE_LOCKFILES = [
  'package-lock.json',
  'npm-shrinkwrap.json',
  'pnpm-lock.yaml',
  'yarn.lock',
  'bun.lock',
  'bun.lockb',
];

const NODE_TOOL_DIRS = [
  'node_modules',
  '.nvm',
  '.volta',
  '.fnm',
  '.npm-global',
  '.vscode',
  '.vscode-insiders',
  '.vscode-oss',
  '.vscode-server',
  '.cursor',
  '.cursor-server',
  '.windsurf',
];

const nodeModules = (age: Age): Matcher =>
  named({
    names: ['node_modules'],
    sibling: 'package.json',
    siblingAny: NODE_LOCKFILES,
    notInside: NODE_TOOL_DIRS,
    notBelow: ['/usr', '/opt'],
    age,
  });

const KEPT_REASON = (what: string): string =>
  `Your ${what} folder. Left alone on purpose; nothing inside is suggested for cleanup.`;

// Same order as the backend's table: when two rules match one folder, a `keep` rule wins, then the earlier one.
const RULES: Rule[] = [
  // Safe: regenerated or downloaded again on demand.
  rule(
    'apt-cache',
    'APT package archives',
    'safe',
    fixed('/var/cache/apt/archives'),
    'Package files APT downloaded for installs and upgrades. APT downloads them again when it needs them.',
  ),
  rule(
    'dnf-cache',
    'DNF cache',
    'safe',
    fixed('/var/cache/dnf'),
    'Package metadata and downloads that DNF keeps. DNF downloads them again when it needs them.',
  ),
  rule(
    'rust-target',
    'Rust build output',
    'safe',
    named({ names: ['target'], sibling: 'Cargo.toml', age: { kind: 'idle', days: 30 } }),
    'Build output that cargo creates again on the next build. {n:project} with nothing modified in over {days} days.',
  ),
  rule(
    'node-modules',
    'node_modules',
    'safe',
    nodeModules({ kind: 'idle', days: 60 }),
    'Dependencies that a package manager installs again from package.json. {n:project} with nothing modified in over {days} days.',
  ),
  rule(
    'python-cache',
    'Python caches',
    'safe',
    named({ names: ['__pycache__', '.pytest_cache', '.mypy_cache', '.ruff_cache'] }),
    'Bytecode and tool caches that Python, pytest, mypy and ruff recreate on the next run. Found in {n:folder}.',
  ),
  rule(
    'npm-cache',
    'npm cache',
    'safe',
    home('.npm'),
    'Packages npm downloaded before. npm downloads them again when a project needs them.',
  ),
  rule(
    'bun-cache',
    'Bun install cache',
    'safe',
    home('.bun/install/cache'),
    'Packages Bun downloaded before. Bun downloads them again when a project needs them.',
  ),
  rule(
    'pnpm-store',
    'pnpm store',
    'safe',
    home('.pnpm-store'),
    'The shared package store of pnpm. pnpm downloads again what a project needs and the store no longer has. Files that projects still link to are not freed until those projects are cleaned too.',
  ),
  rule(
    'yarn-cache',
    'Yarn cache',
    'safe',
    home('.yarn/cache'),
    'Packages Yarn downloaded before. Yarn downloads them again when a project needs them.',
  ),
  rule(
    'cargo-registry',
    'Cargo registry',
    'safe',
    home('.cargo/registry'),
    'Crate sources and indexes cargo downloaded. Cargo fetches the ones a build needs again, which requires a network connection.',
  ),
  rule(
    'gradle-cache',
    'Gradle caches',
    'safe',
    home('.gradle/caches'),
    'Dependencies and build caches of Gradle. Gradle downloads or rebuilds what a build needs.',
  ),
  rule(
    'nuget-packages',
    'NuGet packages',
    'safe',
    home('.nuget/packages'),
    'Packages NuGet downloaded. NuGet restores the ones a build needs.',
  ),
  rule(
    'go-modules',
    'Go module cache',
    'safe',
    home('go/pkg/mod'),
    'Module sources Go downloaded. Go fetches the ones a build needs again, which requires a network connection.',
  ),
  rule(
    'trash',
    'Trash',
    'safe',
    home('.local/share/Trash'),
    'Files moved to the trash. Emptying it deletes them for good; a file that is also hard-linked elsewhere keeps its space.',
  ),
  rule(
    'coredumps',
    'Core dumps',
    'safe',
    fixed('/var/lib/systemd/coredump'),
    'Memory dumps saved when programs crashed. They are mainly useful for debugging those crashes.',
  ),
  rule(
    'steam-shader-cache',
    'Steam shader cache',
    'safe',
    home('.local/share/Steam/steamapps/shadercache'),
    'Compiled shaders Steam keeps for games. Steam rebuilds them the next time a game needs them, which can cause brief stutter.',
  ),

  // Review: probably removable, but only the owner can tell.
  rule(
    'pacman-cache',
    'Pacman package cache',
    'review',
    fixed('/var/cache/pacman/pkg'),
    'Package files pacman keeps after installing them, including older versions you could downgrade to. Pacman downloads a package again if it needs one that is gone, but an old version may no longer be available.',
  ),
  rule(
    'journal',
    'System journal',
    'review',
    fixed('/var/log/journal'),
    'System logs kept by systemd-journald, including the current boot. Removing entries loses that history; journalctl can trim them for you.',
  ),
  rule(
    'app-cache',
    'Application caches',
    'review',
    home('.cache'),
    'Rebuilt by the apps that use it. Large downloads kept here, such as models, would have to be downloaded again, and some apps keep things here that cannot be rebuilt.',
  ),
  rule(
    'maven-repository',
    'Maven repository',
    'review',
    home('.m2/repository'),
    'Artifacts Maven downloaded, and those your own builds installed with mvn install. Downloaded ones are fetched again when a build needs them; installed ones exist only here.',
  ),
  rule(
    'docker-data',
    'Docker data',
    'review',
    fixed('/var/lib/docker'),
    'Images, containers and volumes. Volumes can hold data that exists nowhere else, so look before removing anything.',
  ),
  rule(
    'docker-desktop-vm',
    'Docker Desktop virtual machine',
    'review',
    home('.docker/desktop/vms'),
    'The disk image Docker Desktop keeps its images, containers and volumes in. Removing it deletes all of them.',
  ),
  rule(
    'btrfs-snapshots',
    'btrfs snapshots',
    'review',
    fixed('/.snapshots'),
    'Snapshots in the default snapper location. Deleting them removes those rollback points; how much space that frees depends on how much the snapshots share with the live system.',
  ),
  rule(
    'old-downloads',
    'Old downloads',
    'review',
    files({ home: 'Downloads', age: { kind: 'idle', days: 365 } }),
    '{n:file} in Downloads not modified for {days} days or more. These are personal files, so only you can tell whether you still need them.',
  ),
  rule(
    'disk-images',
    'Virtual machine and disk images',
    'review',
    files({ home: '', category: 'diskImage', minBytes: GIB }),
    '{n:image} of {size} or more in your home folder. Often a whole virtual machine, so check before removing one.',
  ),
  rule(
    'rust-target-recent',
    'Rust build output (recent)',
    'review',
    named({ names: ['target'], sibling: 'Cargo.toml', age: { kind: 'recent', days: 30 } }),
    '{n:project} touched in the last {days} days, so you are probably still working on them. Removing the build output means a full rebuild.',
  ),
  rule(
    'node-modules-recent',
    'node_modules (recent)',
    'review',
    nodeModules({ kind: 'recent', days: 60 }),
    '{n:project} touched in the last {days} days, so you are probably still working on them. Removing the dependencies means a reinstall.',
  ),

  // Keep: shown so the user can see what is deliberately left alone. Nothing inside is suggested.
  rule(
    'steam-library',
    'Steam library',
    'keep',
    home('.local/share/Steam/steamapps/common'),
    'Installed games. Left alone on purpose: they would have to be downloaded again.',
  ),
  rule('documents', 'Documents', 'keep', userDir('Documents'), KEPT_REASON('documents')),
  rule('pictures', 'Pictures', 'keep', userDir('Pictures'), KEPT_REASON('pictures')),
  rule('videos', 'Videos', 'keep', userDir('Videos'), KEPT_REASON('videos')),
  rule('music', 'Music', 'keep', userDir('Music'), KEPT_REASON('music')),
  rule('desktop', 'Desktop', 'keep', userDir('Desktop'), KEPT_REASON('desktop')),
];

const VERDICT_ORDER: Record<Verdict, number> = { safe: 0, review: 1, keep: 2 };

/** Where the scan lives, and which folder is home. */
interface Environment {
  /** Path of the scan root. The demo disks `?root=/real` and `?root=/skew` stand for a whole root filesystem. */
  root: string;
  /** The home directory, when there is one in the scan or above it. Without one nothing can be kept. */
  home: string | null;
  now: number;
}

function environmentOf(tree: MockTree): Environment {
  const root = tree.rootPath === REAL_ROOT || tree.rootPath === SKEW_ROOT ? '/' : tree.rootPath;
  const inside = /^\/home\/[^/]+/.exec(root);

  if (inside) {
    return { root, home: inside[0], now: tree.now };
  }

  if (root === '/') {
    const top = childrenOf(tree, tree.nodes[0] as MockNode).find((child) => child.name === 'home');
    const user = top && childrenOf(tree, top).find((child) => child.kind === 'dir');

    return { root, home: user ? `/home/${user.name}` : null, now: tree.now };
  }

  return { root, home: null, now: tree.now };
}

/** The folder a path rule (fixed, home or personal folder) is about, when it is known. */
function claimedPath(matcher: Matcher, env: Environment): string | null {
  switch (matcher.type) {
    case 'fixed':
      return matcher.path;
    case 'home':
      return env.home && `${env.home}/${matcher.relative}`;
    case 'userDir':
      return env.home && `${env.home}/${matcher.name}`;
    default:
      return null;
  }
}

/** The folder whose files a file rule looks at, when it is known. */
function regionPath(files: FilesMatcher, env: Environment): string | null {
  if (!env.home) {
    return null;
  }

  return files.home === '' ? env.home : `${env.home}/${files.home}`;
}

function holds(age: Age | undefined, mtime: number, now: number): boolean {
  if (!age) {
    return true;
  }

  const seconds = now - mtime;
  const limit = age.days * DAY;

  return age.kind === 'idle' ? seconds >= limit : seconds < limit;
}

/** `/home/me/code` becomes `~/code` and the home directory itself `~`; other paths are kept. */
function shortenHome(path: string, home: string | null): string {
  if (home && path === home) {
    return '~';
  }

  return home && path.startsWith(`${home}/`) ? `~${path.slice(home.length)}` : path;
}

const joinPath = (base: string, tail: string): string =>
  base.endsWith('/') ? `${base}${tail}` : `${base}/${tail}`;

/** The deepest folder that holds all the paths. */
function commonParent(paths: string[]): string {
  const split = paths.map((path) => path.split('/').slice(0, -1));
  const first = split[0] ?? [];
  let length = first.length;

  for (const parts of split) {
    let shared = 0;

    while (shared < length && parts[shared] === first[shared]) {
      shared++;
    }

    length = shared;
  }

  return first.slice(0, length).join('/') || '/';
}

/** The rule's work so far: sums, the matched nodes and where they are. */
interface Found {
  allocated: number;
  sharedFiles: number;
  nodes: MockNode[];
  /** Absolute paths, in the order of `nodes`. */
  paths: string[];
  /** Which of a name rule's names matched. */
  names: Set<string>;
}

/** Whether `node` or a directory below it could not be read. */
function hasUnreadable(tree: MockTree, node: MockNode): boolean {
  if (node.kind !== 'dir') {
    return false;
  }

  const stack = [node];

  for (let current = stack.pop(); current; current = stack.pop()) {
    if (current.unreadable) {
      return true;
    }

    for (const child of childrenOf(tree, current)) {
      if (child.kind === 'dir') {
        stack.push(child);
      }
    }
  }

  return false;
}

/** A regular file called `marker` sits next to `node`. */
function hasSiblingFile(tree: MockTree, node: MockNode, marker: string): boolean {
  const parent = tree.nodes[node.parent];

  return (
    parent !== undefined &&
    childrenOf(tree, parent).some((sibling) => sibling.name === marker && sibling.kind === 'file')
  );
}

/** A rule's verdict on this machine: a name rule is not trusted with `safe` when no home is known. */
function effectiveVerdict(entry: Rule, env: Environment): Verdict {
  return entry.verdict === 'safe' && entry.matcher.type === 'named' && !env.home ? 'review' : entry.verdict;
}

/** Whether the directory `node` at `path` carries the name and satisfies every condition of `rule`. */
function namedMatches(tree: MockTree, env: Environment, rule: Named, node: MockNode, path: string): boolean {
  if (!rule.names.includes(node.name) || !holds(rule.age, node.mtime, env.now)) {
    return false;
  }

  if (rule.sibling !== undefined && !hasSiblingFile(tree, node, rule.sibling)) {
    return false;
  }

  if (rule.siblingAny && !rule.siblingAny.some((marker) => hasSiblingFile(tree, node, marker))) {
    return false;
  }

  const parentParts = path.split('/').slice(0, -1);

  if (rule.notInside?.some((name) => parentParts.includes(name))) {
    return false;
  }

  return !rule.notBelow?.some((prefix) => path === prefix || path.startsWith(`${prefix}/`));
}

/** The single pass: walks the tree from the top and hands every rule the nodes it matched. */
class Evaluation {
  readonly found = new Map<Rule, Found>();
  private readonly regions: { rule: Rule; files: FilesMatcher; path: string }[] = [];

  constructor(
    private readonly tree: MockTree,
    private readonly env: Environment,
  ) {
    for (const entry of RULES) {
      const path = entry.matcher.type === 'files' ? regionPath(entry.matcher.files, env) : null;

      if (path !== null && entry.matcher.type === 'files') {
        this.regions.push({ rule: entry, files: entry.matcher.files, path });
      }
    }
  }

  /** Walks down from the scan root. A folder some rule takes is not entered again, so no byte is counted twice. */
  run(): void {
    const stack = [{ node: this.tree.nodes[0] as MockNode, path: this.env.root, active: [] as Rule[] }];

    for (let current = stack.pop(); current; current = stack.pop()) {
      const { node, path } = current;

      if (node.kind === 'file') {
        this.matchFile(node, path, current.active);
        continue;
      }

      if (node.kind !== 'dir') {
        continue;
      }

      const isRoot = node.parent < 0;
      const matched = this.matchingRule(node, path, isRoot);

      if (matched && this.take(matched.rule, matched.name, node, path)) {
        continue;
      }

      const active = this.activeBelow(path, current.active);
      const prefix = path === '/' ? '' : path;

      for (const child of childrenOf(this.tree, node)) {
        stack.push({ node: child, path: `${prefix}/${child.name}`, active });
      }
    }
  }

  /** The file rules active below `path`: the inherited ones plus those whose folder this is. */
  private activeBelow(path: string, inherited: Rule[]): Rule[] {
    const here = this.regions.filter((region) => region.path === path).map((region) => region.rule);

    return here.length === 0
      ? inherited
      : RULES.filter((entry) => inherited.includes(entry) || here.includes(entry));
  }

  /**
   * The rule that claims this directory: a `keep` rule first, then the earliest of the path and name rules. The scan
   * root has no parent in the tree, so a name rule cannot check its surroundings there.
   */
  private matchingRule(node: MockNode, path: string, isRoot: boolean): { rule: Rule; name?: string } | null {
    const claims = RULES.filter((entry) => claimedPath(entry.matcher, this.env) === path);
    const kept = claims.find((entry) => entry.verdict === 'keep');
    let best: Rule | undefined = kept ?? claims[0];
    let name: string | undefined;

    if (kept || isRoot) {
      return best ? { rule: best } : null;
    }

    for (const entry of RULES) {
      if (best && RULES.indexOf(best) < RULES.indexOf(entry)) {
        break;
      }

      if (
        entry.matcher.type === 'named' &&
        namedMatches(this.tree, this.env, entry.matcher.named, node, path)
      ) {
        best = entry;
        name = node.name;
        break;
      }
    }

    return best ? { rule: best, name } : null;
  }

  /** Records the directory under `rule` unless it may not be suggested. Returns whether the folder is taken (not entered). */
  private take(rule: Rule, name: string | undefined, node: MockNode, path: string): boolean {
    // A kept folder is never entered, whatever is in it.
    if (rule.verdict === 'keep') {
      if (node.allocated > 0) {
        this.add(rule, node, path, name);
      }

      return true;
    }

    // Unreadable folders have an unknown size and empty ones nothing to offer. A folder with an unreadable part is
    // never `safe`: its size and age only cover what was read.
    const verdict = effectiveVerdict(rule, this.env);
    const suggestible =
      node.allocated > 0 && !node.unreadable && (verdict !== 'safe' || !hasUnreadable(this.tree, node));

    if (suggestible) {
      this.add(rule, node, path, name);
    }

    return suggestible;
  }

  /** Records the file under the first active file rule it satisfies. */
  private matchFile(node: MockNode, path: string, active: Rule[]): void {
    for (const entry of active) {
      if (entry.matcher.type !== 'files') {
        continue;
      }

      const { age, category, minBytes } = entry.matcher.files;

      if (
        node.allocated > 0 &&
        node.allocated >= (minBytes ?? 0) &&
        (category === undefined || node.category === category) &&
        holds(age, node.mtime, this.env.now)
      ) {
        this.add(entry, node, path, undefined);
        return;
      }
    }
  }

  private add(rule: Rule, node: MockNode, path: string, name: string | undefined): void {
    let found = this.found.get(rule);

    if (!found) {
      found = { allocated: 0, sharedFiles: 0, nodes: [], paths: [], names: new Set() };
      this.found.set(rule, found);
    }

    found.allocated += node.allocated;
    found.sharedFiles += node.sharedFiles;
    found.nodes.push(node);
    found.paths.push(path);

    if (name !== undefined) {
      found.names.add(name);
    }
  }
}

/** The names that really matched: `target`, or `{__pycache__,.mypy_cache}` for several, in the rule's order. */
function namePattern(rule: Named, matched: Set<string>): string {
  const names = rule.names.filter((name) => matched.has(name));

  return names.length === 1 ? (names[0] as string) : `{${names.join(',')}}`;
}

/** The path or pattern shown for a rule: its own path for fixed and home rules, the real path for a single match, otherwise a glob. */
function displayPath(rule: Rule, found: Found, env: Environment): string {
  const { matcher } = rule;

  switch (matcher.type) {
    case 'fixed':
      return matcher.path;
    case 'home':
      return matcher.relative === '' ? '~' : `~/${matcher.relative}`;
    case 'userDir':
      return `~/${matcher.name}`;
    case 'named':
    case 'files': {
      if (found.nodes.length === 1) {
        return shortenHome(found.paths[0] as string, env.home);
      }

      const pattern = matcher.type === 'named' ? `**/${namePattern(matcher.named, found.names)}` : '**';

      return joinPath(shortenHome(commonParent(found.paths), env.home), pattern);
    }
  }
}

/** `1073741824` becomes `1 GiB`, `1610612736` becomes `1.5 GiB`. */
function sizeText(bytes: number): string {
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let value = bytes;
  let unit = 0;

  while (value >= 1024 && unit + 1 < units.length) {
    value /= 1024;
    unit++;
  }

  return `${Number.isInteger(value) ? value : value.toFixed(1)} ${units[unit]}`;
}

/** Fills `{n:noun}`, `{n:one|many}`, `{days}` and `{size}`. Anything else in braces is kept as written. */
function renderReason(rule: Rule, count: number): string {
  const { matcher } = rule;
  const age =
    matcher.type === 'named' ? matcher.named.age : matcher.type === 'files' ? matcher.files.age : undefined;
  const minBytes = matcher.type === 'files' ? matcher.files.minBytes : undefined;

  return rule.reason.replace(/\{([^}]*)\}/g, (token: string, inner: string) => {
    if (inner === 'days') {
      return age ? String(age.days) : token;
    }

    if (inner === 'size') {
      return minBytes === undefined ? token : sizeText(minBytes);
    }

    if (!inner.startsWith('n:')) {
      return token;
    }

    const nouns = inner.slice(2);
    const [singular, plural] = nouns.includes('|') ? nouns.split('|') : [nouns, `${nouns}s`];

    return `${count} ${count === 1 ? singular : plural}`;
  });
}

/** The ids of all rules, in the table's order (the backend plans groups in this order). */
export const RULE_IDS: readonly string[] = RULES.map((entry) => entry.id);

/** The mock's stand-in for `get_suggestions`. */
export function suggestionsOf(tree: MockTree): SuggestionsDto {
  const env = environmentOf(tree);
  const empty: SuggestionsDto = { suggestions: [], totals: { safe: 0, review: 0, keep: 0 } };

  // A scan that starts inside a kept folder (but is not that folder) gets no suggestions.
  const insideKept = RULES.some((entry) => {
    const kept = claimedPath(entry.matcher, env);

    return entry.verdict === 'keep' && kept !== null && env.root !== kept && env.root.startsWith(`${kept}/`);
  });

  if (insideKept) {
    return empty;
  }

  const evaluation = new Evaluation(tree, env);
  const totals = { ...empty.totals };
  const ranked: { index: number; suggestion: SuggestionDto }[] = [];

  evaluation.run();
  RULES.forEach((entry, index) => {
    const found = evaluation.found.get(entry);

    if (!found) {
      return;
    }

    const verdict = effectiveVerdict(entry, env);
    const largest = found.nodes
      .slice()
      .sort((a, b) => b.allocated - a.allocated || a.id - b.id)
      .slice(0, MAX_SUGGESTION_NODES);

    totals[verdict] += found.allocated;
    ranked.push({
      index,
      suggestion: {
        id: entry.id,
        title: entry.title,
        path: displayPath(entry, found, env),
        verdict,
        reason: renderReason(entry, found.nodes.length),
        allocated: found.allocated,
        items: found.nodes.length,
        sharedFiles: found.sharedFiles,
        nodes: largest.map((node): NodeId => node.id),
      },
    });
  });

  ranked.sort(
    (a, b) =>
      VERDICT_ORDER[a.suggestion.verdict] - VERDICT_ORDER[b.suggestion.verdict] ||
      b.suggestion.allocated - a.suggestion.allocated ||
      a.index - b.index,
  );

  return { suggestions: ranked.map((entry) => entry.suggestion), totals };
}
