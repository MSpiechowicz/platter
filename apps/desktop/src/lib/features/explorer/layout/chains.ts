import { COLLAPSED_NODE_ID } from '../../../api/client';
import type { LodNode, NodeDto, NodeId } from '../../../api/types';
import { mergeNodes } from './legibility';

/**
 * Single-child chains ("compact folders"). A folder whose content is essentially one folder (`home` holding only
 * `sam`) is drawn together with it as one tile, so the levels of the map are spent on what is inside the last
 * folder of the chain instead of on the empty ones.
 */

/** A folder counts as one child directory when that directory holds at least this share of it. */
export const CHAIN_SHARE = 0.9;
/** A chain is never longer than this many folders. */
export const MAX_CHAIN_LENGTH = 8;
/** At most this many extra `getLod` requests are made for one view. */
export const MAX_CHAIN_EXPANSIONS = 3;

/** A node of the map's tree: one folder, or a chain of folders drawn as one. */
export interface ChainedLod {
  /** The first folder of the chain: its size, its id (selection) and its name are what the tile stands for. */
  node: NodeDto;
  children: ChainedLod[];
  /** Two or more folders, first to last; absent for a plain folder or file. */
  chain?: readonly NodeDto[];
  /** For a chain: the ids of the last folder's own children. The rest of `children` come from the folders before it. */
  own?: ReadonlySet<NodeId>;
}

/** The folders a tile stands for, first to last. */
export function linksOf(item: { node: NodeDto; chain?: readonly NodeDto[] }): readonly NodeDto[] {
  return item.chain ?? [item.node];
}

/** The folder that opening the tile goes to: the last of its chain. */
export function lastOf(item: { node: NodeDto; chain?: readonly NodeDto[] }): NodeDto {
  return linksOf(item).at(-1) as NodeDto;
}

/** The one child directory that holds at least `CHAIN_SHARE` of `item`, if there is one. */
export function dominantChild(item: LodNode): LodNode | undefined {
  const total = item.node.allocated > 0 ? item.node.allocated : sizeOf(item.children);

  if (total <= 0) {
    return undefined;
  }

  return item.children.find(
    (child) =>
      child.node.id !== COLLAPSED_NODE_ID &&
      child.node.hasChildren &&
      child.node.allocated / total >= CHAIN_SHARE,
  );
}

const sizeOf = (items: readonly LodNode[]): number =>
  items.reduce((sum, item) => sum + item.node.allocated, 0);

/** Joins "smaller items" nodes among `items` into one, so the same id is not there twice. */
function mergeCollapsed<T extends LodNode>(items: readonly T[]): (T | LodNode)[] {
  const collapsed = items.filter((item) => item.node.id === COLLAPSED_NODE_ID);

  if (collapsed.length < 2) {
    return [...items];
  }

  const rest = items.filter((item) => item.node.id !== COLLAPSED_NODE_ID);

  return [...rest, { node: mergeNodes(collapsed.map((item) => item.node)), children: [] }];
}

/** Whether a folder has children that were not loaded (the answer ended above them). */
const unloaded = (item: LodNode): boolean => item.node.hasChildren && item.children.length === 0;

function collapseNode(item: LodNode): ChainedLod {
  const links: NodeDto[] = [item.node];
  const others: LodNode[] = [];

  let last = item;

  while (links.length < MAX_CHAIN_LENGTH) {
    const next = dominantChild(last);

    if (!next) {
      break;
    }

    // What the folders on the way hold besides the next one is not lost: it is shown inside.
    others.push(...last.children.filter((child) => child !== next));
    links.push(next.node);
    last = next;
  }

  if (links.length === 1) {
    return { node: item.node, children: item.children.map(collapseNode) };
  }

  // A chain whose last folder was not read is a plain tile: what is known of it is only a part of what it holds.
  const content = unloaded(last) ? [] : mergeCollapsed([...last.children, ...others]).map(collapseNode);

  return {
    node: item.node,
    children: content,
    chain: links,
    own: new Set(last.children.map((child) => child.node.id)),
  };
}

/**
 * The tree to draw: below the focused folder every chain of single-child folders is one node. The focused folder
 * itself stays as it is, so the page can say what it holds.
 */
export function collapseChains(root: LodNode): ChainedLod {
  return { node: root.node, children: root.children.map(collapseNode) };
}

/** `root` with the subtree of `sub.node` replaced by `sub` (a deeper answer for the same folder). */
export function spliceLod(root: LodNode, sub: LodNode): LodNode {
  if (root.node.id === sub.node.id && root.node.id !== COLLAPSED_NODE_ID) {
    return sub;
  }

  let changed = false;
  const children = root.children.map((child) => {
    const next = spliceLod(child, sub);

    changed ||= next !== child;

    return next;
  });

  return changed ? { node: root.node, children } : root;
}

function findLod(item: LodNode, id: NodeId): LodNode | undefined {
  if (item.node.id === id) {
    return item;
  }

  for (const child of item.children) {
    const found = findLod(child, id);

    if (found) {
      return found;
    }
  }

  return undefined;
}

/**
 * The last folder of the chain `item`, when its content must be read before the chain can be drawn: its children are
 * not known, or (for a chain at the top of the map, whose children are drawn with their own children) are known but
 * none of their own children are.
 */
function missingDepth(item: ChainedLod, lod: LodNode, topLevel: boolean): NodeId | undefined {
  const last = item.chain?.at(-1);
  const real = last ? findLod(lod, last.id) : undefined;

  if (!last || !real || !last.hasChildren) {
    return undefined;
  }

  const shallow =
    topLevel &&
    real.children.some((child) => child.node.id !== COLLAPSED_NODE_ID && child.node.hasChildren) &&
    real.children.every((child) => child.children.length === 0);

  return unloaded(real) || shallow ? last.id : undefined;
}

/** How many levels of the map hold something drawn inside a tile: a chain deeper than this shows no content. */
const LEVELS_WITH_CONTENT = 2;

/**
 * The chains on the map that need a deeper answer, the biggest first: those at the top level, and those one level
 * down (a level further has no room for what is inside). Folders already asked for are left out.
 */
export function chainsToExpand(model: ChainedLod, lod: LodNode, asked: ReadonlySet<NodeId>): NodeId[] {
  const found: { id: NodeId; size: number }[] = [];

  const visit = (item: ChainedLod, level: number): void => {
    const id = missingDepth(item, lod, level === 1);

    if (id !== undefined && !asked.has(id)) {
      found.push({ id, size: item.node.allocated });
    }

    if (level < LEVELS_WITH_CONTENT) {
      item.children.forEach((child) => visit(child, level + 1));
    }
  };

  model.children.forEach((child) => visit(child, 1));

  return found.sort((a, b) => b.size - a.size).map((entry) => entry.id);
}
