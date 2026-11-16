/** Rectangles, segments and the connector route: plain geometry for the label placement. */

export type Point = readonly [x: number, y: number];
/** Left, top, right, bottom. */
export type Rect = readonly [x0: number, y0: number, x1: number, y1: number];

/** The connector runs this far down from the label before it turns across. */
const STUB = 9;
/** The last run into the dot is at least this long. */
const TURN_ABOVE_DOT = 6;

export function rectsOverlap(a: Rect, b: Rect, air = 0): boolean {
  return a[0] < b[2] + air && a[2] > b[0] - air && a[1] < b[3] + air && a[3] > b[1] - air;
}

/** True when the segment from `p` to `q` passes through the rectangle (Liang-Barsky clipping). */
export function segmentHitsRect(p: Point, q: Point, rect: Rect, air = 0): boolean {
  const dx = q[0] - p[0];
  const dy = q[1] - p[1];
  const window = { enter: 0, leave: 1 };

  return (
    clip(-dx, p[0] - (rect[0] - air), window) &&
    clip(dx, rect[2] + air - p[0], window) &&
    clip(-dy, p[1] - (rect[1] - air), window) &&
    clip(dy, rect[3] + air - p[1], window)
  );
}

/** Narrows the part of the segment that is inside one edge of the rectangle; false when nothing is left. */
function clip(direction: number, distance: number, window: { enter: number; leave: number }): boolean {
  if (direction === 0) {
    return distance >= 0;
  }

  const at = distance / direction;

  if (direction < 0) {
    window.enter = Math.max(window.enter, at);
  } else {
    window.leave = Math.min(window.leave, at);
  }

  return window.enter <= window.leave;
}

export function pathHitsRect(path: readonly Point[], rect: Rect, air = 0): boolean {
  return path.slice(1).some((point, index) => segmentHitsRect(path[index] as Point, point, rect, air));
}

/**
 * Down from the middle of the label, across, then down into the dot. Never a diagonal.
 * `turn` is where it runs across: just under the label, or just above the dot.
 */
export function route(rect: Rect, dot: Point, turn: 'label' | 'dot' = 'label'): Point[] {
  const x = (rect[0] + rect[2]) / 2;
  const bottom = rect[3];
  const above = dot[1] - TURN_ABOVE_DOT;
  const across = turn === 'label' ? Math.min(bottom + STUB, above) : above;

  if (Math.abs(x - dot[0]) < 2) {
    return [
      [x, bottom],
      [dot[0], dot[1]],
    ];
  }

  if (across <= bottom) {
    // No room to turn above the dot: across at the dot's height instead.
    return [
      [x, bottom],
      [x, dot[1]],
      [dot[0], dot[1]],
    ];
  }

  return [
    [x, bottom],
    [x, across],
    [dot[0], across],
    [dot[0], dot[1]],
  ];
}
