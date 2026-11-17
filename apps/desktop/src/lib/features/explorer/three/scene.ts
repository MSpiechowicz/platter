import {
  AmbientLight,
  BoxGeometry,
  Color,
  DirectionalLight,
  EdgesGeometry,
  Group,
  LineSegments,
  Mesh,
  PerspectiveCamera,
  Raycaster,
  Scene,
  Vector2,
  Vector3,
  WebGLRenderer,
} from 'three';
import { COLLAPSED_NODE_ID } from '../../../api/client';
import type { NodeId } from '../../../api/types';
import { theme } from '../../../state/theme.svelte';
import { colorOf, columnMix, type ColorMap } from '../colors';
import { ANCHOR_RISE, type ColumnsLayout, type Column } from '../layout/columns';
import type { Rect } from '../layout/geometry';
import { attachControls, type Reactions } from './controls';
import {
  BLOCK_CORNERS,
  FAR,
  fitFor,
  FIELD_OF_VIEW,
  INITIAL_VIEW,
  layoutCorners,
  NEAR,
  placeCamera,
  type CameraView,
} from './fit';
import { Look } from './look';

/** A label's place on screen, in pixels of the host element. */
export interface ProjectedLabel {
  id: NodeId;
  /** Where the label would sit, bottom centre. */
  x: number;
  y: number;
  /** The dot on the column. */
  ax: number;
  ay: number;
  /** False when the column is behind the camera. */
  visible: boolean;
}

export interface SceneFrame {
  width: number;
  height: number;
  labels: ProjectedLabel[];
  /** The block's outline on screen: left, top, right, bottom. */
  block: Rect;
}

export interface SceneHandlers extends Reactions {
  /** After every rendered frame: where the labels belong. */
  onFrame(frame: SceneFrame): void;
}

export interface SceneOptions {
  /** Keep the picture after it is shown, so a headless screenshot is not blank. */
  keepDrawing: boolean;
  /** Let the columns grow when a folder is shown. */
  animate: boolean;
}

const GROW_MS = 650;
const MIN_SIZE = 0.05;
const AMBIENT = 2.1;
const SUN = 1.5;
/** The dot sits on the lid, a little below the label's anchor. */
const DOT_DROP = ANCHOR_RISE / 2;

/** The page background the columns are mixed towards, and the card the scene is cleared to: tokens of the theme on screen. */
const pageBackground = (): Color => new Color(theme.token('--bg'));
const cardColor = (): Color => new Color(theme.token('--card'));

const easeOut = (k: number): number => 1 - Math.pow(1 - k, 3);

/**
 * The 3D columns: renderer, camera and picking. It knows nothing about folders or the page; it is given a layout
 * and colours and reports clicks, hovers and where the labels belong.
 * A frame is drawn only when the layout, the highlight, the camera, the size or the theme changes (and while the
 * columns grow), so nothing runs while the view is idle; `dispose()` ends it and frees what the GPU holds.
 */
export class ColumnsScene {
  readonly #host: HTMLElement;
  readonly #handlers: SceneHandlers;
  readonly #options: SceneOptions;
  readonly #renderer: WebGLRenderer;
  readonly #scene = new Scene();
  readonly #camera = new PerspectiveCamera(FIELD_OF_VIEW, 1, NEAR, FAR);
  readonly #raycaster = new Raycaster();
  readonly #listeners = new AbortController();
  readonly #resizeObserver: ResizeObserver;
  readonly #unsubscribeTheme: () => void;
  readonly #box = new BoxGeometry(1, 1, 1);
  readonly #outline = new EdgesGeometry(this.#box, 30);
  readonly #view: CameraView = { ...INITIAL_VIEW };

  /** What the camera frames: the real extent of the boxes, not a nominal block. */
  #corners: readonly Vector3[] = BLOCK_CORNERS;
  /** The size of the host the camera was last set up for; `null` until it has one. */
  #size: { width: number; height: number } | null = null;
  /** What the columns are mixed towards, from the theme on screen. */
  #background = pageBackground();
  #mixes = columnMix();
  #group: Group | null = null;
  #columns: Column[] = [];
  /** One look per column, and one per floor of a column whose floors have colours of their own. */
  #looks = new Map<string, { id: NodeId; look: Look }>();
  #ids = new Map<Mesh, NodeId>();
  #picks: Mesh[] = [];
  #selected: NodeId | null = null;
  #hovered: NodeId | null = null;
  #zoomed = false;
  #growStart = 0;
  #frame = 0;
  #disposed = false;

  /** Throws when the browser cannot give WebGL; the caller shows a message and falls back to the flat map. */
  constructor(host: HTMLElement, handlers: SceneHandlers, options: SceneOptions) {
    this.#host = host;
    this.#handlers = handlers;
    this.#options = options;

    this.#renderer = new WebGLRenderer({ antialias: true, preserveDrawingBuffer: options.keepDrawing });
    this.#renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.#renderer.setClearColor(cardColor());
    host.prepend(this.#renderer.domElement);

    const sun = new DirectionalLight(0xffffff, SUN);

    sun.position.set(-60, 120, 80);
    this.#scene.add(new AmbientLight(0xffffff, AMBIENT), sun);

    this.#resizeObserver = new ResizeObserver(() => this.#resize());
    this.#resizeObserver.observe(host);
    this.#resize();
    this.#unsubscribeTheme = theme.subscribe(() => this.#retheme());

    attachControls(
      host,
      {
        view: this.#view,
        hit: (event) => this.#hit(event),
        turned: () => {
          if (!this.#zoomed) {
            this.#fit();
          }
        },
        zoomed: () => (this.#zoomed = true),
        invalidate: () => this.invalidate(),
      },
      {
        onHover: (id) => {
          this.#hovered = id;
          handlers.onHover(id);
        },
        onPick: (id) => handlers.onPick(id),
        onDrag: (active) => handlers.onDrag(active),
      },
      this.#listeners.signal,
    );
  }

  /** Replaces the columns. The camera fits the new block, and zooming by hand is forgotten. */
  setLayout(layout: ColumnsLayout, colors: ColorMap): void {
    this.#clear();

    const group = new Group();

    this.#group = group;
    this.#columns = layout.columns;
    this.#corners = layoutCorners(layout);

    for (const column of layout.columns) {
      for (const box of column.boxes) {
        const look = this.#lookFor(column.top, box.hue, colors);

        const sides = [box.x1 - box.x0, box.y1 - box.y0, box.z1 - box.z0];

        // The layout makes no such box; this only keeps a bad one from reaching the GPU.
        if (!sides.every((side) => Number.isFinite(side) && side >= MIN_SIZE)) {
          continue;
        }

        const mesh = new Mesh(this.#box, (box.rest ? look.quietFills : look.fills)[box.level - 1]);
        mesh.position.set((box.x0 + box.x1) / 2, (box.y0 + box.y1) / 2, (box.z0 + box.z1) / 2);
        mesh.scale.set(sides[0] as number, sides[1] as number, sides[2] as number);

        if (box.edges) {
          mesh.add(new LineSegments(this.#outline, (box.rest ? look.quietEdges : look.edges)[box.level - 1]));
        }

        group.add(mesh);
        this.#picks.push(mesh);
        this.#ids.set(mesh, column.top);
      }
    }

    this.#scene.add(group);
    this.#zoomed = false;
    this.#growStart = performance.now();
    this.#paint();
    this.#fit();
    this.invalidate();
  }

  /** The selected column stays lit, the others go dark; the hovered one glows. */
  setHighlight(selected: NodeId | null, hovered: NodeId | null): void {
    this.#selected = selected;
    this.#hovered = hovered;
    this.#paint();
    this.invalidate();
  }

  /** Draws a frame on the next animation frame. */
  invalidate(): void {
    if (this.#frame === 0 && !this.#disposed) {
      this.#frame = requestAnimationFrame(this.#draw);
    }
  }

  /** Stops the loop and frees everything the GPU holds. */
  dispose(): void {
    if (this.#disposed) {
      return;
    }

    this.#disposed = true;
    this.#unsubscribeTheme();
    cancelAnimationFrame(this.#frame);
    this.#frame = 0;
    this.#listeners.abort();
    this.#resizeObserver.disconnect();
    this.#clear();
    this.#box.dispose();
    this.#outline.dispose();
    this.#renderer.dispose();
    this.#renderer.forceContextLoss();
    this.#renderer.domElement.remove();
  }

  /** The look of a column, or of one of its floors when that has a colour of its own (the lid keeps the column's). */
  #lookFor(top: NodeId, hue: NodeId | undefined, colors: ColorMap): Look {
    const key = `${top}:${hue ?? ''}`;
    const known = this.#looks.get(key);

    if (known) {
      return known.look;
    }

    const color = hue === undefined ? colorOf(colors, top) : (colors.get(hue) ?? colorOf(colors, top));
    const look = new Look(color, this.#background, this.#mixes);

    this.#looks.set(key, { id: top, look });

    return look;
  }

  /** The theme changed: the card behind and every column's mix towards the page are redone, and a frame is drawn. */
  #retheme(): void {
    this.#background = pageBackground();
    this.#mixes = columnMix();
    this.#renderer.setClearColor(cardColor());

    for (const { look } of this.#looks.values()) {
      look.retheme(this.#background, this.#mixes);
    }

    this.#paint();
    this.invalidate();
  }

  #clear(): void {
    if (this.#group) {
      this.#scene.remove(this.#group);
    }

    this.#looks.forEach(({ look }) => look.dispose());
    this.#group = null;
    this.#columns = [];
    this.#looks = new Map();
    this.#ids = new Map();
    this.#picks = [];
  }

  #paint(): void {
    for (const { id, look } of this.#looks.values()) {
      const selectable = id !== COLLAPSED_NODE_ID;

      look.paint(this.#selected !== null && id !== this.#selected, selectable && id === this.#hovered);
    }
  }

  #resize(): void {
    const width = this.#host.clientWidth;
    const height = this.#host.clientHeight;

    if (!width || !height) {
      return;
    }

    this.#size = { width, height };
    this.#renderer.setSize(width, height);
    this.#camera.aspect = width / height;
    this.#camera.updateProjectionMatrix();

    // Whatever the window does, the block stays whole inside the card.
    if (this.#group) {
      this.#fit();
    }

    this.invalidate();
  }

  /** Fits the block to the card as it is now. Before the card has a size there is nothing to fit to: `#resize` does it. */
  #fit(): void {
    if (this.#size) {
      this.#view.d = fitFor(this.#size.width, this.#size.height, this.#view, this.#corners);
    }
  }

  #grown(): number {
    if (!this.#options.animate) {
      return 1;
    }

    return easeOut(Math.min(1, (performance.now() - this.#growStart) / GROW_MS));
  }

  #screen(point: Vector3, width: number, height: number): [number, number, number] {
    const projected = point.project(this.#camera);

    return [((projected.x + 1) / 2) * width, ((1 - projected.y) / 2) * height, projected.z];
  }

  #project(grown: number, width: number, height: number): SceneFrame {
    const labels = this.#columns.map((column): ProjectedLabel => {
      const { x, y, z } = column.anchor;
      const [labelX, labelY, depth] = this.#screen(new Vector3(x, y * grown, z), width, height);
      const [dotX, dotY] = this.#screen(new Vector3(x, (y - DOT_DROP) * grown, z), width, height);

      return { id: column.top, x: labelX, y: labelY, ax: dotX, ay: dotY, visible: depth <= 1 };
    });

    const corners = this.#corners.map((corner) => this.#screen(corner.clone(), width, height));
    const xs = corners.map((corner) => corner[0]);
    const ys = corners.map((corner) => corner[1]);

    return {
      width,
      height,
      labels,
      block: [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)],
    };
  }

  readonly #draw = (): void => {
    this.#frame = 0;

    if (this.#disposed || !this.#group) {
      return;
    }

    const grown = this.#grown();

    this.#group.scale.set(1, Math.max(0.001, grown), 1);
    placeCamera(this.#camera, this.#view);
    this.#renderer.render(this.#scene, this.#camera);
    this.#handlers.onFrame(this.#project(grown, this.#host.clientWidth, this.#host.clientHeight));

    if (grown < 1) {
      this.invalidate();
    }
  };

  #hit(event: PointerEvent): NodeId | null {
    const bounds = this.#host.getBoundingClientRect();
    const pointer = new Vector2(
      ((event.clientX - bounds.left) / bounds.width) * 2 - 1,
      -((event.clientY - bounds.top) / bounds.height) * 2 + 1,
    );

    this.#raycaster.setFromCamera(pointer, this.#camera);

    const nearest = this.#raycaster.intersectObjects(this.#picks, false)[0];
    const id = nearest ? (this.#ids.get(nearest.object as Mesh) ?? null) : null;

    return id;
  }
}
