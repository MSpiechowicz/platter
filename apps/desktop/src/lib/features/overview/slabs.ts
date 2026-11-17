import type { NodeDto, NodeId } from '../../api/types';
import { childColors } from '../../folderColor';
import { SLATE } from '../../palette';
import { platform } from '../../state/platform.svelte';

/** One layer of the "What is on the disk" stack. */
export interface Slab {
  key: string;
  /** Path from the scan root: "/home", "/home/sam/Videos"; "other" for the merged rest, "free space". */
  name: string;
  size: number;
  /** Share of the whole, 0 to 1. */
  share: number;
  color: string;
  /** What is inside, in a few names. */
  inside: string;
  /** The node it opens in the Explorer. Merged and free slabs have none. */
  nodeId?: NodeId;
  free?: boolean;
}

/** The children of one folder that the stack looks into. */
export interface SlabLevel {
  /** Path of that folder from the scan root: "" for the root itself, "/home" below it. */
  path: string;
  /** The folder itself; the root has none. */
  parent?: NodeId;
  children: NodeDto[];
}

export interface SlabInput {
  /**
   * `levels[0]` holds the children of the scan root, and `levels[n + 1]` the children of the dominant child of
   * `levels[n]`. A dominant child is replaced by its own children, so it is not a slab itself.
   */
  levels: SlabLevel[];
  /** Names of the biggest things inside a child. */
  insideOf: (id: NodeId) => string[];
  /** What 100% is: the volume size, or the scanned size when there is no volume. */
  total: number;
  /** Free space of the volume; adds a dashed "free space" slab. */
  free?: number;
}

/** Children below this share of the whole are merged into "other". */
export const MIN_SLAB_SHARE = 0.05;
export const MAX_SLABS = 6;
/** A single child holding more than this share of its folder is broken down into its own children. */
export const DOMINANT_SHARE = 0.6;
/** How many folders deep the breakdown goes. */
export const MAX_BREAKDOWN = 3;
const NAMES_SHOWN = 3;

/** A child that holds more than `DOMINANT_SHARE` of `parentSize` and is a folder with something in it. */
export function dominantChild(children: readonly NodeDto[], parentSize: number): NodeDto | undefined {
  if (parentSize <= 0) {
    return undefined;
  }

  return children.find(
    (child) => child.kind === 'dir' && child.hasChildren && child.allocated / parentSize > DOMINANT_SHARE,
  );
}

/** Whether a child is big enough, against the whole, to be a slab of its own. */
export function isSlabSized(child: NodeDto, total: number): boolean {
  return total > 0 && child.allocated / total >= MIN_SLAB_SHARE;
}

interface Candidate {
  node: NodeDto;
  name: string;
}

/**
 * The colour of every folder in `levels`, by the one rule for folder colours (see `folderColor.ts`): the children
 * of the scan root take the palette in order, and below a folder its largest child keeps its colour. A level's
 * parent is the dominant child of the level above, so its colour is already known when the level is reached.
 */
export function levelColors(levels: readonly SlabLevel[]): Map<NodeId, string> {
  const colors = new Map<NodeId, string>();

  for (const level of levels) {
    const parentColor = level.parent === undefined ? null : (colors.get(level.parent) ?? SLATE);

    for (const [id, color] of childColors(parentColor, level.children)) {
      colors.set(id, color);
    }
  }

  return colors;
}

function candidatesOf(levels: SlabLevel[]): Candidate[] {
  return levels.flatMap((level, depth) => {
    const next = levels[depth + 1];

    return level.children
      .filter((child) => child.allocated > 0 && child.id !== next?.parent)
      .map((node) => ({
        node,
        // Files in the root keep their plain name; everything else is shown by its path.
        name:
          node.kind === 'file' && level.path === ''
            ? node.name
            : `${level.path}${platform.separator}${node.name}`,
      }));
  });
}

export function joinNames(names: string[], limit: number): string {
  const shown = names.slice(0, limit).join(' · ');

  return names.length > limit ? `${shown} · …` : shown;
}

/**
 * The biggest children become slabs, each in the colour of its folder (`levelColors`), the colour it has in every
 * other view. The small ones are merged into "other", and free space closes the stack.
 */
export function buildSlabs({ levels, insideOf, total, free }: SlabInput): Slab[] {
  const sorted = candidatesOf(levels).sort((a, b) => b.node.allocated - a.node.allocated);
  const isBig = (entry: Candidate, index: number): boolean =>
    index < MAX_SLABS && isSlabSized(entry.node, total);

  const big = sorted.filter(isBig);
  const small = sorted.filter((entry, index) => !isBig(entry, index));
  const share = (size: number): number => (total > 0 ? size / total : 0);
  const colors = levelColors(levels);

  const slabs: Slab[] = big.map(({ node, name }) => ({
    key: `node-${node.id}`,
    name,
    size: node.allocated,
    share: share(node.allocated),
    color: colors.get(node.id) ?? SLATE,
    inside: joinNames(insideOf(node.id), NAMES_SHOWN),
    nodeId: node.id,
  }));

  if (small.length > 0) {
    const size = small.reduce((sum, entry) => sum + entry.node.allocated, 0);

    slabs.push({
      key: 'other',
      name: 'other',
      size,
      share: share(size),
      color: SLATE,
      inside: joinNames(
        small.map((entry) => entry.node.name),
        NAMES_SHOWN + 1,
      ),
    });
  }

  if (free !== undefined && free > 0) {
    slabs.push({
      key: 'free',
      name: 'free space',
      size: free,
      share: share(free),
      color: 'var(--muted)',
      inside: 'available',
      free: true,
    });
  }

  return slabs;
}
