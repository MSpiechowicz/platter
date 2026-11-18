import { collapsedDto, toDto } from './mock-dto';
import { childrenOf, type MockNode, type MockTree } from './mock-tree';
import type { LodNode } from './types';

/** A node admitted into the result, with the children admitted below it so far. */
interface Picked {
  node: MockNode;
  depth: number;
  admitted: Picked[];
}

/** A node waiting to be admitted, with the admitted directory it hangs below. */
interface Offer {
  node: MockNode;
  parent: Picked;
}

/** Largest first, then the lower id: the order in which the backend admits nodes. */
const bySizeThenId = (a: MockNode, b: MockNode): number => b.allocated - a.allocated || a.id - b.id;

/** A max-heap of offers by `bySizeThenId`. */
class Offers {
  private readonly items: Offer[] = [];

  push(offer: Offer): void {
    const items = this.items;
    let index = items.length;

    items.push(offer);

    while (index > 0) {
      const above = (index - 1) >> 1;

      if (this.before(index, above)) {
        [items[index], items[above]] = [items[above] as Offer, items[index] as Offer];
        index = above;
      } else {
        break;
      }
    }
  }

  pop(): Offer | undefined {
    const items = this.items;
    const top = items[0];
    const last = items.pop();

    if (top === undefined || last === undefined) {
      return top;
    }

    if (items.length > 0) {
      items[0] = last;
      this.sinkDown();
    }

    return top;
  }

  private before(a: number, b: number): boolean {
    return bySizeThenId((this.items[a] as Offer).node, (this.items[b] as Offer).node) < 0;
  }

  private sinkDown(): void {
    const items = this.items;
    let index = 0;

    for (;;) {
      const left = index * 2 + 1;
      const right = left + 1;
      let first = index;

      if (left < items.length && this.before(left, first)) {
        first = left;
      }

      if (right < items.length && this.before(right, first)) {
        first = right;
      }

      if (first === index) {
        return;
      }

      [items[index], items[first]] = [items[first] as Offer, items[index] as Offer];
      index = first;
    }
  }
}

/**
 * A bounded view of the subtree under `rootNode`, admitted the way the backend does it: one queue of everything
 * that could be shown, largest first, seeded with the root's children and refilled with the children of every
 * admitted directory. A directory can therefore show only some of its children plus one `N smaller items` node,
 * and the node limit goes to whatever is largest anywhere. The placeholder counts toward `maxNodes`.
 */
export function lodOf(tree: MockTree, rootNode: MockNode, maxNodes: number, maxDepth: number): LodNode {
  const budget = Math.max(maxNodes, 1);
  const root: Picked = { node: rootNode, depth: 0, admitted: [] };
  const offers = new Offers();
  let used = 1;

  const offerChildrenOf = (parent: Picked): void => {
    if (parent.depth >= maxDepth) {
      return;
    }

    for (const node of childrenOf(tree, parent.node)) {
      offers.push({ node, parent });
    }
  };

  /** Nodes this admission adds: the node, plus the placeholder for the first child when siblings remain, minus it for the last. */
  const costOf = ({ node, admitted }: Picked): number => {
    const first = admitted.length === 0;
    const siblingsLeft = node.childCount - admitted.length > 1;

    if (first && siblingsLeft) {
      return 2;
    }

    return !first && !siblingsLeft ? 0 : 1;
  };

  offerChildrenOf(root);

  for (let offer = offers.pop(); offer; offer = offers.pop()) {
    const cost = costOf(offer.parent);

    if (used + cost > budget) {
      continue;
    }

    used += cost;

    const picked: Picked = { node: offer.node, depth: offer.parent.depth + 1, admitted: [] };

    offer.parent.admitted.push(picked);

    if (offer.node.kind === 'dir') {
      offerChildrenOf(picked);
    }
  }

  return build(tree, root);
}

function build(tree: MockTree, picked: Picked): LodNode {
  const admitted = [...picked.admitted].sort((a, b) => bySizeThenId(a.node, b.node));
  const children = admitted.map((child) => build(tree, child));

  if (admitted.length > 0 && admitted.length < picked.node.childCount) {
    const shown = new Set(admitted.map((child) => child.node.id));
    const hidden = childrenOf(tree, picked.node).filter((child) => !shown.has(child.id));

    children.push({ node: collapsedDto(hidden), children: [] });
  }

  return { node: toDto(picked.node), children };
}
