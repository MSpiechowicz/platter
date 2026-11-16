import { hierarchy, treemap, type HierarchyRectangularNode } from 'd3-hierarchy';
import { createBandTiling } from './band';
import { COLLAPSED_NODE_ID } from '../../../api/client';
import type { NodeDto, NodeId } from '../../../api/types';
import { linksOf, type ChainedLod } from './chains';
import { itemCount, MIN_HEADER_HEIGHT, MIN_HEADER_WIDTH, MIN_TILE, mergeNodes } from './legibility';
import {
  formatShare,
  minFraction,
  READABLE_HEIGHT,
  READABLE_WIDTH,
  readableWeights,
  splitReadable,
} from './scale';
import { fitTitle, fits, joinNames, measure, type Measurer } from './text';

/** Levels drawn below the focused folder. The request to `getLod` asks for the same depth. */
export const MAP_DEPTH = 3;

/** Tiles are laid out to be a little wider than tall: names and sizes are text, which is read along. */
const TILE_RATIO = 2;
const OUTER_PADDING = 5;
const INNER_PADDING = 3;
/** Room above a folder's children for its name and size. */
const HEADER_HEIGHT = 25;

export interface Tile {
  /** Unique within one layout. */
  key: string;
  node: NodeDto;
  /** 1 for a child of the focused folder, 3 at most. */
  depth: 1 | 2 | 3;
  /** The child of the focused folder this tile belongs to (its own id at depth 1). */
  top: NodeId;
  x0: number;
  y0: number;
  x1: number;
  y1: number;
  /** A folder with children shows a header strip with its name and size. */
  header: boolean;
  /** The folders this tile stands for, first to last, when it is a chain ("home / sam"). */
  chain?: readonly NodeDto[];
  /** For a tile inside a top-level one: the child of the top-level tile it belongs to (its second level). */
  inner?: NodeId;
  /** Depth 1 only: the true share of the focused folder, as a fraction. */
  share: number;
  /** Depth 1 only: the tile is drawn larger than its true share, so that it can be read. */
  boosted: boolean;
  /** What the tile sits in, for a tooltip: the focused folder's name at depth 1, else the parent tile's path. */
  parentName: string;
  /** What the tile truly holds of its parent (the folder it is drawn in), as a fraction, however it is drawn. */
  shareOfParent: number;
}

export interface TileGroup {
  top: NodeId;
  /** The depth 1 tile first, then the nested ones. */
  tiles: Tile[];
}

/** A node of the tree being laid out: what it weighs, and what it holds after small things were merged away. */
interface Work {
  node: NodeDto;
  chain?: readonly NodeDto[];
  /** The size the layout gives it; it stays the same when its children are merged or dropped. */
  size: number;
  /** A top-level tile that was made larger to be read: it is laid out in the band of readable tiles. */
  band?: boolean;
  /** The "N smaller items" tile: it is always laid out after its siblings, however big it is. */
  last?: boolean;
  /** What it really weighs. `size` of a top-level node is this, made larger when it is too small to read. */
  trueSize: number;
  children: Work[];
}

/** More passes than this are never needed: every pass removes something. */
const MAX_PASSES = 40;

function toWork(item: ChainedLod): Work {
  // Items that take no space have no area: they are in the list, not on the map.
  const children = item.children.map(toWork).filter((child) => child.size > 0);
  const size =
    children.length > 0 ? children.reduce((sum, child) => sum + child.size, 0) : item.node.allocated;

  return {
    node: item.node,
    chain: item.chain,
    size,
    trueSize: size,
    children,
    ...(item.node.id === COLLAPSED_NODE_ID ? { last: true } : {}),
  };
}

function scaleWork(item: Work, factor: number): void {
  item.size *= factor;
  item.children.forEach((child) => scaleWork(child, factor));
}

/**
 * The top level of the map: what is too small is folded into one "N smaller items" tile, and what is small but
 * worth a tile is given the room to show its name and size. Everything below stays strictly proportional.
 */
function readable(root: Work, width: number, height: number): void {
  const min = minFraction(width, height);
  const sizes = root.children.map((child) => child.size);
  const { folded } = splitReadable(
    sizes,
    min,
    root.children.findIndex((child) => child.node.id === COLLAPSED_NODE_ID),
  );

  if (folded.length > 0) {
    mergeInto(
      root,
      folded.map((index) => root.children[index] as Work),
    );
  }

  const total = root.children.reduce((sum, child) => sum + child.size, 0);
  const { weights, boosted } = readableWeights(
    root.children.map((child) => child.size),
    min,
  );

  root.children.forEach((child, index) => {
    scaleWork(child, ((weights[index] as number) * total) / child.size);
    child.band = boosted[index] as boolean;
  });
}

type Placed = HierarchyRectangularNode<Work>;

const tiling = createBandTiling<Work>({
  ratio: TILE_RATIO,
  // A top-level tile narrower or shorter than this cannot show a short name and its size on two lines.
  minWidth: 80,
  minHeight: READABLE_HEIGHT - 2,
  tileWidth: READABLE_WIDTH + INNER_PADDING,
  rowHeight: READABLE_HEIGHT + INNER_PADDING,
  // A single small folder is not stretched across the whole map.
  maxTileWidth: READABLE_WIDTH * 2.5,
});

/** Biggest first, except that the "smaller items" tile goes after everything, whatever it weighs. */
function byValueMergedLast(a: Work, aValue: number, b: Work, bValue: number): number {
  return Number(a.last === true) - Number(b.last === true) || bValue - aValue;
}

function place(root: Work, width: number, height: number): Placed {
  const tree = hierarchy(root, (item) => (item.children.length > 0 ? item.children : null))
    .sum((item) => (item.children.length > 0 ? 0 : item.size))
    .sort((a, b) => byValueMergedLast(a.data, a.value ?? 0, b.data, b.value ?? 0));

  return treemap<Work>()
    .tile(tiling)
    .size([width, height])
    .paddingOuter(OUTER_PADDING)
    .paddingTop((item) =>
      item.depth > 0 && item.depth < MAP_DEPTH && item.children ? HEADER_HEIGHT : OUTER_PADDING,
    )
    .paddingInner(INNER_PADDING)
    .round(true)(tree);
}

const tooSmall = (item: Placed): boolean => item.x1 - item.x0 < MIN_TILE || item.y1 - item.y0 < MIN_TILE;
const hasRoomInside = (item: Placed): boolean =>
  !item.data.band && item.x1 - item.x0 >= MIN_HEADER_WIDTH && item.y1 - item.y0 >= MIN_HEADER_HEIGHT;

/** Moves `small` out of `parent` and into its one "smaller items" node (made when there is none yet). */
function mergeInto(parent: Work, small: Work[]): void {
  const gone = new Set(small);
  const rest = parent.children.filter((child) => !gone.has(child));
  const old = rest.find((child) => child.node.id === COLLAPSED_NODE_ID);
  const absorbed = [...(old ? [old] : []), ...small];
  const merged: Work = {
    node: mergeNodes(absorbed.map((child) => child.node)),
    size: absorbed.reduce((sum, child) => sum + child.size, 0),
    trueSize: absorbed.reduce((sum, child) => sum + child.trueSize, 0),
    children: [],
    last: true,
  };

  parent.children = [...rest.filter((child) => child !== old), merged];
}

/** One check of a laid out tree: merges what is too small, empties what is too small to hold anything. */
function tidy(placed: Placed): boolean {
  let changed = false;

  for (const item of placed.descendants()) {
    const work = item.data;
    const small = (item.children ?? []).filter((child) => tooSmall(child));

    if (item.depth > 0 && item.children && !hasRoomInside(item)) {
      work.children = [];
      changed = true;
      continue;
    }

    if (small.length === 0) {
      continue;
    }

    const real = small.filter((child) => child.data.node.id !== COLLAPSED_NODE_ID);

    if (item.depth > 0 && small.length === (item.children ?? []).length) {
      // Nothing in it is big enough: the folder is a plain tile.
      work.children = [];
      changed = true;
    } else if (real.length > 0) {
      // The folder's own "smaller items" node, if it has one, is taken into the merged one.
      mergeInto(
        work,
        real.map((child) => child.data),
      );
      changed = true;
    }

    // A "smaller items" node that is too small on its own stays where it is, so that nothing else moves, and is not
    // drawn (see `layoutTreemap`): its room is the folder's own fill.
  }

  return changed;
}

/**
 * Nested treemap of the focused folder: the sizes are the leaves of the LOD tree, so a folder is as big as
 * what it holds. Tiles come breadth first, so a parent is always drawn before its children.
 * Whatever would come out narrower or shorter than `MIN_TILE` is merged, before drawing, into the "N smaller items"
 * tile of its folder, and the layout is made again, until nothing too small is left. The "smaller items" tile is
 * always the last tile of its folder (the bottom right one). When it is itself narrower or shorter than `MIN_TILE`
 * it is not drawn, and its room stays the folder's own fill: the other tiles keep their places and sizes, so the
 * areas that are drawn still add up to what the folder holds.
 */
export function layoutTreemap(root: ChainedLod, width: number, height: number): Tile[] {
  if (width < 1 || height < 1 || root.children.length === 0) {
    return [];
  }

  const work = toWork(root);

  if (work.size <= 0) {
    return [];
  }

  const trueTotal = work.size;

  readable(work, width, height);

  let placed = place(work, width, height);

  for (let pass = 0; pass < MAX_PASSES && tidy(placed); pass++) {
    if (work.children.length === 0) {
      return [];
    }

    placed = place(work, width, height);
  }

  // What the top-level tiles really take of the map, to tell the ones drawn larger than they are.
  const drawn = (placed.children ?? []).reduce(
    (sum, item) => sum + (item.x1 - item.x0) * (item.y1 - item.y0),
    0,
  );

  return placed
    .descendants()
    .filter((item) => item.depth > 0 && item.depth <= MAP_DEPTH && !tooSmall(item))
    .map((item) => toTile(item, trueTotal, drawn));
}

/** More than this much larger than its true share, and a tile counts as drawn larger than it is. */
const BOOST_TOLERANCE = 1.15;

function toTile(item: Placed, trueTotal: number, drawnTotal: number): Tile {
  const ancestors = item.ancestors().reverse();
  const work = item.data;
  const share = item.depth === 1 && trueTotal > 0 ? work.trueSize / trueTotal : 0;
  const parent = item.parent as Placed;
  const shown = drawnTotal > 0 ? ((item.x1 - item.x0) * (item.y1 - item.y0)) / drawnTotal : 0;

  return {
    key: ancestors.map((link) => link.data.node.id).join('/'),
    node: work.node,
    depth: item.depth as 1 | 2 | 3,
    top: (ancestors[1] as Placed).data.node.id,
    x0: item.x0,
    y0: item.y0,
    x1: item.x1,
    y1: item.y1,
    header: item.depth < MAP_DEPTH && item.children !== undefined && hasRoomInside(item),
    ...(work.chain ? { chain: work.chain } : {}),
    ...(item.depth >= 2 ? { inner: (ancestors[2] as Placed).data.node.id } : {}),
    share,
    boosted: item.depth === 1 && shown > share * BOOST_TOLERANCE,
    parentName: tileTitle(parent.data),
    shareOfParent: parent.data.trueSize > 0 ? work.trueSize / parent.data.trueSize : 0,
  };
}

/** The tiles of each top-level folder together, in the order the folders first appear. */
export function groupByTop(tiles: readonly Tile[]): TileGroup[] {
  const groups = new Map<NodeId, TileGroup>();

  for (const tile of tiles) {
    const group = groups.get(tile.top) ?? { top: tile.top, tiles: [] };

    group.tiles.push(tile);
    groups.set(tile.top, group);
  }

  return [...groups.values()];
}

export const isCollapsed = (node: NodeDto): boolean => node.id === COLLAPSED_NODE_ID;

export interface TileLabel {
  name: string;
  /** The size text, or `''` when it does not fit next to the name. */
  size: string;
  where: 'header' | 'body';
  /** In a header the size goes to the right edge; in a body tile it sits under the name. */
  sizeOnRight: boolean;
}

/** Room left empty at each side of a tile's text, and between a header's name and its size. */
const TEXT_PAD = 8;
const HEADER_GAP = 8;
/** A body tile writes a name over a size from this height and width; a "smaller items" strip one line from 24. */
const TWO_LINES_HEIGHT = 42;
const TWO_LINES_WIDTH = 50;
const ONE_LINE_HEIGHT = 24;

/** The names of the folders a tile stands for, in one text: "home / sam". */
export function tileTitle(tile: Pick<Tile, 'node' | 'chain'>): string {
  return joinNames(linksOf(tile).map((link) => link.name));
}

/** The names that a tile's label is made of: its folder, or the folders of its chain. */
const namesOf = (tile: Pick<Tile, 'node' | 'chain'>): string[] => linksOf(tile).map((link) => link.name);

/**
 * A header strip: the name at the left, the size at the right edge. The size is written only when the name (as it is
 * written whole: the chain, else its first folder) and the size both fit completely with a gap between them; else
 * the name alone gets the whole strip.
 */
function headerLabel(tile: Tile, sizeText: string, measurer: Measurer): TileLabel | null {
  const room = tile.x1 - tile.x0 - 2 * TEXT_PAD;
  const names = namesOf(tile);
  const sizeWidth = sizeText === '' ? 0 : measurer(sizeText, 'size');
  const withSize = [joinNames(names), names[0] ?? ''].find(
    (title) => sizeWidth > 0 && measurer(title, 'name') + HEADER_GAP + sizeWidth <= room,
  );

  if (withSize !== undefined) {
    return { name: withSize, size: sizeText, where: 'header', sizeOnRight: true };
  }

  const name = fitTitle(names, room, 'name', measurer);

  return name ? { name, size: '', where: 'header', sizeOnRight: true } : null;
}

/**
 * The "smaller items" tile: "285 smaller" over the size when both fit, else "+285" on one line, else nothing. The
 * tooltip always says all of it.
 */
function mergedLabel(tile: Tile, sizeText: string, measurer: Measurer): TileLabel | null {
  const width = tile.x1 - tile.x0;
  const height = tile.y1 - tile.y0;
  const room = width - 2 * TEXT_PAD;
  const count = itemCount(tile.node);
  const long = `${count} smaller`;
  const short = `+${count}`;

  if (height <= ONE_LINE_HEIGHT) {
    return null;
  }

  const stacked = height > TWO_LINES_HEIGHT && width > TWO_LINES_WIDTH;

  if (stacked && fits(long, room, 'name', measurer) && fits(sizeText, room, 'size', measurer)) {
    return { name: long, size: sizeText, where: 'body', sizeOnRight: false };
  }

  return fits(short, room, 'name', measurer)
    ? { name: short, size: '', where: 'body', sizeOnRight: false }
    : null;
}

/**
 * What to write on a tile, or `null` when the tile is too small for any text. A name is cut at its end (a chain
 * is written by its first folder when all of it does not fit, see `fitTitle`); a size is written whole or not at all.
 * `measurer` gives the width of a text; the default measures with the browser.
 */
export function tileLabel(tile: Tile, sizeText: string, measurer: Measurer = measure): TileLabel | null {
  const width = tile.x1 - tile.x0;
  const height = tile.y1 - tile.y0;

  if (tile.header) {
    return headerLabel(tile, sizeText, measurer);
  }

  if (isCollapsed(tile.node)) {
    return mergedLabel(tile, sizeText, measurer);
  }

  const name = fitTitle(namesOf(tile), width - 2 * TEXT_PAD, 'name', measurer);

  if (!name || height <= TWO_LINES_HEIGHT || width <= TWO_LINES_WIDTH) {
    return null;
  }

  const size = fits(sizeText, width - 2 * TEXT_PAD, 'size', measurer) ? sizeText : '';

  return { name, size, where: 'body', sizeOnRight: false };
}

/** The tooltip of a "smaller items" tile: what it stands for, how big it is, and how much of its folder that is. */
export function mergedTooltip(tile: Tile, sizeText: string): string {
  return `${tile.node.name} · ${sizeText} · ${formatShare(tile.shareOfParent)} of ${tile.parentName}`;
}
