import { treemapSquarify, type HierarchyRectangularNode } from 'd3-hierarchy';

/** What the band needs to know about the data of a node: whether it belongs in the band. */
export interface Banded {
  band?: boolean;
  /** The "smaller items" tile: it is placed after every other tile, whatever it weighs. */
  last?: boolean;
}

export interface BandOptions {
  /** The target shape of tiles outside the band (width over height). */
  ratio: number;
  /** A tile narrower or shorter than this is not readable: it goes to the band. */
  minWidth: number;
  minHeight: number;
  /** The least room a tile of the band needs, gaps included. */
  tileWidth: number;
  rowHeight: number;
  /** A tile of the band is never wider than this. */
  maxTileWidth: number;
}

type Child<T> = HierarchyRectangularNode<T>;

/** Biggest first, but a tile that goes last stays last. */
function byValueLastLast<T extends Banded>(a: Child<T>, b: Child<T>): number {
  return Number(a.data.last === true) - Number(b.data.last === true) || (b.value ?? 0) - (a.value ?? 0);
}

/** `tiles` with the ones that go last moved to the end, the rest in their order. */
const lastToEnd = <T extends Banded>(tiles: Child<T>[]): Child<T>[] => [
  ...tiles.filter((tile) => tile.data.last !== true),
  ...tiles.filter((tile) => tile.data.last === true),
];

/** A d3 tiling function: it gives each child of `node` its rectangle inside the given one. */
export type Tiling<T> = (node: Child<T>, x0: number, y0: number, x1: number, y1: number) => void;

/**
 * The tiling of the top level (the root's children). Tiles flagged as `band` (those that were made larger to be read),
 * and tiles that the squarified layout would make too thin to read (a 1% folder beside a 97% one is a strip 12 px
 * wide), go in a band along the bottom: as tall as a readable tile, in a row or several. The rest are laid out above
 * it. Anywhere else it is the usual squarified layout. A tile that is moved to the band gets `band` set, so the
 * caller can draw it as a plain tile.
 */
export function createBandTiling<T extends Banded>(options: BandOptions): Tiling<T> {
  const squarify = treemapSquarify.ratio(options.ratio);
  const unreadable = (child: Child<T>): boolean =>
    child.x1 - child.x0 < options.minWidth || child.y1 - child.y0 < options.minHeight;

  /** Squarifies `children` alone in the given rectangle (`node` is told that they are all it has). */
  function squarifyOnly(
    node: Child<T>,
    children: Child<T>[],
    x0: number,
    y0: number,
    x1: number,
    y1: number,
  ): void {
    const { children: all, value } = node;
    const writable = node as { value?: number };

    node.children = children;
    writable.value = children.reduce((sum, child) => sum + (child.value ?? 0), 0);
    squarify(node, x0, y0, x1, y1);
    node.children = all;
    writable.value = value;
  }

  /**
   * When the band's tiles would have to be stretched wider than `maxTileWidth`, the room that is left in the band's
   * row goes to the smallest of the tiles above it (at most four, and only if they fill it about as much as they
   * should). Returns those tiles, and none when the band is full or nothing fits.
   */
  function tilesBesideBand(
    node: Child<T>,
    main: Child<T>[],
    band: Child<T>[],
    at: { width: number; rowHeight: number; rows: number; y0: number; y1: number },
  ): Child<T>[] {
    if (at.rows !== 1 || at.width / band.length <= options.maxTileWidth || main.length < 2) {
      return [];
    }

    const capacity = (at.width - options.maxTileWidth * band.length) * at.rowHeight;
    const perValue = (at.width * (at.y1 - at.y0)) / (node.value || 1);
    const hosts: Child<T>[] = [];

    let used = 0;

    for (const child of [...main].sort((a, b) => (a.value ?? 0) - (b.value ?? 0))) {
      const area = (child.value ?? 0) * perValue;

      if (hosts.length >= 4 || hosts.length >= main.length - 1 || used + area > capacity * 1.3) {
        break;
      }

      hosts.push(child);
      used += area;
    }

    hosts.forEach((child) => (child.data.band = true));

    return lastToEnd(hosts);
  }

  return (node, x0, y0, x1, y1) => {
    const children = node.children ?? [];
    const width = x1 - x0;

    if (node.depth !== 0 || children.length < 2) {
      squarify(node, x0, y0, x1, y1);

      return;
    }

    let band = children.filter((child) => child.data.band);

    for (let pass = 0; pass <= children.length; pass++) {
      // With a band, the tile that goes last stands in it, so that it is the last of the whole map (bottom right).
      if (band.length > 0) {
        band = [
          ...band,
          ...children.filter((child) => child.data.last === true && !band.includes(child)),
        ].sort(byValueLastLast);
      }

      const main = children.filter((child) => !band.includes(child));
      const perRow = Math.max(1, Math.min(band.length, Math.floor(width / options.tileWidth)));
      const rows = Math.ceil(band.length / perRow);
      // As tall as a readable tile, or as tall as the weights ask for when that is more.
      const bandArea = band.reduce((sum, child) => sum + (child.value ?? 0), 0) / (node.value || 1);
      const rowHeight = Math.max(options.rowHeight, band.length === 0 ? 0 : (bandArea * (y1 - y0)) / rows);
      const top = y1 - rowHeight * rows;

      if (main.length === 0 || top - y0 < options.rowHeight) {
        // No room for a band: everything is laid out together.
        squarify(node, x0, y0, x1, y1);

        return;
      }

      squarifyOnly(node, main, x0, y0, x1, top);

      const worst =
        main.length > 1
          ? main.filter(unreadable).sort((a, b) => (a.value ?? 0) - (b.value ?? 0))[0]
          : undefined;

      if (!worst) {
        const hosts = tilesBesideBand(node, main, band, { width, rowHeight, rows, y0, y1 });

        if (hosts.length > 0) {
          // The rest of the band's row is given to the smallest of the other tiles, which become plain tiles too.
          squarifyOnly(
            node,
            main.filter((child) => !hosts.includes(child)),
            x0,
            y0,
            x1,
            top,
          );
          squarifyOnly(node, hosts, x0 + options.maxTileWidth * band.length, top, x1, y1);
        }

        band.forEach((child, index) => {
          const row = Math.floor(index / perRow);
          const inRow = Math.min(perRow, band.length - row * perRow);
          const column = index - row * perRow;
          const each = hosts.length > 0 ? options.maxTileWidth : width / inRow;

          child.x0 = x0 + each * column;
          child.x1 = x0 + each * (column + 1);
          child.y0 = top + rowHeight * row;
          child.y1 = top + rowHeight * (row + 1);
        });

        return;
      }

      // A tile in the band is a plain tile: it is as tall as one line of its name and one of its size.
      worst.data.band = true;
      band = [...band, worst].sort(byValueLastLast);
    }

    squarify(node, x0, y0, x1, y1);
  };
}
