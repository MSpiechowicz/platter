import type { MockNode, MockTree } from './mock-tree';
import type { Category } from './types';

// A hand-made tree for tests, written like the backend's `FakeFs`: a list of files with sizes. Directories have no
// size of their own unless `own` says so, ids are breadth-first in the order paths were given, and children are
// contiguous, like in a scan.

interface Entry {
  name: string;
  kind: 'dir' | 'file' | 'mount';
  size: number;
  kids: Map<string, Entry>;
}

export interface Sketch {
  /** Sizes of files by absolute path (`/r/a/x.bin`). */
  files: Record<string, number>;
  /** Directories that hold nothing. */
  dirs?: string[];
  /** Mount points that were not entered. */
  mounts?: string[];
  /** Blocks a directory owns itself. */
  own?: Record<string, number>;
  /** Directories that could not be read. */
  unreadable?: string[];
  /** Modification times (unix seconds) of folders, for the ones that are not from the day before the scan. */
  mtimes?: Record<string, number>;
  /** The category of a folder or a file, for the ones that are not `other`. */
  categories?: Record<string, Category>;
  /** What the machine calls its personal folders where that is not the English name (`{ Downloads: 'Pobrane' }`). */
  userDirs?: Record<string, string>;
  /**
   * The folder that was scanned, when it is not `/`: only what lies below it is in the tree, and every path of the
   * sketch is still written in full (`/home/u/a` with `root: '/home/u'` is `a`).
   */
  root?: string;
}

const NOW = 1_750_000_000;

function entryAt(root: Entry, path: string, kind: Entry['kind']): Entry {
  const parts = path.split('/').filter((part) => part !== '');
  let current = root;

  parts.forEach((name, index) => {
    const last = index === parts.length - 1;
    let next = current.kids.get(name);

    if (!next) {
      next = { name, kind: last ? kind : 'dir', size: 0, kids: new Map() };
      current.kids.set(name, next);
    }

    current = next;
  });

  return current;
}

/** The sketch of a scan that started at `root`: what lies below it, written from there. */
function scannedFrom(sketch: Sketch, root: string): Sketch {
  const relative = (path: string): string | null =>
    path === root ? '/' : path.startsWith(`${root}/`) ? path.slice(root.length) : null;
  const record = <T>(entries: Record<string, T> | undefined): Record<string, T> | undefined =>
    entries &&
    Object.fromEntries(
      Object.entries(entries).flatMap(([path, value]) => {
        const inner = relative(path);

        return inner === null ? [] : [[inner, value]];
      }),
    );
  const list = (paths: string[] | undefined): string[] | undefined =>
    paths?.flatMap((path) => {
      const inner = relative(path);

      return inner === null ? [] : [inner];
    });

  return {
    ...sketch,
    files: record(sketch.files) ?? {},
    dirs: list(sketch.dirs),
    mounts: list(sketch.mounts),
    own: record(sketch.own),
    unreadable: list(sketch.unreadable),
    mtimes: record(sketch.mtimes),
    categories: record(sketch.categories),
  };
}

export function sketchTree(given: Sketch): MockTree {
  const sketch = given.root === undefined || given.root === '/' ? given : scannedFrom(given, given.root);
  const root: Entry = { name: '/', kind: 'dir', size: 0, kids: new Map() };

  for (const [path, size] of Object.entries(sketch.files)) {
    entryAt(root, path, 'file').size = size;
  }

  for (const path of [...(sketch.dirs ?? []), ...(sketch.unreadable ?? [])]) {
    entryAt(root, path, 'dir');
  }

  for (const path of sketch.mounts ?? []) {
    entryAt(root, path, 'mount');
  }

  for (const [path, size] of Object.entries(sketch.own ?? {})) {
    entryAt(root, path, 'dir').size = size;
  }

  const nodes: MockNode[] = [];
  const queue: { entry: Entry; id: number }[] = [];

  const add = (entry: Entry, parent: number): void => {
    const id = nodes.length;

    nodes.push({
      id,
      parent,
      name: entry.name,
      kind: entry.kind,
      allocated: entry.size,
      apparent: entry.size,
      files: entry.kind === 'file' ? 1 : 0,
      category: 'other',
      mtime: NOW - 86_400,
      sharedFiles: 0,
      firstChild: 0,
      childCount: 0,
    });
    queue.push({ entry, id });
  };

  add(root, -1);

  for (let head = 0; head < queue.length; head++) {
    const { entry, id } = queue[head] as { entry: Entry; id: number };
    const node = nodes[id] as MockNode;
    const kids = [...entry.kids.values()];

    node.firstChild = nodes.length;
    node.childCount = kids.length;
    kids.forEach((kid) => add(kid, id));
  }

  for (let id = nodes.length - 1; id > 0; id--) {
    const node = nodes[id] as MockNode;
    const parent = nodes[node.parent] as MockNode;

    parent.allocated += node.allocated;
    parent.apparent += node.apparent;
    parent.files += node.files;
  }

  for (const path of sketch.unreadable ?? []) {
    const node = nodeAt({ nodes, now: NOW, rootPath: '/' }, path);

    if (node) {
      node.unreadable = true;
    }
  }

  for (const [path, mtime] of Object.entries(sketch.mtimes ?? {})) {
    nodeAt({ nodes, now: NOW, rootPath: '/' }, path).mtime = mtime;
  }

  for (const [path, category] of Object.entries(sketch.categories ?? {})) {
    nodeAt({ nodes, now: NOW, rootPath: '/' }, path).category = category;
  }

  return {
    nodes,
    now: NOW,
    rootPath: given.root ?? '/',
    ...(sketch.userDirs ? { userDirs: sketch.userDirs } : {}),
  };
}

/** The value, or a failed test when there is none. */
export function must<T>(value: T | undefined): T {
  if (value === undefined) {
    throw new Error('expected a value');
  }

  return value;
}

export const rootOf = (tree: MockTree): MockNode => must(tree.nodes[0]);

/** The node at an absolute path of a sketch. */
export function nodeAt(tree: MockTree, path: string): MockNode {
  let current = rootOf(tree);

  for (const name of path.split('/').filter((part) => part !== '')) {
    const next = tree.nodes
      .slice(current.firstChild, current.firstChild + current.childCount)
      .find((child) => child.name === name);

    current = must(next);
  }

  return current;
}
