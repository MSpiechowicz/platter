import { describe, expect, it } from 'vitest';
import { nameFilter, sorter } from './mock-children';
import { rootOf, sketchTree } from './mock-sketch';
import { childrenOf } from './mock-tree';

// The scenarios of the backend's tests of the child order and the name filter (`crates/platter-core/src/query/children.rs`
// and `tests.rs`), with names beyond ASCII.

const NAMES = ['Żółw', 'éa', 'Ä', 'z', '\u{1F600}', 'file10', 'file2', 'Ａ', 'B', 'a', 'A'];

function folder() {
  const tree = sketchTree({ files: Object.fromEntries(NAMES.map((name) => [`/${name}`, 10])) });

  return childrenOf(tree, rootOf(tree));
}

describe('sorter', () => {
  it('orders names by the UTF-8 bytes with only ASCII letters lowercased, then by id', () => {
    const names = folder()
      .sort(sorter('nameAsc'))
      .map((node) => node.name);

    // `A` and `a` are equal once lowercased, so the one that came first (lower id) leads. `Ä` (C3 84) comes before
    // `éa` (C3 A9) and `Żółw` (C5 BB): non-ASCII letters keep their case and their byte value. U+FF21 (EF BC A1) comes
    // before the emoji (F0 9F 98 80) as bytes, although UTF-16 puts the emoji (D83D) first.
    expect(names).toEqual(['a', 'A', 'B', 'file10', 'file2', 'z', 'Ä', 'éa', 'Żółw', 'Ａ', '\u{1F600}']);
  });

  it('breaks the ties of the other keys by that name order', () => {
    const nodes = folder();

    for (const key of ['sizeDesc', 'sizeAsc', 'filesDesc', 'mtimeDesc'] as const) {
      expect([...nodes].sort(sorter(key)).map((node) => node.name)).toEqual(
        [...nodes].sort(sorter('nameAsc')).map((node) => node.name),
      );
    }
  });
});

describe('nameFilter', () => {
  const matching = (query: string | undefined): string[] =>
    folder()
      .filter(nameFilter(query) ?? (() => true))
      .map((node) => node.name);

  it('matches ignoring case beyond ASCII', () => {
    expect(matching('ŻÓŁ')).toEqual(['Żółw']);
    expect(matching('żółw')).toEqual(['Żółw']);
    expect(matching('ä')).toEqual(['Ä']);
    expect(matching('ÉA')).toEqual(['éa']);
    expect(matching('FILE')).toEqual(['file10', 'file2']);
  });

  it('does not match a letter against its unaccented form, and finds an emoji', () => {
    expect(matching('zolw')).toEqual([]);
    expect(matching('\u{1F600}')).toEqual(['\u{1F600}']);
  });

  it('filters nothing for a blank text', () => {
    for (const query of [undefined, '', '   ']) {
      expect(matching(query)).toHaveLength(NAMES.length);
    }
  });
});
