import { COLLAPSED_NODE_ID } from '../../api/client';
import type { NodeDto, NodeId } from '../../api/types';
import { MAX_MARKS, type Mark } from '../../state/cleanup.svelte';
import { linksOf, type ChainedLod } from './layout/chains';

/*
 * What the Explorer needs to know about marking, with no component in it: which folder a click on "Mark for cleanup"
 * is about, why a row or the button is off, and which tiles of the map are safe to clean.
 */

/** Why something cannot be marked right now, in plain words; `undefined` when it can. */
export interface Blockers {
  /** A cleanup is planned or running. */
  busy: boolean;
  /** A cleanup ran: the scan no longer describes the disk. */
  outdated: boolean;
  /** The folder on screen is still loading, so which folder an item is in is not settled. */
  loading: boolean;
  /** The most items are marked already. */
  full: boolean;
}

/** Why no mark can be added or removed at the moment (not about one item), or `undefined`. */
export function markingOff(blockers: Blockers): string | undefined {
  if (blockers.busy) {
    return 'A cleanup is in progress.';
  }

  if (blockers.outdated) {
    return 'The disk changed. Rescan to mark again.';
  }

  if (blockers.loading) {
    return 'The folder is still loading.';
  }

  return undefined;
}

export const FULL_TEXT = `At most ${MAX_MARKS.toLocaleString('en-US')} items can be marked at once.`;

export const KEPT_TEXT = 'Kept on purpose: Platter will not clean this.';

/** Why one item can never be marked, whatever the state: it is certain the backend refuses it. */
export function itemOff(node: NodeDto): string | undefined {
  switch (node.kind) {
    case 'symlink':
      return 'A link is never cleaned.';
    case 'mount':
      return 'Another disk or a system folder is never cleaned.';
    case 'other':
    case 'collapsed':
      return 'Only files and folders can be cleaned.';
    default:
      return node.unreadable ? 'Part of it could not be read, so it is not cleaned.' : undefined;
  }
}

/** What the mark button of the details card is about: a node and the folders above it. */
export interface MarkTarget {
  node: NodeDto;
  /** The folders above the node, the scan root first. */
  ancestors: NodeId[];
}

/**
 * The folder the details card is about, with its ancestry: the open folder (when nothing is selected), a selected
 * child, or the last folder of a selected chain tile (`links` are the folders of the tile, first to last).
 */
export function markTarget(
  node: NodeDto | undefined,
  crumbs: readonly NodeId[],
  selected: { links: readonly NodeDto[] } | undefined,
): MarkTarget | undefined {
  if (!node) {
    return undefined;
  }

  if (!selected) {
    // The open folder: the crumbs end with itself.
    return { node, ancestors: crumbs.slice(0, -1) };
  }

  return { node, ancestors: [...crumbs, ...selected.links.slice(0, -1).map((link) => link.id)] };
}

export interface ButtonState {
  /** What the button says and does. */
  label: 'Mark for cleanup' | 'Unmark';
  /** Why it is off; `undefined` when it can be clicked. */
  off: string | undefined;
}

/** The details card's mark button: always there, off when marking does not apply to what the card shows. */
export function markButton(
  target: MarkTarget | undefined,
  context: {
    groupSelected: boolean;
    marked: boolean;
    kept: boolean;
    blockers: Blockers;
  },
): ButtonState {
  const label = context.marked ? 'Unmark' : 'Mark for cleanup';

  if (context.groupSelected) {
    return { label, off: 'This tile stands for several small items. Mark them in the list.' };
  }

  if (!target) {
    return { label, off: 'Nothing to mark.' };
  }

  if (target.ancestors.length === 0) {
    return { label, off: 'The scanned folder itself is never cleaned.' };
  }

  const off = markingOff(context.blockers);

  if (off !== undefined) {
    return { label, off };
  }

  if (context.kept) {
    return { label, off: KEPT_TEXT };
  }

  const never = itemOff(target.node);

  if (never !== undefined) {
    return { label, off: never };
  }

  if (!context.marked && context.blockers.full) {
    return { label, off: FULL_TEXT };
  }

  return { label, off: undefined };
}

/** A `Mark` for the store from an item and the folders above it. */
export function markOf(node: NodeDto, ancestors: readonly NodeId[]): Mark {
  return { id: node.id, allocated: node.allocated, ancestors: [...ancestors] };
}

/**
 * The tiles to draw as safe: the topmost ones that a `safe` suggestion lists. A folder inside one is not drawn as
 * safe again, and when the open folder is itself in a safe one (`inside`) nothing is. A chain tile counts when its
 * first folder is listed (what the rest of the chain holds is not claimed). Returns the ids of the tiles (the id of
 * their first folder).
 */
export function safeTops(
  model: ChainedLod | undefined,
  safe: ReadonlySet<NodeId>,
  inside: boolean,
): ReadonlySet<NodeId> {
  const tops = new Set<NodeId>();

  if (!model || inside || safe.size === 0) {
    return tops;
  }

  const walk = (item: ChainedLod): void => {
    if (item.node.id === COLLAPSED_NODE_ID) {
      return;
    }

    if (safe.has(item.node.id)) {
      tops.add(item.node.id);
      return;
    }

    item.children.forEach(walk);
  };

  model.children.forEach(walk);

  return tops;
}

/** Whether a tile (or column) is marked: any folder it stands for is. */
export function tileMarked(
  item: { node: NodeDto; chain?: readonly NodeDto[] },
  marked: ReadonlySet<NodeId>,
): boolean {
  return marked.size > 0 && linksOf(item).some((link) => marked.has(link.id));
}
