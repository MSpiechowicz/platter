import { describe, expect, it } from 'vitest';
import { advisorNodes, suggestionsOf, type MockLabel } from './mock-suggestions';
import { buildTree, childrenOf, type MockNode, type MockTree } from './mock-tree';

const NOW = 1_750_000_000;

function treeOf(root: string): MockTree {
  return buildTree({ seed: 20240607, root, oneFileSystem: true, bytes: 186_000_000_000, now: NOW });
}

/** The node at a path below the scan root, by names. */
function at(tree: MockTree, path: string): MockNode {
  let node = tree.nodes[0] as MockNode;

  for (const name of path.split('/').filter((part) => part !== '')) {
    const next = childrenOf(tree, node).find((child) => child.name === name);

    expect(next, `${name} of ${path}`).toBeDefined();
    node = next as MockNode;
  }

  return node;
}

const review = (tree: MockTree, path: string): MockLabel => ({
  node: at(tree, path).id,
  verdict: 'review',
  reason: 'A reason.',
});

describe('the advisor group of the mock suggestions', () => {
  it('leaves out labels that a rule covers, that hold a rule match, or that say keep', () => {
    const tree = treeOf('/home/demo');
    const labels: MockLabel[] = [
      review(tree, '.cache'),
      review(tree, '.cache/google-chrome'),
      review(tree, '.local'),
      review(tree, '.local/share'),
      review(tree, 'Downloads'),
      { ...review(tree, 'Projekty/blog/src'), verdict: 'keep' },
      review(tree, 'Projekty/web-shop/src'),
    ];

    expect(advisorNodes(tree, labels).map((node) => node.id)).toEqual([at(tree, 'Projekty/web-shop/src').id]);

    const group = suggestionsOf(tree, labels).suggestions.find((entry) => entry.id === 'advisor');

    expect(group).toMatchObject({
      verdict: 'review',
      source: 'model',
      items: 1,
      path: '~/Projekty/web-shop/src',
      nodes: [at(tree, 'Projekty/web-shop/src').id],
    });
  });

  it('has no group when every label is left out', () => {
    const tree = treeOf('/home/demo');
    const { suggestions } = suggestionsOf(tree, [review(tree, '.cache'), review(tree, '.local')]);

    expect(suggestions.some((entry) => entry.id === 'advisor')).toBe(false);
    expect(suggestions.every((entry) => entry.source === 'rule')).toBe(true);
  });

  it('shows a glob for several folders and lists the largest first', () => {
    const tree = treeOf('/home/demo');
    const labels = ['Projekty/blog/src', 'Projekty/web-shop/src', 'Projekty/platter/src'].map((path) =>
      review(tree, path),
    );
    const group = suggestionsOf(tree, labels).suggestions.find((entry) => entry.id === 'advisor');
    const sizes = (group?.nodes ?? []).map((id) => (tree.nodes[id] as MockNode).allocated);

    expect(group?.items).toBe(3);
    expect(group?.path).toBe('~/Projekty/**');
    expect(sizes).toEqual([...sizes].sort((a, b) => b - a));
  });

  it('suggests nothing in a scan that starts inside a kept folder, labels or not', () => {
    const tree = treeOf('/home/demo/Videos/recordings');

    expect(suggestionsOf(tree, [{ node: 0, verdict: 'review', reason: 'A reason.' }])).toEqual({
      suggestions: [],
      totals: { safe: 0, review: 0, keep: 0 },
    });
  });

  it('ignores a label of a node the tree does not have', () => {
    const tree = treeOf('/home/demo');

    expect(advisorNodes(tree, [{ node: 9_999_999, verdict: 'review', reason: 'A reason.' }])).toEqual([]);
  });
});
