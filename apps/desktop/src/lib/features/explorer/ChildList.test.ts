import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
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

  it('has no footer when the count is not known yet or there are none', () => {
    open({ uncountedTotal: undefined });
    expect(screen.queryByText(/not counted/)).toBeNull();
  });

  it('says "1 item" in the footer for one', () => {
    open({ uncounted: [zero(12, 'srv')], uncountedTotal: 1 });
    expect(screen.getByRole('button', { name: '1 item not counted' })).toBeTruthy();
  });
});

const named = (id: number, name: string, allocated = 100 - id): NodeDto => ({ ...item, id, name, allocated });

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
  afterEach(() => {
    vi.useRealTimers();
  });

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

  it('applies what is typed after a short pause, once', async () => {
    vi.useFakeTimers();

    const onquery = vi.fn();
    const { field } = search({ onquery });

    await fireEvent.input(field, { target: { value: 'c' } });
    await fireEvent.input(field, { target: { value: 'ca' } });
    vi.advanceTimersByTime(100);
    expect(onquery).not.toHaveBeenCalled();

    vi.advanceTimersByTime(40);
    expect(onquery).toHaveBeenCalledTimes(1);
    expect(onquery).toHaveBeenCalledWith('ca');
  });

  it('shows only the rows whose name contains the query, whatever the case, in the same order', () => {
    search({ query: 'cargo' });

    expect(names()).toEqual(['Cargo.lock', 'CARGO_HOME']);
  });

  it('underlines the matching part of the name, with no background', () => {
    search({ query: 'RG' });

    const marks = [...document.querySelectorAll('.row mark')];

    expect(marks.map((mark) => mark.textContent)).toEqual(['rg', 'rg', 'RG']);
    expect(document.querySelector('.row .nm')?.textContent).toBe('Cargo.lock');
  });

  it('counts "M of N" in the header while filtering', () => {
    search({ query: 'cargo' });

    expect(document.querySelector('.head .count')?.textContent).toBe('2 of 5 items');
  });

  it('says so in one line when nothing matches, and shows no rows', () => {
    search({ query: 'zzz' });

    expect(names()).toEqual([]);
    expect(screen.getByText('No folder or file here matches "zzz"')).toBeTruthy();
  });

  it('does not say nothing matches while pages are still coming', () => {
    search({ query: 'zzz', searching: true, hasMore: true, loaded: 1400, allTotal: 3200 });

    expect(screen.queryByText(/No folder or file/)).toBeNull();
    expect(screen.getByText('Searching… 1,400 of 3,200')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Load more/ })).toBeNull();
  });

  it('says how many items were searched when the folder has more than the limit', () => {
    search({ query: 'cargo', searchTruncated: true, hasMore: true, allTotal: 7200, loaded: 5000 });

    expect(screen.getByText('Searched the first 5,000 of 7,200 items')).toBeTruthy();
  });

  it('keeps a filtered-out selected row selected, and does not list it', () => {
    const onchoose = vi.fn();

    search({ query: 'docs', selected: 1, onchoose });

    expect(names()).toEqual(['docs']);
    expect(document.querySelector('.row.sel')).toBeNull();
    expect(onchoose).not.toHaveBeenCalled();
  });

  it('filters the rows that are not counted with the same query, and drops the footer when none match', async () => {
    const off = [zero(10, 'boot', { kind: 'mount' }), zero(11, 'root', { unreadable: true })];

    const { unmount } = search({ query: 'oo', uncounted: off, uncountedTotal: 2 });

    expect(screen.getByRole('button', { name: '2 items not counted' })).toBeTruthy();
    unmount();

    search({ query: 'boo', uncounted: off, uncountedTotal: 2 });
    expect(screen.getByRole('button', { name: '1 item not counted' })).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: '1 item not counted' }));
    expect([...document.querySelectorAll('.row.off .nm')].map((nm) => nm.textContent)).toEqual(['boot']);
  });

  it('has no footer at all when no uncounted row matches', () => {
    const off = [zero(10, 'boot', { kind: 'mount' })];

    search({ query: 'cargo', uncounted: off, uncountedTotal: 1 });

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
