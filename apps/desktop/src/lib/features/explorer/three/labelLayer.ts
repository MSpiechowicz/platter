import type { NodeId } from '../../../api/types';
import { colorOf, type ColorMap } from '../colors';
import { placeLabels, type Hint, type LabelRequest } from '../layout/labels';
import type { SceneFrame } from './scene';

const SVG_NS = 'http://www.w3.org/2000/svg';
const DIM_BORDER = '55';
const BORDER = 'cc';
const DIM_LINK = 0.3;
/** The dot's outline, the page background: a CSS variable, so the page's theme is picked up without a redraw. */
const DOT_OUTLINE = 'var(--bg)';

interface Link {
  group: SVGGElement;
  path: SVGPathElement;
  dot: SVGCircleElement;
}

/**
 * Puts the HTML labels and their connectors where `placeLabels` says, every frame. It only touches the DOM
 * (labels are rendered by the page, the connectors are drawn here), so the scene itself stays free of the page.
 */
export class LabelLayer {
  #svg: SVGSVGElement | null = null;
  readonly #labels = new Map<NodeId, HTMLElement>();
  readonly #links = new Map<NodeId, Link>();
  #selected: NodeId | null = null;
  #hovered: NodeId | null = null;
  #colors: ColorMap = new Map();
  /** Where each label went last frame: while that spot stays free the label stays put. */
  #hints = new Map<NodeId, Hint>();

  attach(svg: SVGSVGElement): void {
    this.#svg = svg;
  }

  /** What is selected and hovered and which colours the columns have; applied on the next frame. */
  configure(selected: NodeId | null, hovered: NodeId | null, colors: ColorMap): void {
    this.#selected = selected;
    this.#hovered = hovered;
    this.#colors = colors;
  }

  /** A label element appeared. Returns what to call when it goes away. */
  track(id: NodeId, element: HTMLElement): () => void {
    this.#labels.set(id, element);

    return () => {
      this.#labels.delete(id);
      this.#links.get(id)?.group.remove();
      this.#links.delete(id);
    };
  }

  update(frame: SceneFrame): void {
    const { width, height } = frame;
    const shown = frame.labels.filter((label) => label.visible && this.#labels.has(label.id));

    // Measure everything first, then write: reading a size after a write would force a layout per label.
    const requests = shown.map((label): LabelRequest => {
      const element = this.#labels.get(label.id) as HTMLElement;

      return {
        id: label.id,
        x: label.x,
        y: label.y,
        ax: label.ax,
        ay: label.ay,
        width: element.offsetWidth,
        height: element.offsetHeight,
        // Past ten labels only the ones the pointer or the selection is on are shown besides.
        pinned: label.id === this.#selected || label.id === this.#hovered,
      };
    });
    const results = placeLabels(requests, { width, height }, this.#hints);
    const placed = new Map(results.map((label) => [label.id, label]));

    this.#hints = new Map(results.map((label) => [label.id, label.hint]));

    for (const label of frame.labels) {
      const element = this.#labels.get(label.id);
      const spot = placed.get(label.id);

      if (!element) {
        continue;
      }

      if (!spot) {
        element.style.visibility = 'hidden';
        this.#linkFor(label.id)?.group.setAttribute('visibility', 'hidden');
        continue;
      }

      const color = colorOf(this.#colors, label.id);
      const dim = this.#selected !== null && label.id !== this.#selected;
      const link = this.#linkFor(label.id);

      element.style.visibility = 'visible';
      element.style.left = `${(spot.rect[0] + spot.rect[2]) / 2}px`;
      element.style.top = `${spot.rect[3]}px`;
      element.style.borderColor = `${color}${dim ? DIM_BORDER : BORDER}`;

      if (link) {
        link.group.setAttribute('visibility', 'visible');
        link.group.setAttribute('opacity', String(dim ? DIM_LINK : 1));
        link.path.setAttribute(
          'd',
          `M${spot.path.map((point) => `${point[0].toFixed(1)},${point[1].toFixed(1)}`).join('L')}`,
        );
        link.path.setAttribute('stroke', color);
        link.dot.setAttribute('cx', spot.dot[0].toFixed(1));
        link.dot.setAttribute('cy', spot.dot[1].toFixed(1));
        link.dot.setAttribute('fill', color);
      }
    }
  }

  #linkFor(id: NodeId): Link | undefined {
    const existing = this.#links.get(id);

    if (existing || !this.#svg) {
      return existing;
    }

    const group = document.createElementNS(SVG_NS, 'g');
    const path = document.createElementNS(SVG_NS, 'path');
    const dot = document.createElementNS(SVG_NS, 'circle');

    path.setAttribute('fill', 'none');
    path.setAttribute('stroke-width', '1.5');
    path.setAttribute('stroke-linejoin', 'round');
    dot.setAttribute('r', '3.5');
    dot.style.stroke = DOT_OUTLINE;
    dot.setAttribute('stroke-width', '1.5');
    group.append(path, dot);
    this.#svg.append(group);

    const link = { group, path, dot };

    this.#links.set(id, link);

    return link;
  }
}
