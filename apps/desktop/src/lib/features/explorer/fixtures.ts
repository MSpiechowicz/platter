import { COLLAPSED_NODE_ID } from '../../api/client';
import type { LodNode, NodeDto } from '../../api/types';

/** Builders for the layout tests: a folder or file of a given size with the given children. */
let nextId = 1;

export function node(name: string, allocated: number, extra: Partial<NodeDto> = {}): NodeDto {
  return {
    id: nextId++,
    name,
    kind: 'dir',
    allocated,
    apparent: allocated,
    files: 1,
    hasChildren: false,
    category: 'other',
    mtime: 1_700_000_000,
    ...extra,
  };
}

export function lod(name: string, size: number, children: LodNode[] = []): LodNode {
  return { node: node(name, size, { hasChildren: children.length > 0 }), children };
}

export function leaf(name: string, size: number): LodNode {
  return { node: node(name, size, { kind: 'file' }), children: [] };
}

export function smaller(count: number, size: number): LodNode {
  return {
    node: node(`${count} smaller items`, size, { id: COLLAPSED_NODE_ID, kind: 'collapsed' }),
    children: [],
  };
}

/** A focused folder with the given sizes as children, each with two sub-folders. */
export function sample(sizes: number[]): LodNode {
  const children = sizes.map((size, index) =>
    lod(`dir${index}`, size, [
      lod(`a${index}`, size * 0.6, [leaf(`x${index}`, size * 0.4), leaf(`y${index}`, size * 0.2)]),
      lod(`b${index}`, size * 0.4, [leaf(`z${index}`, size * 0.4)]),
    ]),
  );

  return lod(
    'focus',
    sizes.reduce((sum, size) => sum + size, 0),
    children,
  );
}

/**
 * A folder shaped like a real disk: one child with nearly everything, a few small ones, then dust. Every child has
 * a few sub-folders, so the map has more than one level to draw. Sizes are percentages of the folder.
 */
export function skewed(
  percents: number[] = [97, 1.3, 1.3, 0.2, 0.05, 0.05, 0.04, 0.03, 0.02, 0.01],
): LodNode {
  const scale = 100_000;
  const children = percents.map((percent, index) => {
    const size = Math.round(percent * scale);
    const count = index === 0 ? 12 : 4;
    const weights = Array.from({ length: count }, (_, part) => 1 / (part + 1));
    const sum = weights.reduce((total, weight) => total + weight, 0);
    const parts = weights.map((weight, part) => leaf(`f${index}-${part}`, (size * weight) / sum));

    return lod(`dir${index}`, size, parts);
  });

  return lod(
    'focus',
    children.reduce((sum, child) => sum + child.node.allocated, 0),
    children,
  );
}

/** A folder whose content was not read: the answer ended above it (it has children, but the data has none). */
export function unread(name: string, size: number): LodNode {
  return { node: node(name, size, { hasChildren: true }), children: [] };
}
