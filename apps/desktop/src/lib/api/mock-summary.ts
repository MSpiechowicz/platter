import { childrenOf, type MockNode, type MockTree } from './mock-tree';

/** Directories listed in `largestDirs`. */
const LARGEST_DIRS = 8;

/** A directory is replaced by its child directories when one of them holds at least this share of its bytes. */
const DOMINANT_SHARE_PERCENT = 80;

/** Candidates are at most this many levels below the summarized node (its children are level 1). */
const MAX_DEPTH = 6;

/** A directory that may be listed, with its depth below the summarized node. */
interface Candidate {
  dir: MockNode;
  depth: number;
}

/** Largest first; on equal size the lower id wins, so the result is deterministic. */
const bySizeThenId = (a: MockNode, b: MockNode): number => b.allocated - a.allocated || a.id - b.id;

/** Directory children of `dir` that have content. Mounts and empty (or unreadable) directories are skipped. */
function childDirs(tree: MockTree, dir: MockNode): MockNode[] {
  return childrenOf(tree, dir).filter((child) => child.kind === 'dir' && child.allocated > 0);
}

function below(tree: MockTree, parent: Candidate): Candidate[] {
  return childDirs(tree, parent.dir).map((dir) => ({ dir, depth: parent.depth + 1 }));
}

/** Whether some child directory of `dir` holds at least `DOMINANT_SHARE_PERCENT` of its bytes. */
function hasDominantChild(tree: MockTree, dir: MockNode): boolean {
  if (dir.allocated === 0) {
    return false;
  }

  return childDirs(tree, dir).some(
    (child) => child.allocated * 100 >= dir.allocated * DOMINANT_SHARE_PERCENT,
  );
}

/** Replaces every dominated candidate with its child directories, repeatedly, as long as the depth limit allows. */
function settleDominated(tree: MockTree, candidates: Candidate[]): Candidate[] {
  let current = candidates;

  for (;;) {
    let changed = false;
    const next: Candidate[] = [];

    for (const candidate of current) {
      if (candidate.depth < MAX_DEPTH && hasDominantChild(tree, candidate.dir)) {
        next.push(...below(tree, candidate));
        changed = true;
      } else {
        next.push(candidate);
      }
    }

    current = next;

    if (!changed) {
      return current;
    }
  }
}

/** The largest candidate whose expansion adds to the list: not at the depth limit, with at least two child directories. */
function largestExpandable(tree: MockTree, candidates: Candidate[]): Candidate | undefined {
  return candidates
    .filter((candidate) => candidate.depth < MAX_DEPTH && childDirs(tree, candidate.dir).length >= 2)
    .sort((a, b) => bySizeThenId(a.dir, b.dir))[0];
}

/**
 * Up to eight directories below `node` for the "largest folders" list, largest first. Same rule as the backend:
 *
 * 1. The candidates start as the non-empty directory children of `node`.
 * 2. A candidate with a child directory that holds at least 80% of its bytes is replaced by its directory
 *    children, again and again, down to 6 levels.
 * 3. While there are fewer than eight candidates, the largest candidate with at least two directory children
 *    (and not at the depth limit) is replaced by its directory children, and step 2 is applied to the result.
 *
 * Empty directories and mount points are never candidates; on equal size the lower id wins.
 */
export function largestDirs(tree: MockTree, node: MockNode): MockNode[] {
  let candidates = settleDominated(
    tree,
    childDirs(tree, node).map((dir) => ({ dir, depth: 1 })),
  );

  while (candidates.length < LARGEST_DIRS) {
    const parent = largestExpandable(tree, candidates);

    if (!parent) {
      break;
    }

    candidates = settleDominated(tree, [
      ...candidates.filter((candidate) => candidate !== parent),
      ...below(tree, parent),
    ]);
  }

  return candidates
    .map((candidate) => candidate.dir)
    .sort(bySizeThenId)
    .slice(0, LARGEST_DIRS);
}
