import { flushSync, mount, unmount } from 'svelte';
import { waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ColorMap } from './colors';
import ColumnsView from './ColumnsView.svelte';
import type { ColumnsLayout } from './layout/columns';

// The 3D engine needs WebGL, which jsdom has not: a stand-in records what the view asks of it.
const { calls } = vi.hoisted(() => ({ calls: [] as { method: string; args: unknown[] }[] }));

vi.mock('./three/scene', () => ({
  ColumnsScene: class {
    setLayout(...args: unknown[]): void {
      calls.push({ method: 'setLayout', args });
    }

    setColors(...args: unknown[]): void {
      calls.push({ method: 'setColors', args });
    }

    setHighlight(): void {}

    invalidate(): void {}

    dispose(): void {}
  },
}));

const layout = (): ColumnsLayout => ({ columns: [], omitted: 0, boosted: false });
const called = (method: string): unknown[][] =>
  calls.filter((call) => call.method === method).map((call) => call.args);

describe('the 3D columns view', () => {
  // Separate values, so that changing one does not make the view read the other again.
  let shown = $state.raw<ColumnsLayout>(layout());
  let colors = $state.raw<ColorMap>(new Map());
  let mounted: ReturnType<typeof mount> | undefined;

  beforeEach(() => {
    calls.length = 0;
    shown = layout();
    colors = new Map();
  });

  afterEach(() => {
    if (mounted) {
      void unmount(mounted);
    }

    mounted = undefined;
  });

  it('only recolours the scene when the colours arrive late, and replaces the columns only for a new layout', async () => {
    mounted = mount(ColumnsView, {
      target: document.body,
      props: {
        get layout() {
          return shown;
        },
        get colors() {
          return colors;
        },
        selected: null,
        hovered: null,
        onchoose: () => {},
        onhover: () => {},
        onclear: () => {},
        ongroup: () => {},
        onfail: () => {},
      },
    });

    await waitFor(() => expect(called('setLayout')).toHaveLength(1));

    const late: ColorMap = new Map([[7, '#3ec5d8']]);

    colors = late;
    flushSync();

    await waitFor(() => expect(called('setColors').at(-1)?.[0]).toBe(late));
    expect(called('setLayout')).toHaveLength(1);

    const next = layout();

    shown = next;
    flushSync();

    await waitFor(() => expect(called('setLayout')).toHaveLength(2));
    // The new columns are made with the colours known by then.
    expect(called('setLayout')[1]).toEqual([next, late]);
  });
});
