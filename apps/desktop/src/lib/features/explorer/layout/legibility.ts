import { COLLAPSED_NODE_ID } from '../../../api/client';
import type { NodeDto, NodeId } from '../../../api/types';

/**
 * What is big enough to be drawn. Both views follow these rules and nothing else decides it:
 * a thing below its limit is not drawn on its own, it is merged into the "N smaller items" node of its parent.
 */

/** Flat map: a tile narrower or shorter than this (pixels) is not drawn. */
export const MIN_TILE = 14;
/** Flat map: a folder tile shows a header strip, and the tiles inside it, only from this size. */
export const MIN_HEADER_WIDTH = 60;
export const MIN_HEADER_HEIGHT = 44;
/** Flat map: corner radius of a tile. It shrinks with the tile, so a small tile is never a pill. */
export const MAX_RADIUS = 4;

export function tileRadius(width: number, height: number): number {
  return Math.min(MAX_RADIUS, Math.min(width, height) / 4);
}

/** 3D: the side of a column's footprint, as a share of the block's side, below which it is merged. */
export const MIN_COLUMN_SIDE = 0.03;
/** 3D: the merged column is drawn (and labelled) only when its footprint side is at least this share of the block's. */
export const MIN_MERGED_SIDE = 0.015;
/** 3D: floors per column, the last of them the "rest" floor. */
export const MAX_FLOORS = 12;
/** 3D: a floor thinner than this share of the column's height merges into the rest floor. */
export const MIN_FLOOR_SHARE = 1 / 40;
/** 3D: parts per floor, the last of them the "rest" part. */
export const MAX_PARTS = 8;
/** 3D: a part shorter than this share of its floor's length merges into the rest part. */
export const MIN_PART_SHARE = 1 / 30;
/** 3D: a box thinner than this (scene units, on its smallest side) gets no edge lines. */
export const MIN_EDGE_SIDE = 0.6;
/** 3D: nothing smaller than this on any side is made at all. */
export const MIN_BOX_SIDE = 0.05;
/** 3D: more labels than this are not legible; the biggest columns are labelled, the rest on hover. */
export const MAX_LABELS = 10;

/** How many real items a node stands for: 1, or N for a "N smaller items" node. */
export function itemCount(node: NodeDto): number {
  return node.id === COLLAPSED_NODE_ID ? Number.parseInt(node.name, 10) || 1 : 1;
}

/** Which items each merged node stands for (not those the backend merged before: it does not say which they are). */
const MEMBERS = new WeakMap<NodeDto, readonly NodeId[]>();

export function membersOf(node: NodeDto): readonly NodeId[] {
  return MEMBERS.get(node) ?? [];
}

/** The single "N smaller items" node that replaces `nodes`: their sizes added, their items counted. */
export function mergeNodes(nodes: readonly NodeDto[]): NodeDto {
  const count = nodes.reduce((sum, node) => sum + itemCount(node), 0);
  const merged: NodeDto = {
    id: COLLAPSED_NODE_ID,
    name: count === 1 ? '1 smaller item' : `${count} smaller items`,
    kind: 'collapsed',
    allocated: nodes.reduce((sum, node) => sum + node.allocated, 0),
    apparent: nodes.reduce((sum, node) => sum + node.apparent, 0),
    files: nodes.reduce((sum, node) => sum + node.files, 0),
    hasChildren: false,
    category: 'other',
    mtime: nodes.reduce((newest, node) => Math.max(newest, node.mtime), 0),
  };

  MEMBERS.set(
    merged,
    nodes.flatMap((node) => (node.id === COLLAPSED_NODE_ID ? membersOf(node) : [node.id])),
  );

  return merged;
}
