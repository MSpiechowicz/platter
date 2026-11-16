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

/** Keeps the end of a path, which is the folder's own name: "…/sam/Downloads". Cut at a "/" when it can be. */
function clipName(text: string): string {
  if (text.length <= MAX_NAME) {
    return text;
  }

  const segments = text.split('/');
  let kept = '';

  for (let at = segments.length - 1; at > 0; at--) {
    const next = `/${segments.slice(at).join('/')}`;

    if (next.length + 1 > MAX_NAME) {
      break;
    }

    kept = next;
  }

  return kept === '' ? `…${text.slice(1 - MAX_NAME)}` : `…${kept}`;
}

/** Stacks the slabs from the bottom up; the labels alternate between the right and the left side. */
export function layoutStack(slabs: Slab[], total: number): StackLayout {
  const budget = Math.min(FULL_HEIGHT, TOP_ROOM - slabs.length * GAP);
  const perByte = total > 0 ? budget / total : 0;

  let bottom = BASE;

  const placed = slabs.map((slab, index): PlacedSlab => {
    const height = Math.max(MIN_HEIGHT, slab.size * perByte);
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
