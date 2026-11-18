import { baseName, joinPath } from '../paths';
import { CATEGORIES, isProjectMarker, ofDirectory, ofFile } from './mock-categories';
import { layoutFor, type DirSpec, type FileGroup } from './mock-layouts';
import type { Category, NodeKind, OsName, SkipReason } from './types';

export { CATEGORIES };

export const BLOCK_SIZE = 4096;
export const DAY = 86_400;

/** A node of the in-memory tree. The id is the index in `MockTree.nodes`; children are contiguous. */
export interface MockNode {
  id: number;
  parent: number;
  name: string;
  kind: NodeKind;
  allocated: number;
  apparent: number;
  /** Number of non-directory entries below (a file counts as 1 for itself). */
  files: number;
  category: Category;
  mtime: number;
  unreadable?: boolean;
  /** Why a `mount` was not entered. */
  skipped?: SkipReason;
  /** Files below (or the file itself) with more than one hard link. */
  sharedFiles: number;
  firstChild: number;
  childCount: number;
}

export interface MockTree {
  nodes: MockNode[];
  /** Scan time in unix seconds; `byAge` buckets are relative to it. */
  now: number;
  rootPath: string;
  /**
   * What this machine calls its personal folders, by their English name, below the home folder
   * (`{ Downloads: 'Pobrane' }`; the XDG `user-dirs.dirs` of the backend). The demo disks name `Downloads`. A hand-made
   * tree may leave a folder out, and then it has its English name.
   */
  userDirs?: Readonly<Record<string, string>>;
}

export interface TreeOptions {
  seed: number;
  root: string;
  oneFileSystem: boolean;
  /** Bytes the whole tree should add up to (roughly). */
  bytes: number;
  now: number;
  /** Use the lopsided demo layout, where one folder holds nearly everything. */
  dominant?: boolean;
  /** The system the demo disk stands for (default `linux`): a Mac has `Users`, `Applications` and `Library` where Linux has `home`. */
  os?: OsName;
  /** Absolute paths of folders the user leaves out of scans. Never the root. */
  excluded?: readonly string[];
}

/** Deterministic random numbers in [0, 1). */
export function mulberry32(seed: number): () => number {
  let state = seed >>> 0;

  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);

    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

type Random = () => number;

function gaussian(random: Random): number {
  const u = Math.max(random(), 1e-9);
  const v = random();

  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
}

function pick<T>(random: Random, items: readonly T[]): T {
  return items[Math.floor(random() * items.length)] as T;
}

// prettier-ignore
const WORDS = [
  'atlas', 'birch', 'cobalt', 'delta', 'ember', 'falcon', 'granite', 'harbor', 'iris', 'juniper',
  'krill', 'lagoon', 'meadow', 'nimbus', 'orchid', 'pebble', 'quartz', 'raven', 'summit', 'tundra',
  'umber', 'velvet', 'willow', 'xenon', 'yarrow', 'zephyr', 'anchor', 'beacon', 'cinder', 'dune',
];

/** Extensions a file of a category gets in its name. Each one maps back to the category (`ofFile`). */
const NAME_EXTENSIONS: Record<Category, string[]> = {
  video: ['mkv', 'mp4', 'webm'],
  image: ['jpg', 'png', 'webp', 'heic'],
  audio: ['flac', 'mp3', 'ogg'],
  document: ['pdf', 'odt', 'md', 'txt'],
  archive: ['zip', 'tar.zst', '7z', 'tar.gz'],
  code: ['rs', 'ts', 'svelte', 'py', 'toml'],
  build: ['o', 'rlib', 'rmeta', 'd'],
  package: ['deb', 'rpm', 'whl', 'jar'],
  app: ['so', 'pak', 'dll', 'dylib'],
  diskImage: ['iso', 'qcow2', 'img'],
  data: ['sqlite', 'parquet', 'gguf', 'jsonl'],
  other: ['dat', 'bin', 'cache', 'tmp'],
};

function fileName(random: Random, category: Category, index: number): string {
  const word = pick(random, WORDS);
  const ext = pick(random, NAME_EXTENSIONS[category]);

  if (category === 'image') {
    return `IMG_${String(1000 + index).padStart(4, '0')}.${ext}`;
  }

  return `${word}-${index}.${ext}`;
}

interface Draft {
  name: string;
  kind: NodeKind;
  category: Category;
  apparent: number;
  allocated: number;
  mtime: number;
  unreadable?: boolean;
  skipped?: SkipReason;
  /** Set for each of the names of a file that has more than one hard link, the same number for all of them. */
  link?: number;
  kids: Draft[];
}

interface Context {
  random: Random;
  now: number;
  oneFileSystem: boolean;
  excluded: ReadonlySet<string>;
  /** Hard-link groups made so far. */
  links: number;
  /** The second names of hard-linked files that a parent folder has not yet collected. */
  partners: Draft[];
}

/** A directory owns one block, like on a real disk; `apparent` stays 0, as `du` counts it. */
const dirDraft = (name: string, now: number): Draft => ({
  name,
  kind: 'dir',
  category: 'other',
  apparent: 0,
  allocated: BLOCK_SIZE,
  mtime: now - 30 * DAY,
  kids: [],
});

/** A mount that is left out: a virtual filesystem always, any other only when the scan stays on one filesystem. */
const isSkipped = (spec: DirSpec, ctx: Context): boolean =>
  spec.mount === true && (ctx.oneFileSystem || spec.skipped === 'system');

/** The path of a folder below `parent`. */
const below = joinPath;

function weightOf(spec: DirSpec, ctx: Context, path: string): number {
  return isSkipped(spec, ctx) || ctx.excluded.has(path) ? 0 : spec.weight;
}

function buildDir(spec: DirSpec, bytes: number, ctx: Context, path: string, isRoot = false): Draft {
  // A folder the user left out: never read, counted as nothing, as the backend's walker does.
  if (!isRoot && ctx.excluded.has(path)) {
    return { ...dirDraft(spec.name, ctx.now), kind: 'mount', allocated: 0, skipped: 'excluded' };
  }

  if (isSkipped(spec, ctx)) {
    return {
      ...dirDraft(spec.name, ctx.now),
      kind: 'mount',
      allocated: 0,
      ...(spec.skipped ? { skipped: spec.skipped } : {}),
    };
  }

  if (spec.unreadable || spec.empty) {
    return { ...dirDraft(spec.name, ctx.now), allocated: 0, unreadable: spec.unreadable };
  }

  const draft = dirDraft(spec.name, ctx.now);
  const kids = spec.kids ?? [];
  const filesWeight = spec.files ? (spec.files.weight ?? 1) : 0;
  // A left-out folder keeps its share of the split, so what it held is missing from the total, as on a real disk.
  const share = (kid: DirSpec): number =>
    ctx.excluded.has(below(path, kid.name)) && !isSkipped(kid, ctx)
      ? kid.weight
      : weightOf(kid, ctx, below(path, kid.name));
  const total = kids.reduce((sum, kid) => sum + share(kid), 0) + filesWeight;

  const linked: Draft[] = [];

  for (const kid of kids) {
    const kidPath = below(path, kid.name);
    const weight = weightOf(kid, ctx, kidPath);

    // A mount point that is not entered still shows up, as an empty `mount` node.
    if (weight > 0 || kid.mount || kid.unreadable || kid.empty || ctx.excluded.has(kidPath)) {
      draft.kids.push(buildDir(kid, total > 0 ? (bytes * weight) / total : 0, ctx, kidPath));
      linked.push(...ctx.partners.splice(0));
    }
  }

  if (spec.files) {
    addFiles(draft, spec.files, (bytes * filesWeight) / total, ctx);
  }

  for (const name of spec.markers ?? []) {
    draft.kids.push(markerFile(name, ctx.now));
  }

  if (linked.length > 0) {
    draft.kids.push(packageStore(linked, ctx.now));
  }

  return draft;
}

/**
 * Where the other names of the hard-linked files live: a pnpm store inside the project, as `store-dir` can put it.
 * It is deeper than the files it shares with, so the scan, which charges the first name it meets, charges the originals.
 */
function packageStore(partners: Draft[], now: number): Draft {
  const store = dirDraft('.pnpm-store', now);
  const version = dirDraft('v3', now);
  const files = dirDraft('files', now);

  files.kids = partners;
  version.kids = [files];
  store.kids = [version];

  return store;
}

/** A small file such as `Cargo.toml`. It has no weight and does not touch the random numbers, so the rest of the tree stays as it is. */
function markerFile(name: string, now: number): Draft {
  return {
    name,
    kind: 'file',
    category: ofFile(name, 300),
    apparent: 300,
    allocated: BLOCK_SIZE,
    mtime: now - 200 * DAY,
    kids: [],
  };
}

function addFiles(parent: Draft, group: FileGroup, bytes: number, ctx: Context): void {
  const { random, now } = ctx;
  const sigma = group.sigma ?? 1.1;
  const categories = Array.isArray(group.category) ? group.category : [group.category];
  const weights = Array.from({ length: group.count }, () => Math.exp(sigma * gaussian(random)));
  const sum = weights.reduce((a, b) => a + b, 0);

  let bucket: Draft | null = null;
  const bucketSize = group.bucket?.size ?? 0;

  weights.forEach((weight, index) => {
    if (group.bucket && index % bucketSize === 0) {
      bucket = dirDraft(group.bucket.name(index / bucketSize), now);
      parent.kids.push(bucket);
    }

    const target = bucket ?? parent;
    const category = pick(random, categories);
    const apparent = Math.max(200, Math.floor((bytes * weight) / sum));
    const ageDays = Math.min(
      4000,
      (group.minAgeDays ?? 0) + -Math.log(Math.max(random(), 1e-6)) * (group.ageDays ?? 180),
    );
    const mtime = Math.floor(now - ageDays * DAY - random() * DAY);

    // About one entry in 250 is a symlink: it counts its own tiny size only.
    if (random() < 0.004) {
      target.kids.push({
        name: `link-${index}`,
        kind: 'symlink',
        category: 'other',
        apparent: 24,
        allocated: 0,
        mtime,
        kids: [],
      });
      return;
    }

    const name = fileName(random, category, index);
    const file: Draft = {
      name,
      kind: 'file',
      category: ofFile(name, apparent),
      apparent,
      allocated: Math.ceil(apparent / BLOCK_SIZE) * BLOCK_SIZE,
      mtime,
      kids: [],
    };

    if (group.sharedEvery && index % group.sharedEvery === 0) {
      file.link = ctx.links++;
      ctx.partners.push({ ...file, kids: [] });
    }

    target.kids.push(file);
  });
}

/** What the real backend calls the root node: the last path component, or `/` (`C:\`) for the filesystem root. */
export const rootName = baseName;

/** Builds a realistic demo tree: ids in breadth-first order, parents before children, children contiguous. */
export function buildTree(options: TreeOptions): MockTree {
  const ctx: Context = {
    random: mulberry32(options.seed),
    now: options.now,
    oneFileSystem: options.oneFileSystem,
    excluded: new Set(options.excluded ?? []),
    links: 0,
    partners: [],
  };
  const rootSpec: DirSpec = {
    name: rootName(options.root),
    weight: 1,
    kids: layoutFor(options.root, options.dominant, options.os),
  };
  const rootDraft = buildDir(rootSpec, options.bytes, ctx, options.root, true);

  return flatten(rootDraft, options);
}

/** A folder waiting for its children to be copied into the arena. */
interface Pending {
  draft: Draft;
  id: number;
  /** Category imposed by a well-known ancestor folder, if any. */
  inherited: Category | null;
}

/**
 * Ids in breadth-first order, as the backend's assembler gives them. The category of a folder that a well-known name
 * covers (`node_modules`) is imposed on everything below it, the outermost such folder winning. A file otherwise has
 * the category of its name. The first name of a hard-linked file that the walk meets is charged its bytes, the others
 * have none.
 */
function flatten(rootDraft: Draft, options: TreeOptions): MockTree {
  const nodes: MockNode[] = [];
  const queue: Pending[] = [];
  const seenLinks = new Set<number>();

  const add = (draft: Draft, parent: number, category: Category, inherited: Category | null): void => {
    const id = nodes.length;
    const charged = draft.link === undefined || !seenLinks.has(draft.link);

    if (draft.link !== undefined) {
      seenLinks.add(draft.link);
    }

    nodes.push({
      id,
      parent,
      name: draft.name,
      kind: draft.kind,
      allocated: charged ? draft.allocated : 0,
      apparent: charged ? draft.apparent : 0,
      files: draft.kind === 'dir' || draft.kind === 'mount' ? 0 : 1,
      category,
      mtime: draft.mtime,
      ...(draft.unreadable ? { unreadable: true } : {}),
      ...(draft.skipped ? { skipped: draft.skipped } : {}),
      sharedFiles: draft.link === undefined ? 0 : 1,
      firstChild: 0,
      childCount: 0,
    });
    queue.push({ draft, id, inherited });
  };

  const rootImposed = ofDirectory(null, rootDraft.name, () => false);

  add(rootDraft, -1, rootImposed ?? 'other', rootImposed);

  for (let head = 0; head < queue.length; head++) {
    const entry = queue[head];

    if (!entry) {
      break;
    }

    const node = nodes[entry.id] as MockNode;
    const kids = entry.draft.kids;
    let projectLike: boolean | undefined;
    const isProject = (): boolean => (projectLike ??= kids.some((kid) => isProjectMarker(kid.name)));

    node.firstChild = nodes.length;
    node.childCount = kids.length;

    for (const kid of kids) {
      if (kid.kind === 'dir') {
        const imposed = entry.inherited ?? ofDirectory(entry.draft.name, kid.name, isProject);

        add(kid, node.id, imposed ?? 'other', imposed);
      } else if (kid.kind === 'mount') {
        add(kid, node.id, 'other', null);
      } else {
        add(kid, node.id, entry.inherited ?? kid.category, null);
      }
    }
  }

  aggregate(nodes, options.now);

  return {
    nodes,
    now: options.now,
    rootPath: options.root,
    userDirs: { Downloads: 'Downloads' },
  };
}

/**
 * One reverse pass: every child is folded into its parent. A folder's category is the one holding most of the bytes
 * of its files (ties go to the first in `CATEGORIES`). A folder without bytes keeps the category its name imposes, or `other`.
 */
function aggregate(nodes: MockNode[], now: number): void {
  const bytesByCategory = nodes.map(() => new Float64Array(CATEGORIES.length));

  for (const node of nodes) {
    if (node.kind !== 'dir' && node.kind !== 'mount') {
      (bytesByCategory[node.id] as Float64Array)[CATEGORIES.indexOf(node.category)] = node.allocated;
    }

    if (node.kind === 'dir' && node.childCount === 0) {
      node.mtime = now - 30 * DAY;
    } else if (node.kind === 'dir') {
      node.mtime = 0;
    }
  }

  for (let id = nodes.length - 1; id > 0; id--) {
    const node = nodes[id] as MockNode;
    const parent = nodes[node.parent] as MockNode;

    if (node.kind === 'dir') {
      const heaviest = heaviestCategory(bytesByCategory[id] as Float64Array);

      if (heaviest) {
        node.category = heaviest;
      }
    }

    parent.allocated += node.allocated;
    parent.apparent += node.apparent;
    parent.files += node.files;
    parent.sharedFiles += node.sharedFiles;
    parent.mtime = Math.max(parent.mtime, node.mtime);

    const into = bytesByCategory[parent.id] as Float64Array;
    const from = bytesByCategory[id] as Float64Array;
    from.forEach((value, index) => (into[index] = (into[index] ?? 0) + value));
  }

  const root = nodes[0] as MockNode;
  const heaviest = heaviestCategory(bytesByCategory[0] as Float64Array);

  if (heaviest) {
    root.category = heaviest;
  }
}

/** The category with the most bytes, the first of equals, or null when there are none. */
function heaviestCategory(bytes: Float64Array): Category | null {
  let best = 0;

  bytes.forEach((value, index) => {
    if (value > (bytes[best] ?? 0)) {
      best = index;
    }
  });

  return (bytes[best] ?? 0) > 0 ? (CATEGORIES[best] ?? null) : null;
}

export function childrenOf(tree: MockTree, node: MockNode): MockNode[] {
  return tree.nodes.slice(node.firstChild, node.firstChild + node.childCount);
}

export function isDirectory(node: MockNode): boolean {
  return node.kind === 'dir' || node.kind === 'mount';
}
