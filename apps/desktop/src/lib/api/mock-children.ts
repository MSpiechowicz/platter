import type { MockNode } from './mock-tree';
import type { SortKey } from './types';

const encoder = new TextEncoder();

/**
 * Names in the backend's order (`children.rs`, `name_order`): the UTF-8 bytes of the name with only ASCII letters
 * lowercased, compared byte by byte. A character outside ASCII keeps its case, and a longer name follows its prefix.
 * This is not JavaScript's string order, which compares UTF-16 units and puts an emoji before U+FF21.
 */
function nameOrder(a: string, b: string): number {
  const left = encoder.encode(a);
  const right = encoder.encode(b);
  const common = Math.min(left.length, right.length);

  for (let index = 0; index < common; index++) {
    const difference = asciiLower(left[index] as number) - asciiLower(right[index] as number);

    if (difference !== 0) {
      return difference;
    }
  }

  return left.length - right.length;
}

function asciiLower(byte: number): number {
  return byte >= 0x41 && byte <= 0x5a ? byte + 0x20 : byte;
}

/** Orders by `sort`, then by name as above, then by id, so paging is stable. */
export function sorter(sort: SortKey): (a: MockNode, b: MockNode) => number {
  const byName = (a: MockNode, b: MockNode): number => nameOrder(a.name, b.name) || a.id - b.id;

  switch (sort) {
    case 'sizeDesc':
      return (a, b) => b.allocated - a.allocated || byName(a, b);
    case 'sizeAsc':
      return (a, b) => a.allocated - b.allocated || byName(a, b);
    case 'nameAsc':
      return byName;
    case 'filesDesc':
      return (a, b) => b.files - a.files || byName(a, b);
    case 'mtimeDesc':
      return (a, b) => b.mtime - a.mtime || byName(a, b);
  }
}

/**
 * The name filter (`children.rs`, `NameFilter`): a blank text filters nothing, otherwise a child matches when its
 * name contains the text, ignoring case. Both sides are lowercased in full Unicode, so `ŻÓŁ` finds `zażółć`.
 */
export function nameFilter(query: string | undefined): ((node: MockNode) => boolean) | undefined {
  if (query === undefined || query.trim() === '') {
    return undefined;
  }

  const needle = query.toLowerCase();

  return (node) => node.name.toLowerCase().includes(needle);
}
