import type { Api } from './api/client';
import { COLLAPSED_NODE_ID } from './api/client';
import type { NodeId, ScanId } from './api/types';
import { PALETTE, SLATE } from './palette';

/**
 * The one rule for folder colours, the same in every view (Overview, Explorer map, 3D columns, lists):
 *
 * 1. The scan root has no colour.
 * 2. Among the children of a folder, biggest first (ties by id), the largest inherits the parent's colour and the
 *    others take the palette colours in order, skipping the parent's. Below the root, which has no colour, every
 *    child takes the palette in order. When the palette is used up the rest are slate, and so are zero-size
 *    children and the synthetic "smaller items" node.
 *
 * A folder's colour is therefore a property of the folder, whatever view it is drawn in and however deep the
 * view starts.
 */
export function childColors(
  parentColor: string | null,
  children: readonly { id: number; allocated: number }[],
): Map<number, string> {
  const sorted = [...children].sort((a, b) => b.allocated - a.allocated || a.id - b.id);
  const free = PALETTE.filter((color) => color.toLowerCase() !== parentColor?.toLowerCase());
  const colors = new Map<number, string>();
  let next = 0;

  sorted.forEach((child, index) => {
    if (child.allocated <= 0 || child.id === COLLAPSED_NODE_ID) {
      colors.set(child.id, SLATE);
    } else if (index === 0 && parentColor !== null) {
      colors.set(child.id, parentColor);
    } else {
      colors.set(child.id, free[next++] ?? SLATE);
    }
  });

  return colors;
}

/** How many of a folder's biggest children are looked at to find a child's colour. */
export const COLOR_CHILDREN = 16;

export interface FolderColorResolver {
  /** The colour of a folder, found from the tree (and remembered). The scan root has none: it answers slate. */
  colorOf(node: NodeId): Promise<string>;
  /**
   * Colours the given children of a folder whose colour is `parentColor`, and remembers them. Pass all the sized
   * children you know; the answer is `childColors` of them.
   */
  childrenOf(
    parent: NodeId,
    parentColor: string | null,
    children: readonly { id: number; allocated: number }[],
  ): Map<number, string>;
  /** Tells the resolver a colour that is already known, so it need not look it up. A known colour is never replaced. */
  seed(node: NodeId, color: string): void;
}

/**
 * Finds folder colours from the tree of one scan: the path of a folder gives its ancestors, and each ancestor's
 * biggest children give the next colour down. Answers are cached for the life of the resolver, and one request
 * is made per ancestor however many folders are asked about at once. Make a new resolver for every scan.
 */
export function createFolderColorResolver(api: Api, scan: ScanId): FolderColorResolver {
  const known = new Map<NodeId, string>();
  const children = new Map<NodeId, Promise<{ id: number; allocated: number }[]>>();
  const pending = new Map<NodeId, Promise<string>>();

  const remember = (colors: Map<number, string>): void => {
    for (const [id, color] of colors) {
      if (!known.has(id)) {
        known.set(id, color);
      }
    }
  };

  const biggestChildren = (parent: NodeId): Promise<{ id: number; allocated: number }[]> => {
    let request = children.get(parent);

    if (!request) {
      request = api
        .getChildren(scan, parent, 0, COLOR_CHILDREN, 'sizeDesc')
        .then((page) => page.items.map(({ id, allocated }) => ({ id, allocated })));
      children.set(parent, request);
      // A failed request is not kept: the next ask tries again.
      request.catch(() => children.delete(parent));
    }

    return request;
  };

  /** Walks down from the deepest ancestor whose colour is known. */
  const resolve = async (node: NodeId): Promise<string> => {
    const path = await api.getPath(scan, node);
    const ids = path.segments.map((segment) => segment.id);

    // The scan root has no colour; slate stands in for it where a colour is needed, and is not remembered
    // (the children of the root take the palette from its first colour).
    if (ids.length === 1) {
      return known.get(node) ?? SLATE;
    }

    // The deepest known colour wins; the walk starts below it.
    let start = ids.length - 1;

    while (start > 0 && !known.has(ids[start] as NodeId)) {
      start--;
    }

    let color: string | null = known.get(ids[start] as NodeId) ?? null;

    for (let depth = start; depth < ids.length - 1; depth++) {
      const parent = ids[depth] as NodeId;
      const child = ids[depth + 1] as NodeId;

      remember(childColors(color, await biggestChildren(parent)));
      color = known.get(child) ?? SLATE;
      known.set(child, color);
    }

    return known.get(node) as string;
  };

  return {
    colorOf(node) {
      const cached = known.get(node);

      if (cached !== undefined) {
        return Promise.resolve(cached);
      }

      let request = pending.get(node);

      if (!request) {
        request = resolve(node);
        pending.set(node, request);
        void request.then(
          () => pending.delete(node),
          () => pending.delete(node),
        );
      }

      return request;
    },

    childrenOf(_parent, parentColor, kids) {
      const colors = childColors(parentColor, kids);

      remember(colors);

      return colors;
    },

    seed(node, color) {
      if (!known.has(node)) {
        known.set(node, color);
      }
    },
  };
}
