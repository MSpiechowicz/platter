import { PerspectiveCamera } from 'three';
import { describe, expect, it } from 'vitest';
import { leaf, lod, sample, skewed } from '../fixtures';
import { layoutColumns } from '../layout/columns';
import {
  BLOCK_CORNERS,
  FAR,
  FIELD_OF_VIEW,
  fitDistance,
  fitFor,
  INITIAL_VIEW,
  layoutCorners,
  MAX_DISTANCE,
  NEAR,
  placeCamera,
  type CameraView,
} from './fit';

function camera(aspect: number): PerspectiveCamera {
  const result = new PerspectiveCamera(32, aspect, 1, 2000);

  result.updateProjectionMatrix();

  return result;
}

/** Where the corners of the block land: normalised device coordinates, x and y from -1 to 1. */
function projected(cam: PerspectiveCamera, view: CameraView, distance: number): { x: number; y: number }[] {
  placeCamera(cam, view, distance);

  return BLOCK_CORNERS.map((corner) => {
    const point = corner.clone().project(cam);

    return { x: point.x, y: point.y };
  });
}

const VIEWS: CameraView[] = [
  INITIAL_VIEW,
  { ...INITIAL_VIEW, th: 2.4 },
  { ...INITIAL_VIEW, th: -1.2, phi: 0.3 },
  { ...INITIAL_VIEW, phi: 1.5 },
];
const ASPECTS = [0.7, 1, 1.3, 2.2];

describe('fitDistance', () => {
  it('puts the whole block inside the picture for any window shape and any turn', () => {
    for (const aspect of ASPECTS) {
      for (const view of VIEWS) {
        const cam = camera(aspect);
        const corners = projected(cam, view, fitDistance(cam, view));

        for (const corner of corners) {
          expect(Math.abs(corner.x), `aspect ${aspect}, turn ${view.th}`).toBeLessThanOrEqual(0.9);
          expect(corner.y).toBeGreaterThanOrEqual(-0.9);
          // Room is left above the block for the labels.
          expect(corner.y).toBeLessThanOrEqual(0.66);
        }
      }
    }
  });

  it('is as close as it can be: a little nearer and the block no longer fits', () => {
    for (const aspect of ASPECTS) {
      const cam = camera(aspect);
      const nearer = projected(cam, INITIAL_VIEW, fitDistance(cam, INITIAL_VIEW) * 0.97);
      const worst = Math.max(
        ...nearer.map((corner) => Math.max(Math.abs(corner.x) / 0.9, corner.y / 0.66, -corner.y / 0.9)),
      );

      expect(worst, `aspect ${aspect}`).toBeGreaterThan(1);
    }
  });

  it('keeps away from the block more in a narrow window than in a wide one', () => {
    expect(fitDistance(camera(0.7), INITIAL_VIEW)).toBeGreaterThan(fitDistance(camera(2.2), INITIAL_VIEW));
  });
});

describe('fitFor, for every card shape and every block', () => {
  const SHAPES = [0.5, 0.7, 1, 1.6, 2.4];
  const SIZES: [number, number][] = [
    [400, 800],
    [700, 1000],
    [900, 900],
    [1280, 800],
    [1500, 625],
  ];
  const BLOCKS = {
    'the nominal block': [...BLOCK_CORNERS],
    'a balanced folder': layoutCorners(layoutColumns(sample([600, 300, 100]))),
    'one dominant folder': layoutCorners(layoutColumns(skewed())),
    'a folder with dust': layoutCorners(layoutColumns(skewed([60, 30, 6, 1.6, 1, 0.6, 0.4, 0.2, 0.2]))),
    'a single column': layoutCorners(layoutColumns(lod('f', 10, [leaf('a', 10)]))),
  };

  /** The camera exactly as the scene makes it: from the size of the card alone. */
  function lookThrough(width: number, height: number, view: CameraView, distance: number): PerspectiveCamera {
    const cam = new PerspectiveCamera(FIELD_OF_VIEW, width / height, NEAR, FAR);

    cam.updateProjectionMatrix();
    placeCamera(cam, view, distance);

    return cam;
  }

  it('has every corner of the real bounding box inside the picture, with room above for the labels', () => {
    for (const [name, corners] of Object.entries(BLOCKS)) {
      for (const [width, height] of SIZES) {
        const distance = fitFor(width, height, INITIAL_VIEW, corners);
        const cam = lookThrough(width, height, INITIAL_VIEW, distance);

        for (const corner of corners) {
          const point = corner.clone().project(cam);

          expect(Math.abs(point.x), `${name}, aspect ${width / height}`).toBeLessThanOrEqual(0.9 + 1e-6);
          expect(point.y).toBeGreaterThanOrEqual(-0.9 - 1e-6);
          expect(point.y).toBeLessThanOrEqual(0.66 + 1e-6);
        }
      }
    }

    expect(SIZES.map(([width, height]) => width / height)).toEqual(SHAPES);
  });

  it('keeps the block whole in a very narrow card too, which the old distance limit of 700 could not', () => {
    const distance = fitFor(300, 1200, INITIAL_VIEW);
    const cam = lookThrough(300, 1200, INITIAL_VIEW, distance);

    expect(distance).toBeGreaterThan(700);
    expect(distance).toBeLessThan(MAX_DISTANCE);

    for (const corner of BLOCK_CORNERS) {
      expect(Math.abs(corner.clone().project(cam).x)).toBeLessThanOrEqual(0.9 + 1e-6);
    }
  });

  it('measures the real boxes, not a nominal square: a block that sticks out is fitted further away', () => {
    const wide = BLOCK_CORNERS.map((corner) => corner.clone().multiplyScalar(2));

    expect(fitFor(900, 900, INITIAL_VIEW, wide)).toBeGreaterThan(fitFor(900, 900, INITIAL_VIEW) * 1.5);
  });

  it('depends on the size it is given and on nothing else', () => {
    const first = fitFor(1000, 600, INITIAL_VIEW);

    // Another card in between must not change the answer for this one.
    fitFor(300, 900, INITIAL_VIEW);

    expect(fitFor(1000, 600, INITIAL_VIEW)).toBe(first);
  });

  it('has the bounds of the layout: its boxes, and the lid at the top', () => {
    const corners = layoutCorners(layoutColumns(sample([600, 300, 100])));
    const ys = corners.map((corner) => corner.y);

    expect(Math.min(...ys)).toBe(0);
    expect(Math.max(...ys)).toBeGreaterThan(30);
    expect(Math.max(...corners.map((corner) => Math.abs(corner.x)))).toBeLessThanOrEqual(42);
    expect(layoutCorners({ columns: [], omitted: 0, boosted: false })).toHaveLength(8);
  });
});
