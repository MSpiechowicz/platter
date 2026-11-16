/**
 * Placing the 3D labels: pure geometry over rectangles, no DOM and no three.js.
 * A label wants to sit just above its column. When another label is in the way it moves to the nearest free spot,
 * and a right-angled connector (down, across, down) joins the middle of its bottom edge to a dot on the column.
 */

import { pathHitsRect, rectsOverlap, route, type Point, type Rect } from './geometry';
import { MAX_LABELS } from './legibility';

export interface LabelRequest {
  id: number;
  /** Where the label would sit if nothing were in the way: bottom centre, before the lift. */
  x: number;
  y: number;
  /** The dot on the column. */
  ax: number;
  ay: number;
  width: number;
  height: number;
  /** Placed even when there are more labels than `MAX_LABELS`, and counted among them (the hovered or selected one). */
  pinned?: boolean;
}

/** Where a label ended up relative to its first choice. Handed back next frame, so labels keep their place. */
export interface Hint {
  offset: Point;
  turn: 'label' | 'dot';
  /** How much it had to give up to find this spot (an index into `TIERS`, 0 is nothing). */
  tier: number;
}

export interface PlacedLabel {
  id: number;
  rect: Rect;
  /** Starts at the middle of the label's bottom edge, ends at the dot. */
  path: Point[];
  dot: Point;
  hint: Hint;
}

export interface Bounds {
  width: number;
  height: number;
}

/** Gap between a label's bottom edge and the point it belongs to. */
export const LABEL_LIFT = 16;
/** Keep this far from the edge of the card. */
const EDGE = 4;
/** Labels and connectors keep this much air between them. */
const AIR = 3;
const DOT_RADIUS = 5;
const LINE_HALF_WIDTH = 2;
/** A label may slide down until its bottom edge is this far above its dot. */
const ABOVE_DOT = 10;

const STEP_X = 16;
const STEP_Y = 10;
const MAX_STEPS_X = 28;
const MAX_STEPS_UP = 24;
const MAX_STEPS_DOWN = 5;
/** Once a spot is found, look this much farther (relative, plus a minimum) for a stricter one. */
const SEARCH_AFTER_FIND = 3;
const SEARCH_AFTER_FIND_MIN = 40;

/** Every offset the label may try, the nearest first. Down is limited per label: it must stay above its own dot. */
const OFFSETS: readonly Point[] = (() => {
  const offsets: Point[] = [];

  for (let row = -MAX_STEPS_UP; row <= MAX_STEPS_DOWN; row++) {
    for (let column = -MAX_STEPS_X; column <= MAX_STEPS_X; column++) {
      offsets.push([column * STEP_X, row * STEP_Y]);
    }
  }

  return offsets.sort(
    (a, b) => Math.hypot(a[0], a[1]) - Math.hypot(b[0], b[1]) || a[1] - b[1] || a[0] - b[0],
  );
})();

function labelRect(request: LabelRequest, offset: Point): Rect {
  const left = request.x + offset[0] - request.width / 2;
  const bottom = request.y - LABEL_LIFT + offset[1];

  return [left, bottom - request.height, left + request.width, bottom];
}

const insideBounds = (rect: Rect, bounds: Bounds): boolean =>
  rect[0] >= EDGE && rect[2] <= bounds.width - EDGE && rect[1] >= EDGE && rect[3] <= bounds.height - EDGE;

/** The rectangles a placed connector keeps for itself, one thin box per segment. */
function lineBoxes(path: readonly Point[]): Rect[] {
  return path.slice(1).map((point, index): Rect => {
    const from = path[index] as Point;

    return [
      Math.min(from[0], point[0]) - LINE_HALF_WIDTH,
      Math.min(from[1], point[1]) - LINE_HALF_WIDTH,
      Math.max(from[0], point[0]) + LINE_HALF_WIDTH,
      Math.max(from[1], point[1]) + LINE_HALF_WIDTH,
    ];
  });
}

interface Taken {
  labels: Rect[];
  dots: Rect[];
  /** Connectors, one thin box per segment. */
  lines: Rect[];
}

/**
 * What a spot must keep clear of, besides what its label never may cover: other labels, other dots and the
 * connectors already drawn. These are given up one by one, last first, when there is no room.
 */
interface Care {
  /** A connector may not pass behind another label. */
  paths: boolean;
  /** The connector may not pass over another label's dot. */
  dots: boolean;
  /** A connector may not cross another connector. */
  crossings: boolean;
}

const TIERS: readonly Care[] = [
  { paths: true, dots: true, crossings: true },
  { paths: true, dots: true, crossings: false },
  { paths: true, dots: false, crossings: false },
  { paths: false, dots: false, crossings: false },
];

interface Spot {
  rect: Rect;
  path: Point[];
  hint: Hint;
  /** How far the label moved from its first choice. */
  moved: number;
}

/** True when the spot touches any of the things the care asks it to avoid. Stops at the first. */
function isBlocked(spot: Spot, taken: Taken, care: Care): boolean {
  const covers = (other: Rect): boolean => rectsOverlap(spot.rect, other, AIR);
  const passes = (other: Rect): boolean => pathHitsRect(spot.path, other, AIR);

  return (
    taken.labels.some((label) => covers(label) || (care.paths && passes(label))) ||
    taken.lines.some((line) => covers(line) || (care.crossings && pathHitsRect(spot.path, line))) ||
    taken.dots.some((dot) => covers(dot) || (care.dots && passes(dot)))
  );
}

/** How many labels the spot overlaps: all that is left to compare when nothing is free. */
const labelClashes = (spot: Spot, taken: Taken): number =>
  taken.labels.filter((label) => rectsOverlap(spot.rect, label, AIR)).length;

function spotAt(request: LabelRequest, offset: Point, turn: 'label' | 'dot', tier = 0): Spot {
  const rect = labelRect(request, offset);

  return {
    rect,
    path: route(rect, [request.ax, request.ay], turn),
    hint: { offset, turn, tier },
    moved: Math.hypot(offset[0], offset[1]),
  };
}

/** Every spot inside the card, the nearest first. */
function* spotsInside(request: LabelRequest, bounds: Bounds): Generator<Spot> {
  const lowest = request.ay - ABOVE_DOT - (request.y - LABEL_LIFT);

  for (const offset of OFFSETS) {
    if (offset[1] > lowest) {
      continue;
    }

    const spot = spotAt(request, offset, 'label');

    if (!insideBounds(spot.rect, bounds)) {
      continue;
    }

    yield spot;
    yield spotAt(request, offset, 'dot');
  }
}

/** The label is wider than the room: slide it back inside, even if it then covers something. */
function squeezeInside(request: LabelRequest, bounds: Bounds): Spot {
  const rect = labelRect(request, [0, 0]);
  const shiftX = Math.max(EDGE - rect[0], Math.min(0, bounds.width - EDGE - rect[2]));
  const shiftY = Math.max(EDGE - rect[1], Math.min(0, bounds.height - EDGE - rect[3]));

  return spotAt(request, [shiftX, shiftY], 'label');
}

function choose(request: LabelRequest, bounds: Bounds, taken: Taken, hint: Hint | undefined): Spot {
  // First the places a label should be: home, then where it was last time while that is still as good.
  const tries: Hint[] = [
    { offset: [0, 0], turn: 'label', tier: 0 },
    { offset: [0, 0], turn: 'dot', tier: 0 },
  ];

  if (hint) {
    tries.push(hint);
  }

  for (const { offset, turn, tier } of tries) {
    const spot = spotAt(request, offset, turn, tier);

    if (insideBounds(spot.rect, bounds) && !isBlocked(spot, taken, TIERS[tier] as Care)) {
      return spot;
    }
  }

  // The nearest spot that touches nothing; a farther one wins over a nearer one only by touching less.
  let found: Spot | undefined;
  let foundTier = TIERS.length;
  let fallback: Spot | undefined;
  let fewest = Infinity;

  for (const spot of spotsInside(request, bounds)) {
    // Far from the first choice a stricter tier is not worth the jump, unless the connector still crosses a label.
    if (
      found &&
      foundTier < TIERS.length - 1 &&
      spot.moved > SEARCH_AFTER_FIND * (found.moved + SEARCH_AFTER_FIND_MIN)
    ) {
      break;
    }

    for (let tier = 0; tier < foundTier; tier++) {
      if (!isBlocked(spot, taken, TIERS[tier] as Care)) {
        found = { ...spot, hint: { ...spot.hint, tier } };
        foundTier = tier;
        break;
      }
    }

    if (foundTier === 0) {
      return found as Spot;
    }

    if (!found) {
      // No room so far: remember the spot that touches the least.
      const count = labelClashes(spot, taken);

      if (count < fewest) {
        fewest = count;
        fallback = { ...spot, hint: { ...spot.hint, tier: TIERS.length - 1 } };
      }
    }
  }

  return found ?? fallback ?? squeezeInside(request, bounds);
}

function placeInOrder(
  requests: readonly LabelRequest[],
  bounds: Bounds,
  hints: ReadonlyMap<number, Hint> | undefined,
): PlacedLabel[] {
  const dots = requests.map((request): Rect => [
    request.ax - DOT_RADIUS,
    request.ay - DOT_RADIUS,
    request.ax + DOT_RADIUS,
    request.ay + DOT_RADIUS,
  ]);
  const labels: Rect[] = [];
  const lines: Rect[] = [];

  return requests.map((request, index) => {
    // Every other dot counts from the start, not only the placed ones: a label must not hide a dot that is still to come.
    const taken: Taken = { labels, lines, dots: dots.filter((_, at) => at !== index) };
    const { rect, path, hint } = choose(request, bounds, taken, hints?.get(request.id));

    labels.push(rect);
    lines.push(...lineBoxes(path));

    return { id: request.id, rect, path, dot: [request.ax, request.ay], hint };
  });
}

/** The ids of the labels whose connector passes behind another label. */
function blockedConnectors(placed: readonly PlacedLabel[]): Set<number> {
  const blocked = new Set<number>();

  for (const label of placed) {
    if (placed.some((other) => other !== label && pathHitsRect(label.path, other.rect))) {
      blocked.add(label.id);
    }
  }

  return blocked;
}

/** How many times a crowded set is placed again with its problem labels first. */
const RETRIES = 3;

/** True when the label, a dot and the margin all fit in the card at all. */
function canBeShown(request: LabelRequest, bounds: Bounds): boolean {
  return (
    request.width <= bounds.width - 2 * EDGE &&
    request.height <= bounds.height - 2 * EDGE &&
    request.ax >= 0 &&
    request.ax <= bounds.width &&
    request.ay >= 0 &&
    request.ay <= bounds.height
  );
}

/** The pinned labels, then the first of the others, `max` in all; in the order they were asked for. */
function mostImportant(requests: readonly LabelRequest[], max: number): LabelRequest[] {
  const pinned = requests.filter((request) => request.pinned).slice(0, max);
  const others = requests.filter((request) => !request.pinned).slice(0, max - pinned.length);
  const chosen = new Set([...pinned, ...others]);

  return requests.filter((request) => chosen.has(request));
}

/**
 * Places the labels in the order given (put the biggest first: it gets the best spot). At most `MAX_LABELS` are
 * placed: the pinned ones, then the first of the rest; a label that cannot fit in the card at all, or whose dot is
 * outside it, is left out. A label never overlaps another label or dot. A connector does not pass behind a label,
 * nor over a dot or another connector, as long as there is room for that; if a label ends up with its connector
 * behind another, the set is placed again with such labels first, because a label placed early has the pick of the
 * spots. Every label that is returned lies inside `bounds`.
 * `hints` (the result of the last frame) let a label stay where it was while that spot is still free.
 */
export function placeLabels(
  all: readonly LabelRequest[],
  bounds: Bounds,
  hints?: ReadonlyMap<number, Hint>,
): PlacedLabel[] {
  const requests = mostImportant(
    all.filter((request) => canBeShown(request, bounds)),
    MAX_LABELS,
  );
  let order = [...requests];
  let best = placeInOrder(order, bounds, hints);
  let fewest = blockedConnectors(best);

  for (let retry = 0; retry < RETRIES && fewest.size > 0; retry++) {
    order = [
      ...order.filter((request) => fewest.has(request.id)),
      ...order.filter((request) => !fewest.has(request.id)),
    ];

    const attempt = placeInOrder(order, bounds, hints);
    const blocked = blockedConnectors(attempt);

    if (blocked.size < fewest.size) {
      best = attempt;
      fewest = blocked;
    } else {
      break;
    }
  }

  // Back in the order that was asked for.
  return requests.map((request) => best.find((label) => label.id === request.id) as PlacedLabel);
}
