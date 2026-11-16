import { describe, expect, it } from 'vitest';
import { COLLAPSED_NODE_ID } from './client';
import { lodOf } from './mock-lod';
import { must, nodeAt, rootOf, sketchTree } from './mock-sketch';
import type { LodNode } from './types';

// The scenarios of the backend's tests of `lod` (`crates/platter-core/src/query/tests.rs`).

function lodFs() {
  const files: Record<string, number> = {};

  for (let i = 0; i < 40; i++) {
    files[`/big/f${String(i).padStart(2, '0')}`] = 1000 + i;
  }

  for (let i = 0; i < 30; i++) {
    files[`/small/f${String(i).padStart(2, '0')}`] = 1 + i;
  }

  files['/medium/a/b/c/d/deep.bin'] = 5000;

  return sketchTree({ files });
}

const count = (lod: LodNode): number => 1 + lod.children.reduce((sum, child) => sum + count(child), 0);
const depth = (lod: LodNode): number => Math.max(0, ...lod.children.map((child) => 1 + depth(child)));

describe('lodOf', () => {
  it('respects maxNodes and merges the rest of a directory into one node', () => {
    const tree = lodFs();
    const result = lodOf(tree, rootOf(tree), 20, 10);

    expect(count(result)).toBe(20);

    const big = must(result.children.find((child) => child.node.name === 'big'));
    const collapsed = must(big.children.at(-1));
    const shown = big.children.slice(0, -1).reduce((sum, child) => sum + child.node.allocated, 0);
    const omitted = 40 - (big.children.length - 1);

    expect(collapsed.node).toMatchObject({
      id: COLLAPSED_NODE_ID,
      kind: 'collapsed',
      name: `${omitted} smaller items`,
      files: omitted,
      unreadable: false,
      hasChildren: false,
    });
    expect(collapsed.children).toEqual([]);
    expect(shown + collapsed.node.allocated).toBe(big.node.allocated);
  });

  it('spends the budget on the largest nodes anywhere, so a small directory loses out to the files of a big one', () => {
    const tree = lodFs();
    const result = lodOf(tree, rootOf(tree), 12, 10);
    const byName = (name: string) => result.children.find((child) => child.node.name === name);

    expect(must(byName('big')).children.length).toBeGreaterThan(0);
    expect(byName('small')).toBeUndefined();

    // The one directory left out is "1 smaller item", singular, and it carries all of its bytes.
    const collapsed = must(result.children.at(-1));

    expect(collapsed.node).toMatchObject({ kind: 'collapsed', name: '1 smaller item', allocated: 465 });
    expect(count(result)).toBe(12);
  });

  it('lists children largest first and the collapsed node last', () => {
    const tree = lodFs();
    const result = lodOf(tree, rootOf(tree), 15, 10);

    for (const node of [result, ...result.children]) {
      const sizes = node.children
        .filter((child) => child.node.kind !== 'collapsed')
        .map((child) => child.node.allocated);

      expect(sizes).toEqual([...sizes].sort((a, b) => b - a));
    }
  });

  it('stops at maxDepth but still says a cut-off directory has children', () => {
    const tree = lodFs();
    const shallow = lodOf(tree, rootOf(tree), 1000, 2);
    const medium = must(shallow.children.find((child) => child.node.name === 'medium'));

    expect(depth(shallow)).toBe(2);
    expect(must(medium.children[0]).node.name).toBe('a');
    expect(must(medium.children[0]).children).toEqual([]);
    expect(must(medium.children[0]).node.hasChildren).toBe(true);

    const full = lodOf(tree, rootOf(tree), 1000, 20);

    expect(depth(full)).toBe(6);
    expect(count(full)).toBe(tree.nodes.length);
  });

  it('returns the root for a tiny budget, and no children at depth 0', () => {
    const tree = lodFs();

    expect(count(lodOf(tree, rootOf(tree), 1, 5))).toBe(1);
    expect(lodOf(tree, rootOf(tree), 0, 5).children).toEqual([]);
    expect(lodOf(tree, rootOf(tree), 100, 0).children).toEqual([]);
  });

  it('starts at the node it is asked about', () => {
    const tree = lodFs();
    const result = lodOf(tree, nodeAt(tree, '/medium'), 100, 10);

    expect(result.node.name).toBe('medium');
    expect(count(result)).toBe(6);
  });

  it('shows a directory with only some of its children plus the merged rest', () => {
    const tree = lodFs();
    const result = lodOf(tree, rootOf(tree), 30, 10);
    const big = must(result.children.find((child) => child.node.name === 'big'));

    expect(big.children.filter((child) => child.node.kind !== 'collapsed').length).toBeGreaterThan(5);
    expect(must(big.children.at(-1)).node.kind).toBe('collapsed');
  });
});
