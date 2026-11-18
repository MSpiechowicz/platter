import { COLLAPSED_NODE_ID } from './client';
import { CATEGORIES, type MockNode } from './mock-tree';
import type { Category, NodeDto } from './types';

/** A node as the backend describes it: `unreadable` always, `skipped` for every mount (`unknown` when no reason is known). */
export function toDto(node: MockNode): NodeDto {
  return {
    id: node.id,
    name: node.name,
    kind: node.kind,
    allocated: node.allocated,
    apparent: node.apparent,
    files: node.files,
    hasChildren: node.childCount > 0,
    unreadable: node.unreadable === true,
    ...(node.kind === 'mount' ? { skipped: node.skipped ?? 'unknown' } : {}),
    category: node.category,
    mtime: node.mtime,
  };
}

/**
 * The category holding the most bytes of `nodes`; on a tie the one that comes first in the category list. Like the
 * backend, items without any size at all give `other`.
 */
function heaviestCategory(nodes: MockNode[]): Category {
  const bytes = new Map<Category, number>();

  for (const node of nodes) {
    bytes.set(node.category, (bytes.get(node.category) ?? 0) + node.allocated);
  }

  let best: Category = 'other';
  let most = 0;

  for (const category of CATEGORIES) {
    const value = bytes.get(category) ?? 0;

    if (value > most) {
      best = category;
      most = value;
    }
  }

  return best;
}

/** The synthetic node that stands for everything `hidden` that did not fit: `1 smaller item`, `N smaller items`. */
export function collapsedDto(hidden: MockNode[]): NodeDto {
  return {
    id: COLLAPSED_NODE_ID,
    name: hidden.length === 1 ? '1 smaller item' : `${hidden.length} smaller items`,
    kind: 'collapsed',
    allocated: hidden.reduce((sum, node) => sum + node.allocated, 0),
    apparent: hidden.reduce((sum, node) => sum + node.apparent, 0),
    files: hidden.reduce((sum, node) => sum + node.files, 0),
    hasChildren: false,
    unreadable: false,
    category: heaviestCategory(hidden),
    mtime: hidden.reduce((newest, node) => Math.max(newest, node.mtime), 0),
  };
}
