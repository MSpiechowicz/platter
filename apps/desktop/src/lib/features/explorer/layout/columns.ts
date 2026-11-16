import { hierarchy, treemap, type HierarchyNode, type HierarchyRectangularNode } from 'd3-hierarchy';
import { COLLAPSED_NODE_ID } from '../../../api/client';
import type { NodeDto, NodeId } from '../../../api/types';
import { createBandTiling } from './band';
import { linksOf, type ChainedLod } from './chains';
import {
  itemCount,
  MAX_FLOORS,
  MAX_PARTS,
  MIN_BOX_SIDE,
  MIN_COLUMN_SIDE,
  MIN_EDGE_SIDE,
  MIN_FLOOR_SHARE,
  MIN_MERGED_SIDE,
  MIN_PART_SHARE,
  membersOf,
  mergeNodes,
} from './legibility';
import { readableWeights, splitReadable } from './scale';

/** Side of the square the columns stand on, in scene units. The block spans -42 to 42 on both axes. */
export const FOOTPRINT = 84;
/** Height of every column up to its top floor. A column is as tall as the others, whatever its size. */
export const COLUMN_HEIGHT = 34;
/** Gap between floors, and between the parts of one floor. */
export const GAP = 0.55;
/** Thickness of the lid. */
export const LID_HEIGHT = 1;
/** How high above the column the label's anchor sits. */
export const ANCHOR_RISE = 3;
/** Highest point of the block that must stay on screen (lid included). */
export const BLOCK_TOP = 36;
/** The footprint of a column that is drawn on its own is at least this wide (scene units), so it and its label show. */
export const MIN_FOOTPRINT_SIDE = 8;
/** More columns than this cannot be labelled legibly: the smallest are merged into one "smaller items" column. */
export const MAX_COLUMNS = 16;

/** 1 for the whole column, 2 for a floor, 3 for a part of a floor. The lid is level 3. */
export type Level = 1 | 2 | 3;

export interface Box {
  x0: number;
  x1: number;
  y0: number;
  y1: number;
  z0: number;
  z1: number;
  level: Level;
  lid: boolean;
  /** Edge lines are drawn only on a box that is thick enough to show them; on a thin one they would be a haze. */
  edges: boolean;
  /** In a framed (neutral) column: the floor this box belongs to, which has a colour of its own. */
  hue?: NodeId;
  /** A floor or part that stands for "N smaller items": it has its parent's hue, one step quieter. */
  rest: boolean;
}

export interface Column {
  top: NodeId;
  node: NodeDto;
  /** The folders the column stands for, first to last, when it is a chain ("home / sam"). */
  chain?: readonly NodeDto[];
  /** What is written on it: the folder's name, or the chain joined with " / ". */
  label: string;
  /** The true share of the folder, as a fraction. */
  share: number;
  /** The footprint is larger than the true share, so that the column can be seen and read. */
  boosted: boolean;
  x0: number;
  x1: number;
  z0: number;
  z1: number;
  boxes: Box[];
  /** For the "smaller items" column: the items merged into it. */
  members: readonly NodeId[];
  /** Where the label's dot is attached, above the middle of the lid. */
  anchor: { x: number; y: number; z: number };
}

export interface ColumnsLayout {
  columns: Column[];
  /** How many items of the folder have no column of their own and are not drawn at all (see the list). */
  omitted: number;
  /** At least one column is drawn larger than its true share. */
  boosted: boolean;
}

/** Merges everything past the biggest `max - 1` children, and the existing smaller-items node, into one node. */
export function capTopLevel(root: ChainedLod, max: number = MAX_COLUMNS): ChainedLod {
  if (root.children.length <= max) {
    return root;
  }

  const real = root.children.filter((child) => child.node.id !== COLLAPSED_NODE_ID);
  const old = root.children.find((child) => child.node.id === COLLAPSED_NODE_ID);
  const ranked = [...real].sort((a, b) => b.node.allocated - a.node.allocated);
  const kept = ranked.slice(0, max - 1);
  const hidden = ranked.slice(max - 1).map((child) => child.node);

  if (old) {
    hidden.push(old.node);
  }

  return { node: root.node, children: [...kept, { node: mergeNodes(hidden), children: [] }] };
}

/** A node of the tree being laid out, with the factor that makes a small column large enough to read. */
interface Item {
  node: NodeDto;
  source: ChainedLod;
  chain?: readonly NodeDto[];
  factor: number;
  /** A column made larger to be read: it stands in the band of small columns along the front of the block. */
  band?: boolean;
  /** The "smaller items" node: always laid out after its siblings. */
  last?: boolean;
  children: Item[];
}

function toItem(lod: ChainedLod, factor: number, band = false): Item {
  return {
    node: lod.node,
    source: lod,
    chain: lod.chain,
    factor,
    band,
    last: lod.node.id === COLLAPSED_NODE_ID,
    children: lod.children.map((child) => toItem(child, factor)),
  };
}

/** The small columns stand in a row along the front, each at least this wide and deep, and at most 30 wide. */
const tiling = createBandTiling<Item>({
  ratio: 1.6,
  minWidth: MIN_FOOTPRINT_SIDE + 3,
  minHeight: MIN_FOOTPRINT_SIDE + 3,
  tileWidth: MIN_FOOTPRINT_SIDE + 6,
  rowHeight: MIN_FOOTPRINT_SIDE + 3,
  maxTileWidth: 30,
});

type Node = HierarchyNode<Item>;

const value = (item: Node): number => item.value ?? 0;
const finite = (...numbers: number[]): boolean => numbers.every(Number.isFinite);

function box(
  x0: number,
  x1: number,
  y0: number,
  y1: number,
  z0: number,
  z1: number,
  level: Level,
  lid = false,
): Box {
  const smallest = Math.min(x1 - x0, y1 - y0, z1 - z0);

  return { x0, x1, y0, y1, z0, z1, level, lid, edges: smallest >= MIN_EDGE_SIDE, rest: false };
}

/** A floor or a part: a child of the node above it, or the leftovers of several children merged into one. */
interface Piece {
  value: number;
  /** What it is split into one level down; none for a merged one. */
  children: readonly Node[] | undefined;
  /** The node it is, or `null` for a merged one. */
  origin: Node | null;
}

const whole = (item: Node): Piece => ({ value: value(item), children: item.children, origin: item });

/**
 * The children that get a floor (or a part) of their own, and the rest merged into one that goes last. A child
 * is kept when it is at least `minShare` of the whole; at most `max` pieces come out, the merged one included.
 * Leftovers that add up to less than `minShare` are not drawn at all.
 */
function pieces(children: readonly Node[], total: number, minShare: number, max: number): Piece[] {
  if (total <= 0 || max < 1) {
    return [];
  }

  // The "smaller items" node of the backend is one more leftover: it joins the rest, which goes last.
  const big = children.filter(
    (child) => child.data.node.id !== COLLAPSED_NODE_ID && value(child) / total >= minShare,
  );
  const tooMany = big.length > max || (big.length === max && big.length < children.length);
  const kept = tooMany ? big.slice(0, max - 1) : big;
  const left = children.filter((child) => !kept.includes(child));
  const result = kept.map(whole);
  const dust = left.reduce((sum, child) => sum + value(child), 0);

  if (left.length > 0 && (dust / total >= minShare || kept.length === 0)) {
    result.push({ value: dust, children: undefined, origin: null });
  }

  return result;
}

function buildColumn(
  top: HierarchyRectangularNode<Item>,
  trueTotal: number,
  total: number,
  frame: NodeId | null,
): Column | null {
  const half = FOOTPRINT / 2;
  const x0 = top.x0 - half;
  const x1 = top.x1 - half;
  const z0 = top.y0 - half;
  const z1 = top.y1 - half;

  if (!finite(x0, x1, z0, z1) || x1 - x0 < MIN_BOX_SIDE || z1 - z0 < MIN_BOX_SIDE || value(top) <= 0) {
    return null;
  }

  const floors: Piece[] = top.children
    ? pieces(top.children, value(top), MIN_FLOOR_SHARE, MAX_FLOORS)
    : [whole(top)];
  const floorSum = floors.reduce((sum, floor) => sum + floor.value, 0);
  const floorSpace = COLUMN_HEIGHT - GAP * (floors.length - 1);
  const alongX = x1 - x0 >= z1 - z0;
  const span = alongX ? x1 - x0 : z1 - z0;
  const boxes: Box[] = [];
  const framed = frame === top.data.node.id;

  let y = 0;

  for (const floor of floors) {
    // The rest floor of a framed column has no colour of its own: it is the column's, one step quieter.
    const hue = framed && floor.origin ? floor.origin.data.node.id : undefined;
    const height = (floor.value / floorSum) * floorSpace;
    // However narrow the column, every part keeps a length it can be seen at.
    const room = Math.max(1, Math.floor((span + GAP) / (MIN_EDGE_SIDE + GAP)));
    const parts: Piece[] = floor.children
      ? pieces(floor.children, floor.value, MIN_PART_SHARE, Math.min(MAX_PARTS, room))
      : [floor];
    const partSum = parts.reduce((sum, part) => sum + part.value, 0);
    const length = span - GAP * (parts.length - 1);

    let offset = alongX ? x0 : z0;

    for (const part of parts) {
      const size = (part.value / partSum) * length;
      const level: Level = part.origin === top ? 1 : part === floor ? 2 : 3;
      const made = alongX
        ? box(offset, offset + size, y, y + height, z0, z1, level)
        : box(x0, x1, y, y + height, offset, offset + size, level);

      if (hue !== undefined) {
        made.hue = hue;
      }

      made.rest = part.origin === null;

      offset += size + GAP;

      // Nothing is made that cannot be seen, or that has no sensible size.
      if (
        finite(made.x0, made.x1, made.y0, made.y1, made.z0, made.z1) &&
        Math.min(made.x1 - made.x0, made.y1 - made.y0, made.z1 - made.z0) >= MIN_BOX_SIDE
      ) {
        boxes.push(made);
      }
    }

    y += height + GAP;
  }

  // The lid: one slab over the whole column, in the column's own colour.
  boxes.push(box(x0, x1, y, y + LID_HEIGHT, z0, z1, 3, true));

  const share = trueTotal > 0 ? top.data.node.allocated / trueTotal : 0;
  const shown = total > 0 ? value(top) / total : 0;

  return {
    top: top.data.node.id,
    node: top.data.node,
    ...(top.data.chain ? { chain: top.data.chain } : {}),
    label: linksOf(top.data)
      .map((link) => link.name)
      .join(' / '),
    share,
    boosted: shown > share * 1.01,
    x0,
    x1,
    z0,
    z1,
    boxes,
    members: membersOf(top.data.node),
    anchor: { x: (x0 + x1) / 2, y: COLUMN_HEIGHT + ANCHOR_RISE, z: (z0 + z1) / 2 },
  };
}

const MAX_PASSES = 20;

function place(children: readonly Item[], root: NodeDto): HierarchyRectangularNode<Item> | null {
  const top: Item = { node: root, source: { node: root, children: [] }, factor: 1, children: [...children] };
  const tree = hierarchy(top, (item) => (item.children.length > 0 ? item.children : null))
    .sum((item) => (item.children.length > 0 ? 0 : item.node.allocated * item.factor))
    .sort((a, b) => Number(a.data.last === true) - Number(b.data.last === true) || value(b) - value(a));

  if (value(tree) <= 0) {
    return null;
  }

  return treemap<Item>().tile(tiling).size([FOOTPRINT, FOOTPRINT]).paddingInner(3).round(false)(tree);
}

const side = (item: HierarchyRectangularNode<Item>): number => Math.min(item.x1 - item.x0, item.y1 - item.y0);

/** Folds the small columns into one and works out how much larger each must be drawn to be readable. */
function readable(children: readonly ChainedLod[]): {
  children: ChainedLod[];
  factors: Map<ChainedLod, number>;
  bands: Set<ChainedLod>;
} {
  const min = ((MIN_FOOTPRINT_SIDE + 3) / FOOTPRINT) ** 2;
  const split = splitReadable(
    children.map((child) => child.node.allocated),
    min,
    children.findIndex((child) => child.node.id === COLLAPSED_NODE_ID),
  );
  let kept = split.kept.map((index) => children[index] as ChainedLod);

  if (split.folded.length > 0) {
    const old = kept.find((child) => child.node.id === COLLAPSED_NODE_ID);
    const absorbed = [...split.folded.map((index) => children[index] as ChainedLod), ...(old ? [old] : [])];

    kept = [
      ...kept.filter((child) => child !== old),
      { node: mergeNodes(absorbed.map((child) => child.node)), children: [] },
    ];
  }

  const total = kept.reduce((sum, child) => sum + child.node.allocated, 0);
  const { weights, boosted } = readableWeights(
    kept.map((child) => child.node.allocated),
    min,
  );
  const factors = new Map<ChainedLod, number>();
  const bands = new Set<ChainedLod>();

  kept.forEach((child, index) => {
    const size = child.node.allocated;

    factors.set(child, size > 0 ? ((weights[index] as number) * total) / size : 1);

    if (boosted[index]) {
      bands.add(child);
    }
  });

  return { children: kept, factors, bands };
}

/**
 * The focused folder as a block of columns: each child is a column of the same height, its floors are its children
 * (as tall as their share), and each floor is split into parts by the grandchildren. Volume stands for size.
 * The small columns are folded into one "smaller items" column and the biggest few of them are made wide enough to
 * show with their label (the true share is on the column); the rest are strictly proportional.
 * A column whose footprint is still too thin to see is merged, with the other thin ones, into the "smaller items"
 * column, which is drawn only if it is itself visible; the layout is made again until nothing thin is left.
 * `frame` is the column that is a neutral frame: its floors get a colour of their own.
 */
export function layoutColumns(
  root: ChainedLod,
  max: number = MAX_COLUMNS,
  frame: NodeId | null = null,
): ColumnsLayout {
  // Items that take no space have no column: they are in the list.
  const capped = capTopLevel(
    { node: root.node, children: root.children.filter((child) => child.node.allocated > 0) },
    max,
  ).children;
  const trueTotal = capped.reduce((sum, child) => sum + child.node.allocated, 0);
  const prepared = readable(capped);
  let children = prepared.children;
  let omitted = 0;

  for (let pass = 0; pass < MAX_PASSES; pass++) {
    const laidOut = place(
      children.map((child) => toItem(child, prepared.factors.get(child) ?? 1, prepared.bands.has(child))),
      root.node,
    );

    if (!laidOut) {
      return { columns: [], omitted: 0, boosted: false };
    }

    const placed = laidOut.children ?? [];
    const thin = placed.filter(
      (item) => item.data.node.id !== COLLAPSED_NODE_ID && side(item) < FOOTPRINT * MIN_COLUMN_SIDE,
    );
    const merged = placed.find((item) => item.data.node.id === COLLAPSED_NODE_ID);

    if (thin.length > 0) {
      const absorbed = [...thin.map((item) => item.data.source), ...(merged ? [merged.data.source] : [])];

      children = [
        ...children.filter((child) => !absorbed.includes(child)),
        { node: mergeNodes(absorbed.map((child) => child.node)), children: [] },
      ];
      continue;
    }

    if (merged && side(merged) < FOOTPRINT * MIN_MERGED_SIDE) {
      omitted += itemCount(merged.data.node);
      children = children.filter((child) => child !== merged.data.source);
      continue;
    }

    const total = value(laidOut);
    const columns = placed
      .map((item) => buildColumn(item, trueTotal, total, frame))
      .filter((column): column is Column => column !== null);

    return { columns, omitted, boosted: columns.some((column) => column.boosted) };
  }

  return { columns: [], omitted, boosted: false };
}
