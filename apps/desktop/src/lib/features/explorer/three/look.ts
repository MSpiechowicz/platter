import { Color, LineBasicMaterial, MeshLambertMaterial } from 'three';
import { MERGED_STEP } from '../colors';

const HOVER_GLOW = 0x2a2f38;
const EDGE_OPACITY = 0.7;
const EDGE_OPACITY_DIM = 0.25;
/** A column that is not selected keeps its colour at this share of its brightness: solid, but dark. */
const DIM_SHARE = 0.32;
/** Edges are drawn lighter than the face they outline. */
const EDGE_LIGHTER = 26;

/** `percent` of `color` over the page background, mixed the way the mock-up mixes it. */
function shade(background: Color, color: Color, percent: number): Color {
  return background.clone().lerp(color, percent / 100);
}

/**
 * How one column is painted: a fill material and an edge material for each of its three levels.
 * Boxes of the same column and level share them, so a column is lit or dimmed by changing three materials.
 * The colours are mixed towards the page background, so a new background (the theme changed) repaints the materials
 * in place with `retheme`, and `paint` applies the highlight again.
 */
export class Look {
  readonly #color: Color;
  #base: Color[] = [];
  #dark: Color[] = [];
  #quietBase: Color[] = [];
  #quietDark: Color[] = [];
  readonly fills = [0, 1, 2].map(() => new MeshLambertMaterial());
  readonly edges = [0, 1, 2].map(() => edgeMaterial());
  /** The same for a floor or part that stands for "N smaller items": the column's colour, one step quieter. */
  readonly quietFills = [0, 1, 2].map(() => new MeshLambertMaterial());
  readonly quietEdges = [0, 1, 2].map(() => edgeMaterial());

  /** `mixes` is `columnMix()`: the share of the colour at each level (1 is a column without floors, 2 a floor, 3 a part and the lid). */
  constructor(color: string, background: Color, mixes: readonly number[]) {
    this.#color = new Color(color);
    this.retheme(background, mixes);
  }

  /** Mixes every colour of the column towards `background`. */
  retheme(background: Color, mixes: readonly number[]): void {
    const percents = [1, 2, 3].map((level) => mixes[level] as number);
    const quiet = percents.map((percent) => percent - MERGED_STEP);
    const mix = (percents: number[], share = 1): Color[] =>
      percents.map((percent) => shade(background, this.#color, percent * share));
    const edgeColors = (percents: number[]): Color[] =>
      percents.map((percent) => shade(background, this.#color, Math.min(100, percent + EDGE_LIGHTER)));

    this.#base = mix(percents);
    this.#dark = mix(percents, DIM_SHARE);
    this.#quietBase = mix(quiet);
    this.#quietDark = mix(quiet, DIM_SHARE);

    this.fills.forEach((material, index) => material.color.copy(this.#base[index] as Color));
    this.quietFills.forEach((material, index) => material.color.copy(this.#quietBase[index] as Color));
    edgeColors(percents).forEach((color, index) => this.edges[index]?.color.copy(color));
    edgeColors(quiet).forEach((color, index) => this.quietEdges[index]?.color.copy(color));
  }

  /** `dim` darkens the column, `glow` makes it shine a little (the pointer is over it). */
  paint(dim: boolean, glow: boolean): void {
    this.fills.forEach((material, index) => {
      material.color.copy((dim ? this.#dark : this.#base)[index] as Color);
      material.emissive.set(glow ? HOVER_GLOW : 0);
    });
    this.quietFills.forEach((material, index) => {
      material.color.copy((dim ? this.#quietDark : this.#quietBase)[index] as Color);
      material.emissive.set(glow ? HOVER_GLOW : 0);
    });
    [...this.edges, ...this.quietEdges].forEach((material) => {
      material.opacity = dim ? EDGE_OPACITY_DIM : EDGE_OPACITY;
    });
  }

  dispose(): void {
    this.fills.forEach((material) => material.dispose());
    [...this.edges, ...this.quietEdges].forEach((material) => material.dispose());
    this.quietFills.forEach((material) => material.dispose());
  }
}

/** The edge material of a face: a lighter line of the face's colour, set by `retheme`. */
function edgeMaterial(): LineBasicMaterial {
  return new LineBasicMaterial({ transparent: true, opacity: EDGE_OPACITY });
}
