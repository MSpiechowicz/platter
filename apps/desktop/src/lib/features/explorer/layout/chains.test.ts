import { describe, expect, it } from 'vitest';
import { COLLAPSED_NODE_ID } from '../../../api/client';
import { leaf, lod, smaller, unread } from '../fixtures';
import {
  CHAIN_SHARE,
  chainsToExpand,
  collapseChains,
  dominantChild,
  lastOf,
  linksOf,
  MAX_CHAIN_LENGTH,
  spliceLod,
} from './chains';

const names = (items: readonly { node: { name: string } }[]): string[] => items.map((item) => item.node.name);

describe('collapseChains', () => {
  it('draws a folder that holds only one folder together with it: home / sam', () => {
    const root = lod('/', 100, [
      lod('home', 97, [lod('sam', 97, [leaf('a', 60), leaf('b', 37)])]),
      leaf('var', 3),
    ]);
    const [home, variable] = collapseChains(root).children as [
      ReturnType<typeof collapseChains>,
      ReturnType<typeof collapseChains>,
    ];

    expect(home.chain?.map((link) => link.name)).toEqual(['home', 'sam']);
    expect(home.node.name).toBe('home');
    expect(names(home.children)).toEqual(['a', 'b']);
    expect(variable.chain).toBeUndefined();
  });

  it('follows a chain of two and of three folders to the last one', () => {
    const deep = lod('/', 100, [
      lod('a', 100, [lod('b', 100, [lod('c', 100, [leaf('x', 70), leaf('y', 30)])])]),
    ]);
    const [tile] = collapseChains(deep).children;

    expect(tile?.chain?.map((link) => link.name)).toEqual(['a', 'b', 'c']);
    expect(lastOf(tile as NonNullable<typeof tile>).name).toBe('c');
    expect(names(tile?.children ?? [])).toEqual(['x', 'y']);
  });

  it('keeps what the folders on the way hold besides the next one, inside the tile', () => {
    const root = lod('/', 100, [
      lod('home', 100, [lod('me', 95, [leaf('a', 60), leaf('b', 35)]), lod('guest', 5, [leaf('g', 5)])]),
    ]);
    const [tile] = collapseChains(root).children;

    expect(tile?.chain?.map((link) => link.name)).toEqual(['home', 'me']);
    expect(names(tile?.children ?? [])).toEqual(['a', 'b', 'guest']);
    // The ones that belong to the last folder are known, so that the colours inside are the ones after opening it.
    expect([...(tile?.own ?? [])]).toHaveLength(2);
  });

  it('joins the "smaller items" nodes of the folders on the way into one', () => {
    const root = lod('/', 100, [
      lod('home', 100, [lod('me', 96, [leaf('a', 90), smaller(3, 6)]), smaller(2, 4)]),
    ]);
    const [tile] = collapseChains(root).children;
    const rest = tile?.children.filter((child) => child.node.id === COLLAPSED_NODE_ID) ?? [];

    expect(rest).toHaveLength(1);
    expect(rest[0]?.node.name).toBe('5 smaller items');
    expect(rest[0]?.node.allocated).toBe(10);
  });

  it(`makes a chain at ${CHAIN_SHARE * 100}% of the folder and not below it`, () => {
    const at = lod('/', 100, [lod('f', 100, [lod('c', 90, [leaf('x', 90)]), leaf('rest', 10)])]);
    const below = lod('/', 100, [lod('f', 100, [lod('c', 89, [leaf('x', 89)]), leaf('rest', 11)])]);

    expect(collapseChains(at).children[0]?.chain).toHaveLength(2);
    expect(collapseChains(below).children[0]?.chain).toBeUndefined();
    expect(dominantChild(lod('f', 100, [lod('c', 89.9, [leaf('x', 1)])]))).toBeUndefined();
  });

  it('stops at a folder with only files, and does not take a file for a folder', () => {
    const root = lod('/', 100, [lod('a', 100, [lod('b', 100, [leaf('one', 99), leaf('two', 1)])])]);
    const tile = collapseChains(root).children[0];

    expect(tile?.chain?.map((link) => link.name)).toEqual(['a', 'b']);
    expect(dominantChild(lod('b', 100, [leaf('one', 99)]))).toBeUndefined();
  });

  it('never makes a chain longer than the limit', () => {
    let inner = lod('end', 100, [leaf('x', 50), leaf('y', 50)]);

    for (let depth = 0; depth < MAX_CHAIN_LENGTH + 4; depth++) {
      inner = lod(`d${depth}`, 100, [inner]);
    }

    const tile = collapseChains(lod('/', 100, [inner])).children[0];

    expect(tile?.chain).toHaveLength(MAX_CHAIN_LENGTH);
  });

  it('collapses the chains below the top level too, so a folder inside shows what is inside it', () => {
    const root = lod('/', 100, [
      lod('top', 100, [lod('a', 60, [lod('b', 60, [leaf('x', 30), leaf('y', 30)])]), leaf('c', 40)]),
    ]);
    const inner = collapseChains(root).children[0]?.children[0];

    expect(inner?.chain?.map((link) => link.name)).toEqual(['a', 'b']);
  });

  it('is the identity, per tile, for a folder with no chains', () => {
    const root = lod('/', 100, [lod('a', 60, [leaf('x', 60)]), lod('b', 40, [leaf('y', 40)])]);
    const tiles = collapseChains(root).children;

    expect(tiles.map((tile) => tile.chain)).toEqual([undefined, undefined]);
    expect(names(tiles)).toEqual(['a', 'b']);
  });

  it('draws a chain whose last folder was not read as a plain tile of the whole size', () => {
    const root = lod('/', 100, [lod('a', 100, [unread('b', 100)])]);
    const tile = collapseChains(root).children[0];

    expect(tile?.chain).toHaveLength(2);
    expect(tile?.children).toEqual([]);
    expect(tile?.node.allocated).toBe(100);
  });
});

describe('what a chain tile stands for', () => {
  const tile = collapseChains(lod('/', 10, [lod('home', 10, [lod('me', 10, [leaf('a', 10)])])])).children[0];

  it('is its first folder for size and selection, and its last folder for opening', () => {
    expect(tile?.node.name).toBe('home');
    expect(lastOf(tile as NonNullable<typeof tile>).name).toBe('me');
    expect(linksOf(leaf('x', 1)).map((link) => link.name)).toEqual(['x']);
  });
});

describe('chainsToExpand', () => {
  it('asks for the last folder of a chain whose children were not read', () => {
    const root = lod('/', 100, [lod('a', 100, [unread('b', 100)])]);

    expect(chainsToExpand(collapseChains(root), root, new Set())).toEqual([
      root.children[0]?.children[0]?.node.id,
    ]);
  });

  it('asks for the biggest chain first, one level down as well, and not twice for the same folder', () => {
    const small = lod('s', 40, [unread('t', 40)]);
    const nested = lod('big', 60, [lod('n', 55, [unread('m', 55)]), leaf('z', 5)]);
    const root = lod('/', 100, [nested, small]);
    const model = collapseChains(root);
    const first = chainsToExpand(model, root, new Set());

    expect(first).toHaveLength(2);
    expect(first[0]).toBe(nested.children[0]?.children[0]?.node.id);
    expect(chainsToExpand(model, root, new Set([first[0] as number]))).toEqual([first[1]]);
  });

  it('asks for a deeper answer for the chain at the top when its folders have none of their own', () => {
    const root = lod('/', 100, [lod('home', 100, [lod('me', 100, [unread('a', 60), unread('b', 40)])])]);

    expect(chainsToExpand(collapseChains(root), root, new Set())).toEqual([
      root.children[0]?.children[0]?.node.id,
    ]);
  });

  it('asks for nothing when the chain is complete, or when there is none', () => {
    const complete = lod('/', 100, [lod('a', 100, [lod('b', 100, [leaf('x', 60), leaf('y', 40)])])]);
    const plain = lod('/', 100, [unread('a', 60), unread('b', 40)]);

    expect(chainsToExpand(collapseChains(complete), complete, new Set())).toEqual([]);
    expect(chainsToExpand(collapseChains(plain), plain, new Set())).toEqual([]);
  });
});

describe('spliceLod', () => {
  it('replaces the subtree of a folder with a deeper answer for it and leaves the rest as it was', () => {
    const stub = unread('b', 100);
    const root = lod('/', 100, [lod('a', 100, [stub]), leaf('other', 1)]);
    const deeper = { node: stub.node, children: [leaf('x', 100)] };
    const spliced = spliceLod(root, deeper);

    expect(spliced.children[0]?.children[0]).toBe(deeper);
    expect(spliced.children[1]).toBe(root.children[1]);
    expect(spliceLod(root, { node: leaf('nobody', 1).node, children: [] })).toBe(root);
  });
});
