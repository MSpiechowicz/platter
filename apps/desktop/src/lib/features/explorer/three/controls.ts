import type { NodeId } from '../../../api/types';
import { MAX_DISTANCE, MIN_DISTANCE, type CameraView } from './fit';

/** A press that moves less than this many pixels is a click, not a drag. */
const CLICK_SLOP = 4;
const TURN_PER_PIXEL = 0.008;
const TILT_PER_PIXEL = 0.006;
const MIN_TILT = 0.12;
const MAX_TILT = 1.5;
const ZOOM_OUT = 1.1;
const ZOOM_IN = 0.91;

/** What the pointer needs from the scene. */
export interface Steering {
  view: CameraView;
  /** The column under the pointer, if it can be selected. */
  hit(event: PointerEvent): NodeId | null;
  /** The camera was turned: fit it again, unless the user has zoomed. */
  turned(): void;
  zoomed(): void;
  invalidate(): void;
}

export interface Reactions {
  onHover(id: NodeId | null): void;
  /** A click without a drag: the id of the column, or `null` for empty space. */
  onPick(id: NodeId | null): void;
  onDrag(active: boolean): void;
}

/**
 * Drag turns the block, the wheel zooms, a click without a drag picks, moving over a column hovers it.
 * All listeners go away when `signal` aborts.
 */
export function attachControls(
  host: HTMLElement,
  steering: Steering,
  reactions: Reactions,
  signal: AbortSignal,
): void {
  const { view } = steering;

  let drag: { x: number; y: number; th: number; phi: number; moved: number } | null = null;
  let hovered: NodeId | null = null;

  host.addEventListener(
    'pointerdown',
    (event) => {
      drag = { x: event.clientX, y: event.clientY, th: view.th, phi: view.phi, moved: 0 };
      host.setPointerCapture(event.pointerId);
    },
    { signal },
  );

  host.addEventListener(
    'pointermove',
    (event) => {
      if (!drag) {
        const id = steering.hit(event);

        if (id !== hovered) {
          hovered = id;
          reactions.onHover(id);
        }

        return;
      }

      const dx = event.clientX - drag.x;
      const dy = event.clientY - drag.y;

      drag.moved = Math.max(drag.moved, Math.abs(dx) + Math.abs(dy));

      if (drag.moved > CLICK_SLOP) {
        reactions.onDrag(true);
        view.th = drag.th - dx * TURN_PER_PIXEL;
        view.phi = Math.max(MIN_TILT, Math.min(MAX_TILT, drag.phi - dy * TILT_PER_PIXEL));
        steering.turned();
        steering.invalidate();
      }
    },
    { signal },
  );

  host.addEventListener(
    'pointerup',
    (event) => {
      const finished = drag;

      drag = null;
      reactions.onDrag(false);

      if (finished && finished.moved <= CLICK_SLOP) {
        reactions.onPick(steering.hit(event));
      }
    },
    { signal },
  );

  host.addEventListener(
    'pointercancel',
    () => {
      drag = null;
      reactions.onDrag(false);
    },
    { signal },
  );

  host.addEventListener(
    'pointerleave',
    () => {
      if (!drag && hovered !== null) {
        hovered = null;
        reactions.onHover(null);
      }
    },
    { signal },
  );

  // Not passive: the page must not scroll while the wheel zooms.
  host.addEventListener(
    'wheel',
    (event) => {
      event.preventDefault();
      steering.zoomed();
      view.d = Math.max(
        MIN_DISTANCE,
        Math.min(MAX_DISTANCE, view.d * (event.deltaY > 0 ? ZOOM_OUT : ZOOM_IN)),
      );
      steering.invalidate();
    },
    { signal, passive: false },
  );
}
