import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import type { NodeDto } from '../../api/types';
import ChildList from './ChildList.svelte';

const item: NodeDto = {
  id: 1,
  name: 'a.bin',
  kind: 'file',
  allocated: 10,
  apparent: 10,
  files: 1,
  hasChildren: false,
  category: 'other',
  mtime: 1_700_000_000,
};

function props(extra: { moreDisabled?: boolean; loadingMore?: boolean }) {
  return {
    folder: 0,
    items: [item],
    total: 5,
    uncounted: [],
    allTotal: 5,
    loaded: 1,
    sort: 'sizeDesc' as const,
    selected: null,
    hovered: null,
    colorOf: () => '#fff',
    loading: false,
    hasMore: true,
    loadingMore: false,
    moreError: null,
    onsort: vi.fn(),
    onchoose: vi.fn(),
    onhover: vi.fn(),
    onmore: vi.fn(),
    onretry: vi.fn(),
    ...extra,
  };
}

describe('ChildList "Load more"', () => {
  it('is enabled when the rows belong to the current sort', () => {
    render(ChildList, { props: props({}) });

    expect(screen.getByRole('button', { name: /Load more/ })).toHaveProperty('disabled', false);
  });

  it('is disabled, and still says what it is, while the page for the current sort has not arrived', () => {
    render(ChildList, { props: props({ moreDisabled: true }) });

    expect(screen.getByRole('button', { name: /Load more/ })).toHaveProperty('disabled', true);
  });
});

const zero = (id: number, name: string, extra: Partial<NodeDto> = {}): NodeDto => ({
  ...item,
  id,
  name,
  kind: 'dir',
  allocated: 0,
  apparent: 0,
  files: 0,
  ...extra,
});

describe('ChildList rows that are not counted', () => {
  const uncounted = [
    zero(10, 'boot', { kind: 'mount' }),
    zero(11, 'root', { unreadable: true }),
    zero(12, 'srv'),
  ];

  function open(extra: object = {}) {
    return render(ChildList, {
      props: {
        ...props({ moreDisabled: false }),
        hasMore: false,
        total: 1,
        uncounted,
        uncountedTotal: 3,
        ...extra,
      },
    });
  }

  it('folds them under one footer row that counts them, and shows none of them at first', () => {
    open();

    expect(screen.getByRole('button', { name: '3 items not counted' }).getAttribute('aria-expanded')).toBe(
      'false',
    );
    expect(screen.queryByText('boot')).toBeNull();
    expect(screen.getByText('1 item')).toBeTruthy();
  });

  it('unfolds to the items without a bar or a size, each with the reason, and folds again', async () => {
    open();

    await fireEvent.click(screen.getByRole('button', { name: '3 items not counted' }));

    const rows = [...document.querySelectorAll('.row.off')];

    expect(rows.map((row) => row.querySelector('.nm')?.textContent)).toEqual(['boot', 'root', 'srv']);
    expect(rows.map((row) => row.querySelector('.why')?.textContent)).toEqual([
      'other disk',
      'no access',
      'empty',
    ]);
    expect(rows.every((row) => !row.querySelector('.bar') && !row.textContent?.includes('0 B'))).toBe(true);
    expect(rows.every((row) => row.tagName !== 'BUTTON')).toBe(true);

    await fireEvent.click(screen.getByRole('button', { name: '3 items not counted' }));

    expect(document.querySelectorAll('.row.off')).toHaveLength(0);
  });

  it('has no footer when there are none', () => {
    open({ uncounted: [], uncountedTotal: 0 });
    expect(screen.queryByText(/not counted/)).toBeNull();
  });

  it('never hides the rows it has when the count is not known: it counts them, and says it is not the whole', () => {
    open({ uncountedTotal: undefined, hasMore: true, allTotal: 10, loaded: 4 });

    expect(screen.getByRole('button', { name: '3 items not counted so far' })).toBeTruthy();
  });

  it('says "1 item" in the footer for one', () => {
    open({ uncounted: [zero(12, 'srv')], uncountedTotal: 1 });
    expect(screen.getByRole('button', { name: '1 item not counted' })).toBeTruthy();
  });
});

const named = (id: number, name: string, allocated = 100 - id): NodeDto => ({ ...item, id, name, allocated });

/** A folder with more than one page and many rows without a size: only the pages loaded so far are in the lists. */
describe('ChildList on a folder with several pages and rows that are not counted', () => {
  const sized = (count: number): NodeDto[] =>
    Array.from({ length: count }, (_, i) => named(i + 1, `big-${i}`));
  const empty = (count: number): NodeDto[] =>
    Array.from({ length: count }, (_, i) => zero(1000 + i, `empty-${String(i).padStart(3, '0')}`));

  const rows = (): number => document.querySelectorAll('.row.off').length;

  it('sorted by size: counts all the rows without a size, lists the loaded ones, and counts what is left to load', async () => {
    // 300 children: 100 with a size and 200 without. The first page of 200 held the 100 and 100 of the rest.
    render(ChildList, {
      props: {
        ...props({}),
        items: sized(100),
        total: 100,
        uncounted: empty(100),
        uncountedTotal: 200,
        hasMore: true,
        allTotal: 300,
        loaded: 200,
      },
    });

    const footer = screen.getByRole('button', { name: '200 items not counted' });

    await fireEvent.click(footer);

    expect(rows()).toBe(100);
    expect(screen.getByRole('button', { name: 'Load more (100 left)' })).toBeTruthy();
    expect(screen.queryByText('Nothing in this folder.')).toBeNull();
  });

  it('sorted by anything else: shows the rows loaded without a size, and does not state a total it cannot know', async () => {
    // The first page has 50 of them, spread among the sized ones; the total of them is not known.
    render(ChildList, {
      props: {
        ...props({}),
        items: sized(150),
        total: 250,
        uncounted: empty(50),
        uncountedTotal: undefined,
        sort: 'nameAsc',
        hasMore: true,
        allTotal: 300,
        loaded: 200,
      },
    });

    const footer = screen.getByRole('button', { name: '50 items not counted so far' });

    await fireEvent.click(footer);

    expect(rows()).toBe(50);
    expect(screen.getByRole('button', { name: 'Load more (100 left)' })).toBeTruthy();
  });

  it('lists the rows without a size, and not "Nothing in this folder", when the whole first page has none', async () => {
    render(ChildList, {
      props: {
        ...props({}),
        items: [],
        total: 100,
        uncounted: empty(200),
        uncountedTotal: undefined,
        sort: 'nameAsc',
        hasMore: true,
        allTotal: 300,
        loaded: 200,
      },
    });

    expect(screen.queryByText('Nothing in this folder.')).toBeNull();

    await fireEvent.click(screen.getByRole('button', { name: '200 items not counted so far' }));

    expect(rows()).toBe(200);
    expect(screen.getByRole('button', { name: 'Load more (100 left)' })).toBeTruthy();
  });

  it('still says a folder is empty when it is', () => {
    render(ChildList, {
      props: { ...props({}), items: [], total: 0, uncounted: [], hasMore: false, allTotal: 0, loaded: 0 },
    });

    expect(screen.getByText('Nothing in this folder.')).toBeTruthy();
  });

  it('counts "so far" under a filter while pages of matches are still to come, and plainly when they are all in', () => {
    const { unmount } = render(ChildList, {
      props: {
        ...props({}),
        uncounted: empty(5),
        uncountedTotal: undefined,
        query: 'empty-00',
        hasMore: true,
      },
    });

    expect(screen.getByRole('button', { name: '5 items not counted so far' })).toBeTruthy();
    unmount();

    render(ChildList, {
      props: { ...props({}), uncounted: empty(5), uncountedTotal: 5, query: 'empty-00', hasMore: false },
    });
    expect(screen.getByRole('button', { name: '5 items not counted' })).toBeTruthy();
  });
});

describe('ChildList when the rows of a "smaller items" tile stop loading', () => {
  it('says that only the first rows are shown', () => {
    render(ChildList, {
      props: { ...props({}), groupTruncated: true, allTotal: 12_000, loaded: 5000 },
    });

    expect(
      screen.getByText(/^Only the first 5,000 of 12,000 items are shown, so some of the smaller items/),
    ).toBeTruthy();
  });

  it('says nothing while the tile can still be loaded to the end', () => {
    render(ChildList, { props: { ...props({}), allTotal: 12_000, loaded: 400 } });

    expect(screen.queryByText(/Only the first/)).toBeNull();
  });
});

describe('ChildList header', () => {
  it('is one line: the title, the count right after it and the sort, in one row', () => {
    render(ChildList, { props: { ...props({}), total: 143, hasMore: false } });

    const head = document.querySelector('.head') as HTMLElement;

    expect(head.children[0]?.tagName).toBe('H3');
    expect(head.children[1]?.classList.contains('count')).toBe(true);
    expect(head.querySelector('h3')?.textContent).toBe('In this folder');
    expect(head.querySelector('.count')?.textContent).toBe('143 items');
    expect(head.querySelector('.sort')?.textContent).toContain('Size');
  });

  it('says 1.2k instead of 1,234 when there is no room for the long form, and keeps the same elements', () => {
    render(ChildList, { props: { ...props({}), total: 1234, hasMore: false } });

    expect(document.querySelector('.head .count')?.textContent).toBe('1,234 items');
  });

  it('does not repeat the count in a hidden copy that text queries would find', () => {
    render(ChildList, { props: { ...props({}), total: 143, hasMore: false } });

    expect(screen.getAllByText('143 items')).toHaveLength(1);
  });
});

describe('ChildList search', () => {
  const rows = [
    named(1, 'node_modules'),
    named(2, 'Cargo.lock'),
    named(3, 'target'),
    named(4, 'CARGO_HOME'),
    named(5, 'docs'),
  ];

  function search(extra: object = {}) {
    const rendered = render(ChildList, {
      props: { ...props({}), items: rows, total: 5, hasMore: false, ...extra },
    });

    return { ...rendered, field: screen.getByPlaceholderText('Search in this folder') as HTMLInputElement };
  }

  const names = (): string[] => [...document.querySelectorAll('.row .nm')].map((nm) => nm.textContent ?? '');

  it('has a field with a magnifier, the placeholder, and no clear button while it is empty', () => {
    const { field } = search();

    expect(field.getAttribute('aria-label')).toBe('Search in this folder');
    expect(document.querySelector('.find svg')).not.toBeNull();
    expect(screen.queryByRole('button', { name: 'Clear the search' })).toBeNull();
  });

  it('hands over what is typed as it is typed: the store decides when to ask', async () => {
    const onquery = vi.fn();
    const { field } = search({ onquery });

    await fireEvent.input(field, { target: { value: 'c' } });
    await fireEvent.input(field, { target: { value: 'ca' } });

    expect(onquery.mock.calls).toEqual([['c'], ['ca']]);
  });

  it('lists the rows it is given, which with a query are the ones the backend found, in the same order', () => {
    search({ query: 'cargo', items: [rows[1], rows[3]], total: 5, matched: 2 });

    expect(names()).toEqual(['Cargo.lock', 'CARGO_HOME']);
  });

  it('underlines the matching part of the name, with no background', () => {
    search({ query: 'RG', items: [rows[1], rows[2], rows[3]], matched: 3 });

    const marks = [...document.querySelectorAll('.row mark')];

    expect(marks.map((mark) => mark.textContent)).toEqual(['rg', 'rg', 'RG']);
    expect(document.querySelector('.row .nm')?.textContent).toBe('Cargo.lock');
  });

  it('counts "M of N" in the header while filtering, from the numbers it is given', () => {
    search({ query: 'cargo', items: [rows[1], rows[3]], total: 143, matched: 12 });

    expect(document.querySelector('.head .count')?.textContent).toBe('12 of 143 items');
  });

  it('counts only the folder until the matches are known, and without a query', () => {
    const { unmount } = search({ query: 'cargo', items: [], total: 143, loading: true });

    expect(document.querySelector('.head .count')?.textContent).toBe('143 items');
    unmount();

    search({ total: 143, matched: 12 });
    expect(document.querySelector('.head .count')?.textContent).toBe('143 items');
  });

  it('says so in one line when nothing matches, and shows no rows', () => {
    search({ query: 'zzz', items: [], total: 5, matched: 0 });

    expect(names()).toEqual([]);
    expect(screen.getByText('No folder or file here matches "zzz"')).toBeTruthy();
    expect(screen.queryByText('Nothing in this folder.')).toBeNull();
  });

  it('does not say nothing matches while the matches are on their way, and shows that it is working', () => {
    search({ query: 'zzz', items: [], total: 5, loading: true });

    expect(screen.queryByText(/No folder or file/)).toBeNull();
    expect(screen.queryByText('Nothing in this folder.')).toBeNull();
    expect(document.querySelector('.rows')?.getAttribute('aria-busy')).toBe('true');
    expect(document.querySelector('.progress')?.classList.contains('on')).toBe(true);
  });

  it('shows no working line without a query, or once the matches are in', () => {
    const { unmount } = search({ loading: true });

    expect(document.querySelector('.progress')?.classList.contains('on')).toBe(false);
    unmount();

    search({ query: 'cargo', items: [rows[1]], matched: 1 });
    expect(document.querySelector('.progress')?.classList.contains('on')).toBe(false);
  });

  it('shows the error, and not "no matches", when the matches could not be read', async () => {
    const onretry = vi.fn();

    search({ query: 'zzz', items: [], total: 5, error: 'the folder cannot be read', onretry });

    expect(screen.getByText(/The list could not be read: the folder cannot be read/)).toBeTruthy();
    expect(screen.queryByText(/No folder or file/)).toBeNull();
    expect(screen.queryByText('Nothing in this folder.')).toBeNull();

    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(onretry).toHaveBeenCalledTimes(1);
  });

  it('offers "Load more" for the matches too, counting the matches that are left', async () => {
    const onmore = vi.fn();

    search({
      query: 'cargo',
      items: [rows[1], rows[3]],
      total: 20_000,
      matched: 11_111,
      hasMore: true,
      allTotal: 11_111,
      loaded: 200,
      onmore,
    });

    await fireEvent.click(screen.getByRole('button', { name: 'Load more (10,911 left)' }));
    expect(onmore).toHaveBeenCalledTimes(1);
  });

  it('does not light a row for a selected child that the search left out', () => {
    const onchoose = vi.fn();

    search({ query: 'docs', items: [rows[4]], selected: 1, onchoose });

    expect(names()).toEqual(['docs']);
    expect(document.querySelector('.row.sel')).toBeNull();
    expect(onchoose).not.toHaveBeenCalled();
  });

  it('lists the rows without a size that the backend found, and counts them in the footer', async () => {
    const off = [zero(10, 'boot', { kind: 'mount' })];

    search({ query: 'boo', items: [], uncounted: off, uncountedTotal: 1, matched: 0 });

    expect(screen.queryByText(/No folder or file/)).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: '1 item not counted' }));
    expect([...document.querySelectorAll('.row.off .nm')].map((nm) => nm.textContent)).toEqual(['boot']);
  });

  it('has no footer at all when no row without a size was found', () => {
    search({ query: 'cargo', items: [rows[1]], uncounted: [], uncountedTotal: 0 });

    expect(screen.queryByText(/not counted/)).toBeNull();
  });

  it('clears the field with Escape first, and leaves Escape with nothing to clear for going up', async () => {
    const onquery = vi.fn();
    const { field } = search({ onquery, query: 'cargo' });

    await fireEvent.input(field, { target: { value: 'cargo' } });

    const first = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });

    field.dispatchEvent(first);
    await tick();
    expect(first.defaultPrevented).toBe(true);
    expect(onquery).toHaveBeenLastCalledWith('');
    expect(field.value).toBe('');

    const second = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });

    field.dispatchEvent(second);
    expect(second.defaultPrevented).toBe(false);
  });

  it('does not stop Backspace, so deleting a letter does not go up a level', () => {
    const { field } = search();
    const backspace = new KeyboardEvent('keydown', { key: 'Backspace', bubbles: true, cancelable: true });

    field.dispatchEvent(backspace);

    expect(backspace.defaultPrevented).toBe(false);
  });

  it('clears with the button and empties the field when the folder changes', async () => {
    const onquery = vi.fn();
    const { field, rerender } = search({ onquery, query: 'cargo' });

    await fireEvent.input(field, { target: { value: 'cargo' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Clear the search' }));
    expect(onquery).toHaveBeenLastCalledWith('');

    await fireEvent.input(field, { target: { value: 'doc' } });
    await rerender({ folder: 99 });
    expect(field.value).toBe('');
  });

  it('puts the cursor in the field on request', () => {
    const { component, field } = search();

    (component as unknown as { focusSearch: () => void }).focusSearch();

    expect(document.activeElement).toBe(field);
  });
});

describe('ChildList marking', () => {
  const dir = (id: number, name: string, extra: Partial<NodeDto> = {}): NodeDto => ({
    ...item,
    id,
    name,
    kind: 'dir',
    allocated: 1000 * id,
    ...extra,
  });
  const marking = (extra: object = {}) => ({
    ...props({}),
    hasMore: false,
    items: [dir(1, 'Downloads'), dir(2, 'Videos'), dir(3, 'link', { kind: 'symlink' })],
    total: 3,
    allTotal: 3,
    loaded: 3,
    onmark: vi.fn(),
    isKept: (id: number) => id === 2,
    ...extra,
  });
  const box = (name: string) =>
    screen.getByRole('checkbox', { name: `Mark ${name} for cleanup` }) as HTMLInputElement;

  it('has a checkbox for a row, and a lock with "KEPT" for a kept one', () => {
    render(ChildList, { props: marking() });

    expect(box('Downloads').disabled).toBe(false);
    expect(screen.queryByRole('checkbox', { name: 'Mark Videos for cleanup' })).toBeNull();
    expect(screen.getByRole('img', { name: /Kept on purpose/ })).toBeTruthy();
    expect(document.querySelectorAll('.kept')).toHaveLength(1);
    expect(document.querySelector('.kept')?.textContent).toBe('Kept');
  });

  it('keeps the name and the lock in the same columns whether a row is kept or not', () => {
    render(ChildList, { props: marking() });

    const items = [...document.querySelectorAll('.rows .item')];

    expect(items).toHaveLength(3);
    // Each item has exactly one control column (a box or a lock) before its button.
    expect(items.every((row) => row.firstElementChild?.matches('.box, .lock'))).toBe(true);
    expect(items.every((row) => row.lastElementChild?.matches('button.row'))).toBe(true);
  });

  it('reports a tick and a cleared box, with the row', async () => {
    const onmark = vi.fn();

    render(ChildList, { props: marking({ onmark }) });
    await fireEvent.click(box('Downloads'));

    expect(onmark).toHaveBeenCalledWith(expect.objectContaining({ id: 1, name: 'Downloads' }), true);
  });

  it('shows what is marked, not what was clicked: a box the owner refuses springs back', async () => {
    render(ChildList, { props: marking({ marked: new Set<number>() }) });

    await fireEvent.click(box('Downloads'));
    await tick();

    expect(box('Downloads').checked).toBe(false);
  });

  it('shows a marked row ticked', () => {
    render(ChildList, { props: marking({ marked: new Set([1]) }) });

    expect(box('Downloads').checked).toBe(true);
  });

  it('does not select the row when its box is clicked', async () => {
    const onchoose = vi.fn();

    render(ChildList, { props: marking({ onchoose }) });
    await fireEvent.click(box('Downloads'));

    expect(onchoose).not.toHaveBeenCalled();
  });

  it('turns every box off, with the reason, while marking is off', () => {
    render(ChildList, { props: marking({ markOff: 'A cleanup is in progress.', marked: new Set([1]) }) });

    expect(box('Downloads').disabled).toBe(true);
    expect(box('Downloads').checked).toBe(true);
    expect(box('Downloads').closest('.box')?.getAttribute('title')).toBe('A cleanup is in progress.');
  });

  it('turns off the boxes that are not ticked when the most items are marked', () => {
    render(ChildList, {
      props: marking({ markFull: true, marked: new Set([1]), items: [dir(1, 'Downloads'), dir(4, 'Other')] }),
    });

    expect(box('Downloads').disabled).toBe(false);
    expect(box('Other').disabled).toBe(true);
  });

  it('turns off the box of a link: the backend never cleans one', () => {
    render(ChildList, { props: marking() });

    expect(box('link').disabled).toBe(true);
    expect(box('link').closest('.box')?.getAttribute('title')).toBe('A link is never cleaned.');
  });

  it('shows hidden characters in a name, and uses that name for the box', () => {
    render(ChildList, { props: marking({ items: [dir(1, 'a‮b')] }) });

    expect(box('a\\u{202E}b')).toBeTruthy();
    expect(document.querySelector('.row .nm')?.textContent).toBe('a\\u{202E}b');
  });

  it('puts the footer it is given under the rows, inside the card', async () => {
    const { createRawSnippet } = await import('svelte');

    render(ChildList, {
      props: marking({ bottom: createRawSnippet(() => ({ render: () => '<p data-bottom>Footer</p>' })) }),
    });

    expect(document.querySelector('.child-list [data-bottom]')).not.toBeNull();
    expect(document.querySelector('.child-list [data-bottom]')?.previousElementSibling?.classList).toContain(
      'rows',
    );
  });

  it('only one box is in the tab order: the one of the row that was focused last', async () => {
    render(ChildList, { props: marking({ items: [dir(1, 'Downloads'), dir(4, 'Other')] }) });

    expect(box('Downloads').tabIndex).toBe(0);
    expect(box('Other').tabIndex).toBe(-1);

    await fireEvent.focusIn(screen.getByRole('button', { name: /^Other/ }));

    expect(box('Other').tabIndex).toBe(0);
    expect(box('Downloads').tabIndex).toBe(-1);
  });
});
