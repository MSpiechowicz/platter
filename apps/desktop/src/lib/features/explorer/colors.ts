import { COLLAPSED_NODE_ID } from '../../api/client';
import type { NodeId } from '../../api/types';
import { childColors } from '../../folderColor';
import { SLATE } from '../../palette';
import { theme } from '../../state/theme.svelte';
import type { ChainedLod } from './layout/chains';
import type { Tile } from './layout/treemap';

export { PALETTE, SLATE } from '../../palette';

/** One top-level tile holding more than this share of the map shows its own children in colours of their own. */
export const DOMINANT_MAP_SHARE = 0.6;

/** The dark theme's page background, `--bg` in app.css, and what `pageBackground` answers where the page has none. */
export const BACKGROUND = '#0a0e17';

/**
 * The page background of the theme on screen, as `#rrggbb`. Deeper levels are mixed towards it. Read inside an effect
 * or a derived value, it makes that run again when the theme changes.
 */
export function pageBackground(): string {
  const background = theme.token('--bg');

  return /^#[0-9a-f]{6}$/i.test(background) ? background : BACKGROUND;
}

/** Share of the folder colour (rest is background) for a flat tile at depth 0 to 3. */
export const FLAT_MIX: readonly number[] = [0, 22, 38, 56];

/** The same for a 3D column: top level, floor, part, and the lid. */
export const COLUMN_MIX: readonly number[] = [0, 34, 52, 72];

/** The same on the light theme: a tint over a light page needs more of the colour to stand out the way a dark one does. */
export const FLAT_MIX_LIGHT: readonly number[] = [0, 32, 50, 68];
export const COLUMN_MIX_LIGHT: readonly number[] = [0, 44, 62, 80];

/** The flat map's mix for the theme on screen. Read inside an effect or a derived value, it follows the theme. */
export function flatMix(): readonly number[] {
  return theme.effective === 'light' ? FLAT_MIX_LIGHT : FLAT_MIX;
}

/** The 3D columns' mix for the theme on screen. */
export function columnMix(): readonly number[] {
  return theme.effective === 'light' ? COLUMN_MIX_LIGHT : COLUMN_MIX;
}

/**
 * How much quieter (percentage points of the folder colour, towards the background) the "smaller items" tile of a
 * coloured folder is than the tiles beside it. It has its parent's hue, never slate.
 */
export const MERGED_STEP = 12;

export type ColorMap = ReadonlyMap<NodeId, string>;

function channels(hex: string): [number, number, number] {
  const value = Number.parseInt(hex.slice(1), 16);

  return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
}

const toHex = (value: number): string => Math.round(value).toString(16).padStart(2, '0');

/**
 * `percent` of `color` over the background (the page's, or the one given), as `#rrggbb`. Same result as CSS
 * `color-mix(in srgb, color p%, bg)`.
 */
export function blend(color: string, percent: number, background: string = pageBackground()): string {
  const share = Math.max(0, Math.min(100, percent)) / 100;
  const from = channels(background);
  const to = channels(color);

  return `#${to.map((channel, index) => toHex((from[index] as number) * (1 - share) + channel * share)).join('')}`;
}

export function colorOf(colors: ColorMap, id: NodeId): string {
  return colors.get(id) ?? SLATE;
}

/** The top-level tile that holds more than `DOMINANT_MAP_SHARE` of the map and has folders inside, if there is one. */
export function dominantTile(children: readonly ChainedLod[]): ChainedLod | undefined {
  const total = children.reduce((sum, child) => sum + child.node.allocated, 0);

  return children.find(
    (child) =>
      total > 0 &&
      child.node.allocated / total > DOMINANT_MAP_SHARE &&
      child.children.some((inner) => inner.node.id !== COLLAPSED_NODE_ID && inner.node.hasChildren),
  );
}

/** The children of a tile that are the real children of the folder it opens (a chain tile also holds some others). */
export function ownChildren(tile: ChainedLod): ChainedLod[] {
  const own = tile.own;

  return own ? tile.children.filter((child) => own.has(child.node.id)) : tile.children;
}

const sized = (items: readonly ChainedLod[]): { id: NodeId; allocated: number }[] =>
  items
    .filter((item) => item.node.id !== COLLAPSED_NODE_ID)
    .map((item) => ({ id: item.node.id, allocated: item.node.allocated }));

/**
 * The colour of every folder on the map, by one rule that every view shares (`childColors`): the largest child
 * inherits the colour of its parent, the others take the palette in order. The top level is coloured from the
 * focused folder's colour (`null` at the scan root, which has none). When one top-level tile holds most of the
 * map the same rule is applied one level down, with the tile's colour as the parent's: the folders inside it get the
 * colours they will have once it is opened. The "smaller items" node is slate.
 */
export function assignMapColors(focusColor: string | null, children: readonly ChainedLod[]): ColorMap {
  const colors = new Map<NodeId, string>(childColors(focusColor, sized(children)));
  const dominant = dominantTile(children);

  if (dominant) {
    const inside = childColors(colors.get(dominant.node.id) ?? null, sized(ownChildren(dominant)));

    inside.forEach((color, id) => colors.set(id, color));
  }

  if (children.some((child) => child.node.id === COLLAPSED_NODE_ID)) {
    colors.set(COLLAPSED_NODE_ID, SLATE);
  }

  return colors;
}

export interface TileStyle {
  fill: string;
  stroke: string;
  /** The "smaller items" tile: drawn with a dashed outline. */
  merged: boolean;
}

/**
 * The colour a flat tile is drawn in. A tile has its top-level folder's colour; inside a dominant tile the folders
 * have colours of their own. The "smaller items" tile of the focused folder (depth 1) mixes folders of different
 * colours and is slate; one inside a folder has that folder's colour, as its siblings do.
 */
export function tileColor(tile: Pick<Tile, 'node' | 'depth' | 'top' | 'inner'>, colors: ColorMap): string {
  const merged = tile.node.id === COLLAPSED_NODE_ID;

  if (merged && tile.depth === 1) {
    return SLATE;
  }

  // The merged tile at the second level has no folder of its own: its parent is the top-level tile.
  const inner =
    merged && tile.depth === 2 ? undefined : tile.inner === undefined ? undefined : colors.get(tile.inner);

  return inner ?? colorOf(colors, tile.top);
}

/** Fill and outline of a flat tile; a merged tile inside a folder is `MERGED_STEP` quieter than its siblings. */
export function tileStyle(tile: Pick<Tile, 'node' | 'depth' | 'top' | 'inner'>, colors: ColorMap): TileStyle {
  const color = tileColor(tile, colors);
  const merged = tile.node.id === COLLAPSED_NODE_ID;
  const mix = flatMix()[tile.depth] as number;

  return {
    fill: blend(color, merged && tile.depth > 1 ? mix - MERGED_STEP : mix),
    stroke: blend(color, 80),
    merged,
  };
}
