import { PerspectiveCamera, Vector3 } from 'three';
import { BLOCK_TOP, FOOTPRINT, type ColumnsLayout } from '../layout/columns';

/** Orbit camera around the block: distance, angle from the vertical, turn, and the height it looks at. */
export interface CameraView {
  d: number;
  phi: number;
  th: number;
  ty: number;
}

export const INITIAL_VIEW: CameraView = { d: 188, phi: 1.02, th: 0.62, ty: 17 };
export const MIN_DISTANCE = 60;
/** Far enough to fit a block into a card a few times taller than it is wide. */
export const MAX_DISTANCE = 2400;
export const FIELD_OF_VIEW = 32;
export const NEAR = 1;
export const FAR = 6000;

/** The block must stay inside this part of the picture (normalised device coordinates). */
const LIMIT_SIDE = 0.9;
const LIMIT_BOTTOM = -0.9;
/** More room above than below: the labels live there. */
const LIMIT_TOP = 0.66;

const SEARCH_MIN = 40;
const SEARCH_STEPS = 24;

const half = FOOTPRINT / 2;

/** The eight corners of the nominal block: what the camera frames when there are no columns to measure. */
export const BLOCK_CORNERS: readonly Vector3[] = cornersOf(-half, half, -half, half, BLOCK_TOP);

function cornersOf(x0: number, x1: number, z0: number, z1: number, top: number): Vector3[] {
  return [x0, x1].flatMap((x) => [z0, z1].flatMap((z) => [0, top].map((y) => new Vector3(x, y, z))));
}

/** The eight corners of the box that holds every box of the layout: this is what has to stay in the picture. */
export function layoutCorners(layout: ColumnsLayout): Vector3[] {
  const boxes = layout.columns.flatMap((column) => column.boxes);

  if (boxes.length === 0) {
    return [...BLOCK_CORNERS];
  }

  return cornersOf(
    Math.min(...boxes.map((box) => box.x0)),
    Math.max(...boxes.map((box) => box.x1)),
    Math.min(...boxes.map((box) => box.z0)),
    Math.max(...boxes.map((box) => box.z1)),
    Math.max(...boxes.map((box) => box.y1)),
  );
}

export function placeCamera(camera: PerspectiveCamera, view: CameraView, distance: number = view.d): void {
  const ring = Math.sin(view.phi);

  camera.position.set(
    distance * ring * Math.sin(view.th),
    view.ty + distance * Math.cos(view.phi),
    distance * ring * Math.cos(view.th),
  );
  camera.lookAt(0, view.ty, 0);
  camera.updateMatrixWorld();
}

function isInPicture(camera: PerspectiveCamera, corners: readonly Vector3[]): boolean {
  return corners.every((corner) => {
    const projected = corner.clone().project(camera);

    return Math.abs(projected.x) < LIMIT_SIDE && projected.y > LIMIT_BOTTOM && projected.y < LIMIT_TOP;
  });
}

/** The shortest distance at which the whole block, with room above it for the labels, is inside the picture. */
export function fitDistance(
  camera: PerspectiveCamera,
  view: CameraView,
  corners: readonly Vector3[] = BLOCK_CORNERS,
): number {
  let near = SEARCH_MIN;
  let far = MAX_DISTANCE;

  for (let step = 0; step < SEARCH_STEPS; step++) {
    const middle = (near + far) / 2;

    placeCamera(camera, view, middle);

    if (isInPicture(camera, corners)) {
      far = middle;
    } else {
      near = middle;
    }
  }

  return far;
}

/**
 * The same, for a picture of `width` by `height` pixels: the camera is made here, from those numbers only, so the
 * result cannot depend on the state of any other camera (a size from before the first resize, say).
 */
export function fitFor(
  width: number,
  height: number,
  view: CameraView,
  corners: readonly Vector3[] = BLOCK_CORNERS,
): number {
  const camera = new PerspectiveCamera(FIELD_OF_VIEW, width / height, NEAR, FAR);

  camera.updateProjectionMatrix();

  return fitDistance(camera, view, corners);
}
