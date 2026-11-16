import type { MockNode, MockTree } from './mock-tree';

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

export function sketchTree(sketch: Sketch): MockTree {
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

  return { nodes, now: NOW, rootPath: '/' };
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
