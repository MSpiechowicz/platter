import type { Verdict } from '../../api/types';

/*
 * The layout of the Cleanup view's flow chart, ported from `drawFlow` of docs/mockups/c-orb.html and kept free of
 * the document: numbers in, numbers out. Every group is a bar on the left; a ribbon runs from it to "Will be
 * freed" or "Stays on disk" on the right. A ribbon is as thick as its bar, and a bar as tall as its share of the
 * total, except that a thin group gets a minimum height, so that it can be read and clicked.
 *
 * The bars on the left, with their gaps, fill the box from the top to the bottom padding exactly: nothing is left
 * empty below them.
 */

export type FlowKind = Verdict | 'rest';
export type FlowSide = 'freed' | 'stays';
/** How far a group is in a running cleanup: waiting, cleaned, or finished with problems. */
export type FlowProgress = 'queued' | 'done' | 'problem';

export interface FlowItem {
  id: string;
  label: string;
  /** Bytes the group covers. A group of no bytes has nothing to draw and is left out. */
  size: number;
  verdict: Verdict;
  side: FlowSide;
  progress?: FlowProgress;
  /** A click on the group moves it between the two sides. */
  clickable: boolean;
}

export interface FlowInput {
  items: readonly FlowItem[];
  /** What the whole chart stands for: the disk's used space, or the scanned size. Everything not in a group is "the rest". */
  total: number;
  width: number;
  height: number;
}

export interface FlowNode {
  /** The item's id; `REST_ID` for everything else. */
  id: string;
  kind: FlowKind;
  label: string;
  size: number;
  side: FlowSide;
  progress: FlowProgress | undefined;
  clickable: boolean;
  /** The bar on the left. */
  y: number;
  h: number;
  /** Where the ribbon ends on the right. */
  ty: number;
  th: number;
  /** The ribbon as an SVG path. */
  path: string;
}

export interface FlowBar {
  y: number;
  h: number;
  /** Bytes of the groups that end here. */
  size: number;
}

export interface FlowLayout {
  width: number;
  height: number;
  /** Left edge of the bars on the left, and their width. */
  barX: number;
  barW: number;
  /** Left edge of the bars on the right. */
  endX: number;
  nodes: FlowNode[];
  /** Absent when no group is on that side. */
  freed: FlowBar | null;
  stays: FlowBar | null;
}

export const REST_ID = 'rest';

/** The geometry of the mockup. */
export const BAR_WIDTH = 14;
export const PAD = 8;
export const SIDE_GAP = 26;
const LABEL_WIDE = 250;
const LABEL_NARROW = 188;
const NARROW_BELOW = 620;
const RIGHT_LABEL = 148;
/** The ribbons need at least this much room between the two columns of bars. */
const MIN_RIBBON = 40;

/**
 * The minimum bar height and the gap between bars, tried in turn: the mockup's 16 and 5 first, then tighter ones
 * when there are so many groups that the first do not fit. Below the last, bars share the room equally.
 */
const RUNGS: readonly { min: number; gap: number }[] = [
  { min: 16, gap: 5 },
  { min: 12, gap: 4 },
  { min: 9, gap: 3 },
  { min: 6, gap: 2 },
];

const round = (value: number): number => Math.round(value * 100) / 100;

/**
 * Heights for `sizes` that add up to `room`. Each is proportional to its size, but none is below `min`: the small
 * ones are held at `min` and the others share what is left, in proportion. When even the minimum does not fit, the
 * room is shared equally. A size of 0 gets the minimum like any other.
 */
export function fitHeights(sizes: readonly number[], room: number, min: number): number[] {
  const count = sizes.length;

  if (count === 0) {
    return [];
  }

  const space = Math.max(0, room);
  const total = sizes.reduce((sum, size) => sum + Math.max(0, size), 0);

  if (count * min >= space || total <= 0) {
    return sizes.map(() => space / count);
  }

  // Hold the smallest at `min`, one more each round, until the rest are proportionally above it.
  const order = sizes
    .map((size, index) => ({ size: Math.max(0, size), index }))
    .sort((a, b) => a.size - b.size);
  const heights = new Array<number>(count).fill(min);
  let held = 0;
  let left = total;

  for (;;) {
    const free = space - held * min;
    const scale = free / left;
    const next = order[held];

    if (!next || next.size * scale >= min) {
      for (const entry of order.slice(held)) {
        heights[entry.index] = entry.size * scale;
      }

      return heights;
    }

    left -= next.size;
    held++;
  }
}

interface Spec {
  id: string;
  kind: FlowKind;
  item: FlowItem | undefined;
  size: number;
}

/** The groups, largest first (equal sizes keep their order), then everything else. */
function specsOf(input: FlowInput): Spec[] {
  const groups: Spec[] = input.items
    .filter((item) => Number.isFinite(item.size) && item.size > 0)
    .map((item, order) => ({ item, order }))
    .sort((a, b) => b.item.size - a.item.size || a.order - b.order)
    .map(({ item }) => ({ id: item.id, kind: item.verdict, item, size: item.size }));
  const covered = groups.reduce((sum, spec) => sum + spec.size, 0);
  // Groups can add up to more than the total (overlapping matches, a stale disk reading); the chart then has no rest.
  const rest = Number.isFinite(input.total) ? input.total - covered : 0;

  return rest > 0 ? [...groups, { id: REST_ID, kind: 'rest', item: undefined, size: rest }] : groups;
}

export function emptyLayout(width: number, height: number): FlowLayout {
  return { width, height, barX: 0, barW: BAR_WIDTH, endX: 0, nodes: [], freed: null, stays: null };
}

/** The ribbon from the bar at `y` (height `h`) on the left to the end at `ty` (height `th`) on the right. */
export function ribbonPath(
  layout: Pick<FlowLayout, 'barX' | 'barW' | 'endX'>,
  node: Pick<FlowNode, 'y' | 'h' | 'ty' | 'th'>,
): string {
  const from = layout.barX + layout.barW;
  const to = layout.endX;
  const mid = (from + to) / 2;
  const { y, h, ty, th } = node;

  return (
    `M${round(from)},${round(y)}C${round(mid)},${round(y)} ${round(mid)},${round(ty)} ${round(to)},${round(ty)}` +
    `L${round(to)},${round(ty + th)}C${round(mid)},${round(ty + th)} ${round(mid)},${round(y + h)} ${round(from)},${round(y + h)}Z`
  );
}

/** Lays the chart out in a box of `width` by `height` pixels. A box with no room, or nothing to show, has no nodes. */
export function layoutFlow(input: FlowInput): FlowLayout {
  const width = Math.max(0, input.width);
  const height = Math.max(0, input.height);
  const specs = specsOf(input);

  if (specs.length === 0 || width === 0 || height === 0) {
    return emptyLayout(width, height);
  }

  const barX = Math.min(width < NARROW_BELOW ? LABEL_NARROW : LABEL_WIDE, Math.max(0, width * 0.4));
  const endX = Math.max(barX + BAR_WIDTH + MIN_RIBBON, width - RIGHT_LABEL);
  const count = specs.length;
  const inner = Math.max(0, height - PAD * 2);
  const rung =
    RUNGS.find((candidate) => count * candidate.min + (count - 1) * candidate.gap <= inner) ??
    (RUNGS[RUNGS.length - 1] as (typeof RUNGS)[number]);
  const room = Math.max(0, inner - rung.gap * (count - 1));
  const heights = fitHeights(
    specs.map((spec) => spec.size),
    room,
    rung.min,
  );

  const sideOf = (spec: Spec): FlowSide =>
    spec.item && spec.item.verdict !== 'keep' ? spec.item.side : 'stays';
  const freedRoom = specs.reduce(
    (sum, spec, index) => sum + (sideOf(spec) === 'freed' ? (heights[index] ?? 0) : 0),
    0,
  );
  const staysRoom = room - freedRoom;
  const both = freedRoom > 0 && staysRoom > 0;
  // The two bars on the right are set apart by a gap; the ribbons get slightly thinner there if the gaps on the left
  // were too small to leave that much room.
  const shrink = both && room > 0 ? Math.min(1, (inner - SIDE_GAP) / room) : 1;
  const staysStart = PAD + freedRoom * shrink + (both ? SIDE_GAP : 0);

  const base = { barX, barW: BAR_WIDTH, endX };
  let y = PAD;
  let freedAt = PAD;
  let staysAt = staysStart;
  let freedSize = 0;
  let staysSize = 0;

  const nodes = specs.map((spec, index): FlowNode => {
    const h = heights[index] ?? 0;
    const side = sideOf(spec);
    const th = h * shrink;
    const ty = side === 'freed' ? freedAt : staysAt;
    const node = {
      id: spec.id,
      kind: spec.kind,
      label: spec.item?.label ?? '',
      size: spec.size,
      side,
      progress: spec.item?.progress,
      clickable: spec.item !== undefined && spec.item.verdict !== 'keep' && spec.item.clickable,
      y,
      h,
      ty,
      th,
    };

    y += h + rung.gap;

    if (side === 'freed') {
      freedAt += th;
      freedSize += spec.size;
    } else {
      staysAt += th;
      staysSize += spec.size;
    }

    return { ...node, path: ribbonPath(base, node) };
  });

  return {
    width,
    height,
    ...base,
    nodes,
    freed: freedRoom > 0 ? { y: PAD, h: freedRoom * shrink, size: freedSize } : null,
    stays: staysRoom > 0 ? { y: staysStart, h: staysRoom * shrink, size: staysSize } : null,
  };
}

/** Two layouts can be blended when the same groups sit in the same places and only the ribbons differ. */
export function canBlend(from: FlowLayout, to: FlowLayout): boolean {
  return (
    from.width === to.width &&
    from.height === to.height &&
    from.nodes.length === to.nodes.length &&
    from.nodes.every((node, index) => node.id === to.nodes[index]?.id && node.h === to.nodes[index]?.h)
  );
}

const mix = (from: number, to: number, t: number): number => from + (to - from) * t;

function blendBar(from: FlowBar | null, to: FlowBar | null, t: number): FlowBar | null {
  return from && to ? { y: mix(from.y, to.y, t), h: mix(from.h, to.h, t), size: to.size } : to;
}

/**
 * The layout `t` (0 to 1) of the way from `from` to `to`: the ribbons' far ends and the bars on the right move,
 * everything else is already where it is going. Layouts that cannot be blended give `to`.
 */
export function blendLayouts(from: FlowLayout, to: FlowLayout, t: number): FlowLayout {
  if (t >= 1 || !canBlend(from, to)) {
    return to;
  }

  const nodes = to.nodes.map((node, index) => {
    const before = from.nodes[index] as FlowNode;
    const ty = mix(before.ty, node.ty, t);
    const th = mix(before.th, node.th, t);

    return { ...node, ty, th, path: ribbonPath(to, { y: node.y, h: node.h, ty, th }) };
  });

  return { ...to, nodes, freed: blendBar(from.freed, to.freed, t), stays: blendBar(from.stays, to.stays, t) };
}
