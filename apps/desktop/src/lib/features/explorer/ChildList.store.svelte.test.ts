import { fireEvent, render, screen } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Api } from '../../api/client';
import { createMockApi } from '../../api/mock';
import type { NodeDto } from '../../api/types';
import ChildList from './ChildList.svelte';
import { ExplorerStore } from './state/explorer.svelte';

/** The list as the page wires it to the store, on a folder of several pages with rows that have no size. */
const cleanups: (() => void)[] = [];

afterEach(() => {
  cleanups.splice(0).forEach((stop) => stop());
});

async function folderOf(sizes: number[]): Promise<ExplorerStore> {
  const base = createMockApi({ durationMs: 0, now: () => 1_750_000_000 });
  const done = await new Promise<{ scanId: number; rootId: number }>((resolve, reject) => {
    base
      .startScan({ root: '/', oneFileSystem: true }, (event) => {
        if (event.type === 'finished') {
          resolve({ scanId: event.scanId, rootId: event.rootId });
        } else if (event.type === 'failed' || event.type === 'cancelled') {
          reject(new Error(event.type));
        }
      })
      .catch(reject);
  });
  const all: NodeDto[] = sizes.map((allocated, index) => ({
    id: 1000 + index,
    name: `item-${String(index).padStart(3, '0')}`,
    kind: allocated > 0 ? 'file' : 'dir',
    allocated,
    apparent: allocated,
    files: 0,
    hasChildren: false,
    category: 'other',
    mtime: 1,
  }));
  const api: Api = {
    ...base,
    getChildren: async (_scan, node, offset, limit, sort) => {
      const sorted = [...all].sort((a, b) =>
        sort === 'nameAsc' ? a.name.localeCompare(b.name) : b.allocated - a.allocated || a.id - b.id,
      );

      return { parentId: node, total: sorted.length, items: sorted.slice(offset, offset + limit) };
    },
  };
  let store!: ExplorerStore;

  cleanups.push(
    $effect.root(() => {
      store = new ExplorerStore(done.scanId, done.rootId, { focus: done.rootId }, api);
    }),
  );
  flushSync();
  await tick();
  await vi.waitFor(() => {
    expect(store.frame.loading).toBe(false);
    expect(store.page.loading).toBe(false);
  });

  return store;
}

function show(store: ExplorerStore) {
  return render(ChildList, {
    props: {
      folder: store.focus,
      items: store.sizedItems,
      total: store.sizedTotal,
      uncounted: store.uncountedItems,
      uncountedTotal: store.uncountedTotal,
      sort: store.sort,
      selected: null,
      hovered: null,
      colorOf: () => '#fff',
      loading: store.page.loading,
      hasMore: store.hasMore,
      loadingMore: false,
      moreError: null,
      allTotal: store.total,
      loaded: store.items.length,
      onsort: vi.fn(),
      onchoose: vi.fn(),
      onhover: vi.fn(),
      onmore: vi.fn(),
      onretry: vi.fn(),
    },
  });
}

const listed = (): string[] =>
  [...document.querySelectorAll('.row.off .nm')].map((nm) => nm.textContent ?? '');

describe('the list on a folder with more than one page of rows without a size', () => {
  it('sorted by size: the footer counts them all, unfolding lists the loaded ones, "Load more" counts the rest', async () => {
    const store = await folderOf([...Array(100).fill(5), ...Array(200).fill(0)]);

    show(store);
    await fireEvent.click(screen.getByRole('button', { name: '200 items not counted' }));

    expect(listed()).toHaveLength(100);
    expect(screen.getByRole('button', { name: 'Load more (100 left)' })).toBeTruthy();
  });

  it('sorted by name: does not hide the loaded rows, and does not say a total it cannot know', async () => {
    const store = await folderOf([...Array(200).fill(0), ...Array(100).fill(5)]);

    store.setSort('nameAsc');
    await vi.waitFor(() => expect(store.pageCurrent).toBe(true));
    show(store);

    expect(screen.queryByText('Nothing in this folder.')).toBeNull();

    await fireEvent.click(screen.getByRole('button', { name: '200 items not counted so far' }));

    expect(listed()).toHaveLength(200);
    expect(screen.getByRole('button', { name: 'Load more (100 left)' })).toBeTruthy();
  });
});
