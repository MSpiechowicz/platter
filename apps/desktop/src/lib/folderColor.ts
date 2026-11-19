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
 * 3. A colour the folders above already use (the siblings of the folder and of each folder above it: its "used
 *    above" colours) comes last in that order. Folders of different levels are shown side by side (the Overview
 *    stack, the map), so a folder deep down does not take the colour of a big folder next to its ancestor
 *    (`/Applications` and `/Users/sam/Projects`). When the palette runs out (the macOS root alone has more folders
 *    than colours), the colour of the smallest folder that uses it is reused first, so a big folder keeps its own.
 *
 * A folder's colour is therefore a property of the folder, whatever view it is drawn in and however deep the
 * view starts.
 */
export function childColors(
  parentColor: string | null,
  children: readonly { id: number; allocated: number }[],
  usedAbove: readonly UsedColor[] = [],
): Map<number, string> {
  const sorted = [...children].sort((a, b) => b.allocated - a.allocated || a.id - b.id);
  const usedBy = new Map(usedAbove.map((used) => [used.color.toLowerCase(), used.allocated]));
  const others = PALETTE.filter((color) => color.toLowerCase() !== parentColor?.toLowerCase());
  const reused = others
    .filter((color) => usedBy.has(color.toLowerCase()))
    .sort((a, b) => (usedBy.get(a.toLowerCase()) ?? 0) - (usedBy.get(b.toLowerCase()) ?? 0));
  const free = [...others.filter((color) => !usedBy.has(color.toLowerCase())), ...reused];
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

/** A colour used above a folder, with the size of the biggest folder that uses it there. */
export interface UsedColor {
  color: string;
  allocated: number;
}

/**
 * The "used above" colours of the children of a folder (see `childColors`, rule 3): the folder's own `usedAbove`
 * and the colours of the folder and its siblings, `siblings`, which `colors` holds. Slate is no colour of the
 * palette and is left out.
 */
export function usedBelow(
  usedAbove: readonly UsedColor[],
  colors: ReadonlyMap<number, string>,
  siblings: readonly { id: number; allocated: number }[],
): UsedColor[] {
  const all = new Map<string, number>();
  const add = (color: string, allocated: number): void => {
    const key = color.toLowerCase();

    if (key !== SLATE.toLowerCase()) {
      all.set(key, Math.max(all.get(key) ?? 0, allocated));
    }
  };

  usedAbove.forEach((used) => add(used.color, used.allocated));
  siblings.forEach((sibling) => {
    const color = colors.get(sibling.id);

    if (color !== undefined) {
      add(color, sibling.allocated);
    }
  });

  return [...all].map(([color, allocated]) => ({ color, allocated }));
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
  /**
   * The colours used above `node` (see `childColors`, rule 3), which its own children avoid. Found from the tree like
   * its colour, and remembered.
   */
  usedAboveOf(node: NodeId): Promise<UsedColor[]>;
  /**
   * Tells the resolver a colour that is already known, so it need not look it up, with the colours used above the
   * folder when they are known too. A known value is never replaced.
   */
  seed(node: NodeId, color: string, usedAbove?: readonly UsedColor[]): void;
}

/**
 * Finds folder colours from the tree of one scan: the path of a folder gives its ancestors, and each ancestor's
 * biggest children give the next colour down. Answers are cached for the life of the resolver, and one request
 * is made per ancestor however many folders are asked about at once. Make a new resolver for every scan.
 */
export function createFolderColorResolver(api: Api, scan: ScanId): FolderColorResolver {
  const known = new Map<NodeId, string>();
  /** The colours used above each folder whose children the walk coloured, or that was seeded with them. */
  const above = new Map<NodeId, UsedColor[]>();
  const children = new Map<NodeId, Promise<{ id: number; allocated: number }[]>>();
  const pending = new Map<NodeId, Promise<void>>();

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

  /**
   * Walks down to `node` from the deepest ancestor whose colour and "used above" colours are both known (the scan
   * root at worst: no colour, nothing used above), remembering both for every folder on the way.
   */
  const resolve = async (node: NodeId): Promise<void> => {
    const path = await api.getPath(scan, node);
    const ids = path.segments.map((segment) => segment.id);
    const root = ids[0] as NodeId;

    if (!above.has(root)) {
      above.set(root, []);
    }

    let start = ids.length - 1;

    while (start > 0 && !(known.has(ids[start] as NodeId) && above.has(ids[start] as NodeId))) {
      start--;
    }

    // The scan root has no colour (slate stands in for it where one is needed, and is not remembered).
    let color: string | null = start === 0 ? null : (known.get(ids[start] as NodeId) ?? null);

    for (let depth = start; depth < ids.length - 1; depth++) {
      const parent = ids[depth] as NodeId;
      const child = ids[depth + 1] as NodeId;
      const parentAbove = above.get(parent) ?? [];
      const kids = await biggestChildren(parent);
      const colors = childColors(color, kids, parentAbove);
      const childAbove = usedBelow(parentAbove, colors, kids);

      remember(colors);
      for (const id of colors.keys()) {
        if (!above.has(id as NodeId)) {
          above.set(id as NodeId, childAbove);
        }
      }

      color = known.get(child) ?? SLATE;
      known.set(child, color);
      if (!above.has(child)) {
        above.set(child, childAbove);
      }
    }
  };

  /** One walk per folder at a time, however many ask. */
  const walk = (node: NodeId): Promise<void> => {
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
  };

  return {
    async colorOf(node) {
      if (!known.has(node)) {
        await walk(node);
      }

      return known.get(node) ?? SLATE;
    },

    async usedAboveOf(node) {
      if (!above.has(node)) {
        await walk(node);
      }

      return above.get(node) ?? [];
    },

    childrenOf(parent, parentColor, kids) {
      const parentAbove = above.get(parent) ?? [];
      const colors = childColors(parentColor, kids, parentAbove);
      const childAbove = usedBelow(parentAbove, colors, kids);

      remember(colors);
      for (const id of colors.keys()) {
        if (!above.has(id as NodeId)) {
          above.set(id as NodeId, childAbove);
        }
      }

      return colors;
    },

    seed(node, color, usedAbove) {
      if (!known.has(node)) {
        known.set(node, color);
      }
      if (usedAbove !== undefined && !above.has(node)) {
        above.set(node, [...usedAbove]);
      }
    },
  };
}
