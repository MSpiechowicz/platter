import { platform } from '../../state/platform.svelte';
import type { Slab } from './slabs';

/**
 * Geometry of the isometric stack. It is drawn at its natural size, 1 unit = 1 px: the viewBox is as wide as the
 * Overview's card at the narrowest width where the page keeps two columns, so it is only scaled below that.
 */
const CX = 360;
const HALF_WIDTH = 150;
const HALF_DEPTH = 62;
const GAP = 10;
const BASE = 420;
const MIN_HEIGHT = 14;
const LEADER = 38;
/** A full disk is this tall in total. */
const FULL_HEIGHT = 304;
const TOP_ROOM = 350;
const MAX_LABEL = 28;
/** The name is bold 14px monospace and has about 165 of the 720 units beside the stack. */
const MAX_NAME = 20;

export const STACK_VIEWBOX = { width: 720, height: 500 };

export interface PlacedSlab {
  slab: Slab;
  index: number;
  top: string;
  left: string;
  right: string;
  leader: { x1: number; x2: number; y: number };
  text: { x: number; y: number; anchor: 'start' | 'end' };
  /** The name, cut at its start when it is long, so the folder's own name stays visible. */
  name: string;
  inside: string;
}

export interface StackLayout {
  /** Outline of the platform the stack stands on. */
  base: string;
  slabs: PlacedSlab[];
}

const points = (...pairs: [number, number][]): string => pairs.map(([x, y]) => `${x},${y}`).join(' ');

const clip = (text: string): string => (text.length > MAX_LABEL ? `${text.slice(0, MAX_LABEL - 1)}…` : text);

/** Keeps the end of a path, which is the folder's own name: "…/sam/Downloads". Cut at a separator when it can be. */
function clipName(text: string): string {
  if (text.length <= MAX_NAME) {
    return text;
  }

  const separator = platform.separator;
  const segments = text.split(separator);
  let kept = '';

  for (let at = segments.length - 1; at > 0; at--) {
    const next = `${separator}${segments.slice(at).join(separator)}`;

    if (next.length + 1 > MAX_NAME) {
      break;
    }

    kept = next;
  }

  return kept === '' ? `…${text.slice(1 - MAX_NAME)}` : `…${kept}`;
}

/**
 * The height of each slab. The sizes are measured against the disk, or against their own sum when that is bigger
 * (on btrfs, shared blocks are charged to every folder that holds them). A slab too thin to show gets the minimum
 * height, and the others give up the room for it, so the heights never add up to more than the budget.
 */
function slabHeights(sizes: number[], total: number, budget: number): number[] {
  if (sizes.length === 0) {
    return [];
  }

  const denominator = Math.max(
    total,
    sizes.reduce((sum, size) => sum + size, 0),
  );
  // With more slabs than the budget can hold at the minimum, they share it equally.
  const floor = Math.min(MIN_HEIGHT, budget / sizes.length);
  const floored = sizes.map(() => false);

  let perByte = 0;

  for (;;) {
    const flooredCount = floored.filter(Boolean).length;
    const flooredBytes = sizes.reduce((sum, size, index) => sum + (floored[index] ? size : 0), 0);
    const room = budget - flooredCount * floor;
    const bytes = denominator - flooredBytes;

    perByte = bytes > 0 ? room / bytes : 0;

    const thin = sizes.flatMap((size, index) => (!floored[index] && size * perByte < floor ? [index] : []));

    if (thin.length === 0) {
      break;
    }

    // A slab below the minimum stays below it as the others give up room, so all of them can be fixed at once.
    for (const index of thin) {
      floored[index] = true;
    }
  }

  return sizes.map((size, index) => (floored[index] ? floor : size * perByte));
}

/** Stacks the slabs from the bottom up; the labels alternate between the right and the left side. */
export function layoutStack(slabs: Slab[], total: number): StackLayout {
  const budget = Math.min(FULL_HEIGHT, TOP_ROOM - slabs.length * GAP);
  const heights = slabHeights(
    slabs.map((slab) => slab.size),
    total,
    budget,
  );

  let bottom = BASE;

  const placed = slabs.map((slab, index): PlacedSlab => {
    const height = heights[index] as number;
    const t = bottom - height;
    const rightSide = index % 2 === 0;
    const edge = rightSide ? CX + HALF_WIDTH : CX - HALF_WIDTH;
    const labelX = rightSide ? edge + LEADER : edge - LEADER;
    const y = t + height / 2;

    bottom = t - GAP;

    return {
      slab,
      index,
      top: points([CX - HALF_WIDTH, t], [CX, t + HALF_DEPTH], [CX + HALF_WIDTH, t], [CX, t - HALF_DEPTH]),
      left: points(
        [CX - HALF_WIDTH, t],
        [CX, t + HALF_DEPTH],
        [CX, t + HALF_DEPTH + height],
        [CX - HALF_WIDTH, t + height],
      ),
      right: points(
        [CX, t + HALF_DEPTH],
        [CX + HALF_WIDTH, t],
        [CX + HALF_WIDTH, t + height],
        [CX, t + HALF_DEPTH + height],
      ),
      leader: { x1: edge, x2: labelX, y },
      text: { x: rightSide ? labelX + 8 : labelX - 8, y, anchor: rightSide ? 'start' : 'end' },
      name: clipName(slab.name),
      inside: clip(slab.inside),
    };
  });

  return {
    base: points(
      [CX - HALF_WIDTH - 26, BASE],
      [CX, BASE + HALF_DEPTH + 11],
      [CX + HALF_WIDTH + 26, BASE],
      [CX, BASE - HALF_DEPTH - 11],
    ),
    slabs: placed,
  };
}
