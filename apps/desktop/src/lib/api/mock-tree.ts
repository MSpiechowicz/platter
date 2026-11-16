import { layoutFor, type DirSpec, type FileGroup } from './mock-layouts';
import type { Category, NodeKind, SkipReason } from './types';

export const BLOCK_SIZE = 4096;
export const DAY = 86_400;

/** In the backend's index order, which breaks ties between categories of equal size. */
export const CATEGORIES: Category[] = [
  'video',
  'image',
  'audio',
  'document',
  'archive',
  'code',
  'build',
  'package',
  'app',
  'diskImage',
  'data',
  'other',
];

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

const EXTENSIONS: Record<Category, string[]> = {
  video: ['mkv', 'mp4', 'webm'],
  image: ['jpg', 'png', 'webp', 'heic'],
  audio: ['flac', 'mp3', 'ogg'],
  document: ['pdf', 'odt', 'md', 'txt'],
  archive: ['zip', 'tar.zst', '7z', 'tar.gz'],
  code: ['rs', 'ts', 'svelte', 'py', 'toml'],
  build: ['o', 'rlib', 'rmeta', 'd'],
  package: ['js', 'mjs', 'json', 'd.ts'],
  app: ['so', 'pak', 'bin', 'dll'],
  diskImage: ['iso', 'qcow2', 'img'],
  data: ['sqlite', 'parquet', 'gguf', 'jsonl'],
  other: ['dat', 'bin', 'cache', 'log'],
};

function fileName(random: Random, category: Category, index: number): string {
  const word = pick(random, WORDS);
  const ext = pick(random, EXTENSIONS[category]);

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
  /** A file that is also hard-linked from somewhere else. */
  shared?: boolean;
  kids: Draft[];
}

interface Context {
  random: Random;
  now: number;
  oneFileSystem: boolean;
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

function weightOf(spec: DirSpec, ctx: Context): number {
  return isSkipped(spec, ctx) ? 0 : spec.weight;
}

function buildDir(spec: DirSpec, bytes: number, ctx: Context): Draft {
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
  const total = kids.reduce((sum, kid) => sum + weightOf(kid, ctx), 0) + filesWeight;

  for (const kid of kids) {
    const weight = weightOf(kid, ctx);

    // A mount point that is not entered still shows up, as an empty `mount` node.
    if (weight > 0 || kid.mount || kid.unreadable || kid.empty) {
      draft.kids.push(buildDir(kid, (bytes * weight) / total, ctx));
    }
  }

  if (spec.files) {
    addFiles(draft, spec.files, (bytes * filesWeight) / total, ctx);
  }

  for (const name of spec.markers ?? []) {
    draft.kids.push(markerFile(name, ctx.now));
  }

  return draft;
}

/** A small file such as `Cargo.toml`. It has no weight and does not touch the random numbers, so the rest of the tree stays as it is. */
function markerFile(name: string, now: number): Draft {
  return {
    name,
    kind: 'file',
    category: name.endsWith('.toml') ? 'code' : 'data',
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

    target.kids.push({
      name: fileName(random, category, index),
      kind: 'file',
      category,
      apparent,
      allocated: Math.ceil(apparent / BLOCK_SIZE) * BLOCK_SIZE,
      mtime,
      ...(group.sharedEvery && index % group.sharedEvery === 0 ? { shared: true } : {}),
      kids: [],
    });
  });
}

/** What the real backend calls the root node: the last path component, or `/` for the filesystem root. */
export function rootName(path: string): string {
  const trimmed = path.replace(/\/+$/, '');

  return trimmed.slice(trimmed.lastIndexOf('/') + 1) || '/';
}

/** Builds a realistic demo tree: ids in breadth-first order, parents before children, children contiguous. */
export function buildTree(options: TreeOptions): MockTree {
  const ctx: Context = {
    random: mulberry32(options.seed),
    now: options.now,
    oneFileSystem: options.oneFileSystem,
  };
  const rootSpec: DirSpec = {
    name: rootName(options.root),
    weight: 1,
    kids: layoutFor(options.root, options.dominant),
  };
  const rootDraft = buildDir(rootSpec, options.bytes, ctx);

  return flatten(rootDraft, options);
}

function flatten(rootDraft: Draft, options: TreeOptions): MockTree {
  const nodes: MockNode[] = [];
  const queue: { draft: Draft; id: number }[] = [];

  const add = (draft: Draft, parent: number): number => {
    const id = nodes.length;

    nodes.push({
      id,
      parent,
      name: draft.name,
      kind: draft.kind,
      allocated: draft.allocated,
      apparent: draft.apparent,
      files: draft.kind === 'dir' || draft.kind === 'mount' ? 0 : 1,
      category: draft.category,
      mtime: draft.mtime,
      ...(draft.unreadable ? { unreadable: true } : {}),
      ...(draft.skipped ? { skipped: draft.skipped } : {}),
      sharedFiles: draft.shared ? 1 : 0,
      firstChild: 0,
      childCount: 0,
    });
    queue.push({ draft, id });

    return id;
  };

  add(rootDraft, -1);

  for (let head = 0; head < queue.length; head++) {
    const entry = queue[head];

    if (!entry) {
      break;
    }

    const node = nodes[entry.id] as MockNode;
    node.firstChild = nodes.length;
    node.childCount = entry.draft.kids.length;
    entry.draft.kids.forEach((kid) => add(kid, node.id));
  }

  aggregate(nodes, options.now);

  return { nodes, now: options.now, rootPath: options.root };
}

/** One reverse pass: every child is folded into its parent. */
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

    parent.allocated += node.allocated;
    parent.apparent += node.apparent;
    parent.files += node.files;
    parent.sharedFiles += node.sharedFiles;
    parent.mtime = Math.max(parent.mtime, node.mtime);

    const into = bytesByCategory[parent.id] as Float64Array;
    const from = bytesByCategory[id] as Float64Array;
    from.forEach((value, index) => (into[index] = (into[index] ?? 0) + value));
  }

  for (const node of nodes) {
    if (node.kind !== 'dir') {
      continue;
    }

    const bytes = bytesByCategory[node.id] as Float64Array;
    let best = bytes.length - 1;

    bytes.forEach((value, index) => {
      if (value > (bytes[best] ?? 0)) {
        best = index;
      }
    });
    node.category = CATEGORIES[best] ?? 'other';
  }
}

export function childrenOf(tree: MockTree, node: MockNode): MockNode[] {
  return tree.nodes.slice(node.firstChild, node.firstChild + node.childCount);
}

export function isDirectory(node: MockNode): boolean {
  return node.kind === 'dir' || node.kind === 'mount';
}
