import { describe, expect, it } from 'vitest';
import { COLLAPSED_NODE_ID } from '../../api/client';
import type { NodeDto } from '../../api/types';
import { collapseChains } from './layout/chains';
import { leaf, lod, node, smaller } from './fixtures';
import {
  FULL_TEXT,
  itemOff,
  KEPT_TEXT,
  markButton,
  markingOff,
  markOf,
  markTarget,
  safeTops,
  tileMarked,
  type Blockers,
} from './marking';

const ON: Blockers = { busy: false, outdated: false, loading: false, full: false };

describe('markingOff', () => {
  it('says why no mark can change, in the order of what matters most', () => {
    expect(markingOff(ON)).toBeUndefined();
    expect(markingOff({ ...ON, busy: true, outdated: true })).toBe('A cleanup is in progress.');
    expect(markingOff({ ...ON, outdated: true, loading: true })).toBe(
      'The disk changed. Rescan to mark again.',
    );
    expect(markingOff({ ...ON, loading: true })).toBe('The folder is still loading.');
  });
});

describe('itemOff', () => {
  it('rules out what the backend always refuses: links, mounts, unreadable items, odd kinds', () => {
    expect(itemOff(node('a', 5, { kind: 'symlink' }))).toBe('A link is never cleaned.');
    expect(itemOff(node('a', 5, { kind: 'mount' }))).toMatch(/Another disk/);
    expect(itemOff(node('a', 5, { kind: 'other' }))).toMatch(/Only files and folders/);
    expect(itemOff(node('a', 5, { unreadable: true }))).toMatch(/could not be read/);
  });

  it('lets files and folders through', () => {
    expect(itemOff(node('a', 5))).toBeUndefined();
    expect(itemOff(node('a', 5, { kind: 'file' }))).toBeUndefined();
  });
});

describe('markTarget', () => {
  const open = node('demo', 100);
  const child = node('cache', 40);
  const crumbs = [1, 2, open.id];

  it('is the open folder, with the folders above it, when nothing is selected', () => {
    expect(markTarget(open, crumbs, undefined)).toEqual({ node: open, ancestors: [1, 2] });
  });

  it('is a selected child, with the open folder among its ancestors', () => {
    expect(markTarget(child, crumbs, { links: [child] })).toEqual({ node: child, ancestors: crumbs });
  });

  it('is the last folder of a chain tile, with the folders of the chain above it', () => {
    const first = node('home', 100);
    const last = node('sam', 100);

    expect(markTarget(last, crumbs, { links: [first, last] })).toEqual({
      node: last,
      ancestors: [...crumbs, first.id],
    });
  });

  it('is nothing without a node', () => {
    expect(markTarget(undefined, crumbs, undefined)).toBeUndefined();
  });
});

describe('markButton', () => {
  const target = { node: node('cache', 40), ancestors: [0, 3] };
  const context = { groupSelected: false, marked: false, kept: false, blockers: ON };

  it('is on for an ordinary folder and says what a click does', () => {
    expect(markButton(target, context)).toEqual({ label: 'Mark for cleanup', off: undefined });
    expect(markButton(target, { ...context, marked: true })).toEqual({ label: 'Unmark', off: undefined });
  });

  it('is off at the scan root', () => {
    expect(markButton({ node: node('root', 9), ancestors: [] }, context).off).toMatch(
      /scanned folder itself/,
    );
  });

  it('is off for a kept item, with the reason', () => {
    expect(markButton(target, { ...context, kept: true }).off).toBe(KEPT_TEXT);
  });

  it('is off for the "smaller items" tile, and for nothing', () => {
    expect(markButton(target, { ...context, groupSelected: true }).off).toMatch(/several small items/);
    expect(markButton(undefined, context).off).toBe('Nothing to mark.');
  });

  it('is off while a cleanup runs, and once the scan is outdated, even for what is marked', () => {
    expect(markButton(target, { ...context, blockers: { ...ON, busy: true } }).off).toBe(
      'A cleanup is in progress.',
    );
    expect(markButton(target, { ...context, marked: true, blockers: { ...ON, outdated: true } })).toEqual({
      label: 'Unmark',
      off: 'The disk changed. Rescan to mark again.',
    });
  });

  it('is off for a link, but an item that is marked can still be unmarked when the list is full', () => {
    expect(markButton({ node: node('l', 1, { kind: 'symlink' }), ancestors: [0] }, context).off).toBe(
      'A link is never cleaned.',
    );
    expect(markButton(target, { ...context, blockers: { ...ON, full: true } }).off).toBe(FULL_TEXT);
    expect(
      markButton(target, { ...context, marked: true, blockers: { ...ON, full: true } }).off,
    ).toBeUndefined();
  });
});

describe('markOf', () => {
  it('keeps what is needed to total and nest a mark, and copies the ancestry', () => {
    const above = [0, 1];
    const mark = markOf(node('a', 7), above);

    above.push(9);

    expect(mark).toMatchObject({ allocated: 7, ancestors: [0, 1] });
  });
});

describe('safeTops', () => {
  const build = () => {
    const target = lod('target', 50, [leaf('a', 30), leaf('b', 20)]);
    const project = lod('project', 100, [target, leaf('src', 50)]);
    const other = lod('other', 40, [leaf('x', 40)]);
    const focus = lod('focus', 200, [project, other, smaller(3, 10)]);

    return { model: collapseChains(focus), target, project, other };
  };

  it('is the topmost safe tile: the folder a safe rule lists, not the folders around it', () => {
    const { model, target, project } = build();
    const tops = safeTops(model, new Set([target.node.id]), false);

    expect([...tops]).toEqual([target.node.id]);
    expect(tops.has(project.node.id)).toBe(false);
  });

  it('draws a safe folder once: what is inside it is not outlined again', () => {
    const { model, target } = build();
    const inner = target.children[0] as { node: NodeDto };

    expect([...safeTops(model, new Set([target.node.id, inner.node.id]), false)]).toEqual([target.node.id]);
  });

  it('is empty when nothing is safe, when the open folder is inside a safe one, and without a model', () => {
    const { model, target } = build();

    expect(safeTops(model, new Set(), false).size).toBe(0);
    expect(safeTops(model, new Set([target.node.id]), true).size).toBe(0);
    expect(safeTops(undefined, new Set([target.node.id]), false).size).toBe(0);
  });

  it('never marks the "smaller items" tile', () => {
    const { model } = build();

    expect(safeTops(model, new Set([COLLAPSED_NODE_ID]), false).size).toBe(0);
  });

  it('counts a chain by its first folder only: what the rest of the chain holds is not claimed', () => {
    const last = lod('inner', 95, [leaf('f', 95)]);
    const first = lod('outer', 100, [last, leaf('small', 5)]);
    const model = collapseChains(lod('focus', 100, [first]));
    const tile = model.children[0];

    expect(tile?.chain).toHaveLength(2);
    expect(safeTops(model, new Set([last.node.id]), false).size).toBe(0);
    expect([...safeTops(model, new Set([first.node.id]), false)]).toEqual([first.node.id]);
  });
});

describe('tileMarked', () => {
  const first = node('home', 100);
  const last = node('sam', 100);

  it('is true when the folder, or any folder of a chain tile, is marked', () => {
    expect(tileMarked({ node: first }, new Set([first.id]))).toBe(true);
    expect(tileMarked({ node: first, chain: [first, last] }, new Set([last.id]))).toBe(true);
    expect(tileMarked({ node: first, chain: [first, last] }, new Set([999]))).toBe(false);
    expect(tileMarked({ node: first }, new Set())).toBe(false);
  });
});
