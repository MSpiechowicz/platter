import { Color, LineBasicMaterial, MeshLambertMaterial } from 'three';
import { BACKGROUND, COLUMN_MIX, MERGED_STEP } from '../colors';

const HOVER_GLOW = 0x2a2f38;
const EDGE_OPACITY = 0.7;
const EDGE_OPACITY_DIM = 0.25;
/** A column that is not selected keeps its colour at this share of its brightness: solid, but dark. */
const DIM_SHARE = 0.32;
/** Edges are drawn lighter than the face they outline. */
const EDGE_LIGHTER = 26;

const background = new Color(BACKGROUND);

/** `percent` of `color` over the page background, mixed the way the mock-up mixes it. */
function shade(color: string, percent: number): Color {
  return background.clone().lerp(new Color(color), percent / 100);
}

/** The edge material of a face that is `percent` of `color`: a lighter line of the same colour. */
function edge(color: string, percent: number): LineBasicMaterial {
  return new LineBasicMaterial({
    color: shade(color, Math.min(100, percent + EDGE_LIGHTER)),
    transparent: true,
    opacity: EDGE_OPACITY,
  });
}

/**
 * How one column is painted: a fill material and an edge material for each of its three levels.
 * Boxes of the same column and level share them, so a column is lit or dimmed by changing three materials.
 */
export class Look {
  readonly #base: Color[];
  readonly #dark: Color[];
  readonly #quietBase: Color[];
  readonly #quietDark: Color[];
  readonly fills: MeshLambertMaterial[];
  readonly edges: LineBasicMaterial[];
  /** The same for a floor or part that stands for "N smaller items": the column's colour, one step quieter. */
  readonly quietFills: MeshLambertMaterial[];
  readonly quietEdges: LineBasicMaterial[];

  constructor(color: string) {
    // Level 1 is a column without floors, 2 a floor, 3 a part of a floor and the lid.
    const percents = [1, 2, 3].map((level) => COLUMN_MIX[level] as number);

    this.#base = percents.map((percent) => shade(color, percent));
    this.#dark = percents.map((percent) => shade(color, percent * DIM_SHARE));
    const quiet = percents.map((percent) => percent - MERGED_STEP);

    this.#quietBase = quiet.map((percent) => shade(color, percent));
    this.#quietDark = quiet.map((percent) => shade(color, percent * DIM_SHARE));
    this.quietFills = this.#quietBase.map((base) => new MeshLambertMaterial({ color: base }));
    this.quietEdges = quiet.map((percent) => edge(color, percent));
    this.fills = this.#base.map((base) => new MeshLambertMaterial({ color: base }));
    this.edges = percents.map((percent) => edge(color, percent));
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
