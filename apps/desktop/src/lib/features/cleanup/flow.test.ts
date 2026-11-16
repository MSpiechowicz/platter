import { describe, expect, it } from 'vitest';
import type { Verdict } from '../../api/types';
import {
  BAR_WIDTH,
  blendLayouts,
  canBlend,
  fitHeights,
  layoutFlow,
  PAD,
  REST_ID,
  SIDE_GAP,
  type FlowItem,
  type FlowLayout,
  type FlowSide,
} from './flow';

const GB = 1_000_000_000;

function item(id: string, gb: number, side: FlowSide = 'stays', verdict: Verdict = 'safe'): FlowItem {
  return { id, label: `Group ${id}`, size: gb * GB, verdict, side, clickable: verdict !== 'keep' };
}

const sum = (values: number[]): number => values.reduce((total, value) => total + value, 0);
const nodeOf = (layout: FlowLayout, id: string) => {
  const node = layout.nodes.find((candidate) => candidate.id === id);

  if (!node) {
    throw new Error(`no node ${id}`);
  }

  return node;
};

/** The box of the mockup's screenshot. */
const WIDTH = 704;
const HEIGHT = 440;

describe('fitHeights', () => {
  it('shares the room in proportion to the sizes', () => {
    const heights = fitHeights([300, 100], 400, 16);

    expect(heights[0]).toBeCloseTo(300, 6);
    expect(heights[1]).toBeCloseTo(100, 6);
  });

  it('holds a thin one at the minimum and takes the difference from the others, so the sum stays exact', () => {
    const heights = fitHeights([1000, 1], 200, 16);

    expect(heights[1]).toBe(16);
    expect(heights[0]).toBeCloseTo(184, 6);
    expect(sum(heights)).toBeCloseTo(200, 6);
  });

  it('keeps the order of the sizes it was given', () => {
    const heights = fitHeights([1, 1000, 2, 500], 300, 16);

    expect(heights[1]).toBeGreaterThan(heights[3] as number);
    expect(heights[3]).toBeGreaterThan(heights[2] as number);
    expect(heights[0]).toBe(16);
    expect(heights[2]).toBeGreaterThanOrEqual(16);
    expect(sum(heights)).toBeCloseTo(300, 6);
  });

  it('shares equally when not even the minimum fits, and when every size is 0', () => {
    expect(fitHeights([5, 1, 1, 1], 40, 16)).toEqual([10, 10, 10, 10]);
    expect(fitHeights([0, 0], 30, 16)).toEqual([15, 15]);
  });

  it('has nothing to give for no sizes, and never a negative height for no room', () => {
    expect(fitHeights([], 100, 16)).toEqual([]);
    expect(fitHeights([1, 2], -10, 16)).toEqual([0, 0]);
  });
});

describe('layoutFlow', () => {
  it('draws nothing for a box with no room, and nothing when there is nothing to show', () => {
    expect(layoutFlow({ items: [item('a', 10)], total: 100 * GB, width: 0, height: HEIGHT }).nodes).toEqual(
      [],
    );
    expect(layoutFlow({ items: [item('a', 10)], total: 100 * GB, width: WIDTH, height: 0 }).nodes).toEqual(
      [],
    );
    expect(layoutFlow({ items: [], total: 0, width: WIDTH, height: HEIGHT }).nodes).toEqual([]);
  });

  it('with no groups there is only the rest, and it stays on the disk', () => {
    const layout = layoutFlow({ items: [], total: 100 * GB, width: WIDTH, height: HEIGHT });

    expect(layout.nodes.map((node) => node.id)).toEqual([REST_ID]);
    expect(layout.freed).toBeNull();
    expect(layout.stays?.size).toBe(100 * GB);
    expect(nodeOf(layout, REST_ID).clickable).toBe(false);
  });

  it('orders the groups largest first and puts everything else last', () => {
    const layout = layoutFlow({
      items: [item('small', 5), item('big', 50), item('mid', 20)],
      total: 200 * GB,
      width: WIDTH,
      height: HEIGHT,
    });

    expect(layout.nodes.map((node) => node.id)).toEqual(['big', 'mid', 'small', REST_ID]);
    expect(nodeOf(layout, REST_ID).size).toBe(125 * GB);
  });

  it('keeps the given order between groups of one size', () => {
    const layout = layoutFlow({
      items: [item('b', 10), item('a', 10), item('c', 10)],
      total: 100 * GB,
      width: WIDTH,
      height: HEIGHT,
    });

    expect(layout.nodes.map((node) => node.id).slice(0, 3)).toEqual(['b', 'a', 'c']);
  });

  it('fills the box from the top to the bottom padding, with no empty band', () => {
    const layout = layoutFlow({
      items: [item('a', 88), item('b', 61), item('c', 3), item('d', 1)],
      total: 1400 * GB,
      width: WIDTH,
      height: HEIGHT,
    });
    const first = layout.nodes[0];
    const last = layout.nodes[layout.nodes.length - 1];

    expect(first?.y).toBe(PAD);
    expect((last?.y ?? 0) + (last?.h ?? 0)).toBeCloseTo(HEIGHT - PAD, 6);

    for (const [index, node] of layout.nodes.slice(1).entries()) {
      const before = layout.nodes[index];

      // A fixed gap between neighbours, and no overlap.
      expect(node.y - ((before?.y ?? 0) + (before?.h ?? 0))).toBeCloseTo(5, 6);
    }
  });

  it('draws bars in proportion to their size when none is thin', () => {
    const layout = layoutFlow({
      items: [item('a', 300), item('b', 100)],
      total: 600 * GB,
      width: WIDTH,
      height: HEIGHT,
    });

    expect(nodeOf(layout, 'a').h / nodeOf(layout, 'b').h).toBeCloseTo(3, 6);
    expect(nodeOf(layout, REST_ID).h / nodeOf(layout, 'b').h).toBeCloseTo(2, 6);
  });

  it('gives a thin group a minimum height so it can be read and clicked, and takes it from the big ones', () => {
    const layout = layoutFlow({
      items: [item('huge', 900), item('thin', 0.5)],
      total: 1000 * GB,
      width: WIDTH,
      height: HEIGHT,
    });

    expect(nodeOf(layout, 'thin').h).toBe(16);
    expect(nodeOf(layout, 'thin').h).toBeGreaterThan((0.5 / 1000) * HEIGHT);
    expect(sum(layout.nodes.map((node) => node.h)) + 2 * 5).toBeCloseTo(HEIGHT - 2 * PAD, 6);
  });

  it('a group of 97% of the total takes nearly all the height; the others keep the minimum', () => {
    const layout = layoutFlow({
      items: [item('home', 970), item('tiny-1', 0.3), item('tiny-2', 0.2)],
      total: 1000 * GB,
      width: WIDTH,
      height: HEIGHT,
    });
    const big = nodeOf(layout, 'home');

    expect(nodeOf(layout, 'tiny-1').h).toBe(16);
    expect(nodeOf(layout, 'tiny-2').h).toBe(16);
    expect(nodeOf(layout, REST_ID).h).toBe(16);
    expect(big.h).toBeGreaterThan(HEIGHT * 0.7);
    expect(big.h + 3 * 16 + 3 * 5).toBeCloseTo(HEIGHT - 2 * PAD, 6);
  });

  it('with a total of 0 and groups, there is no rest and the groups fill the box', () => {
    const layout = layoutFlow({
      items: [item('a', 3), item('b', 1)],
      total: 0,
      width: WIDTH,
      height: HEIGHT,
    });

    expect(layout.nodes.map((node) => node.id)).toEqual(['a', 'b']);
    expect(layout.nodes.every((node) => Number.isFinite(node.h) && node.h > 0)).toBe(true);
    expect(sum(layout.nodes.map((node) => node.h)) + 5).toBeCloseTo(HEIGHT - 2 * PAD, 6);
  });

  it('leaves out groups of no bytes and with a size that is not a number', () => {
    const layout = layoutFlow({
      items: [item('a', 10), item('zero', 0), { ...item('nan', 1), size: Number.NaN }],
      total: 100 * GB,
      width: WIDTH,
      height: HEIGHT,
    });

    expect(layout.nodes.map((node) => node.id)).toEqual(['a', REST_ID]);
  });

  it('a total below what the groups add up to leaves no rest and makes no negative height', () => {
    const layout = layoutFlow({
      items: [item('a', 80), item('b', 60)],
      total: 100 * GB,
      width: WIDTH,
      height: HEIGHT,
    });

    expect(layout.nodes.map((node) => node.id)).toEqual(['a', 'b']);
    expect(layout.nodes.every((node) => node.h > 0)).toBe(true);
  });

  it('with very many tiny groups every bar still has a height, nothing overflows, and the gaps tighten first', () => {
    const items = Array.from({ length: 40 }, (_, index) => item(`g${index}`, 0.1 + index / 1000));
    const layout = layoutFlow({ items, total: 1000 * GB, width: WIDTH, height: HEIGHT });
    const last = layout.nodes[layout.nodes.length - 1];

    expect(layout.nodes).toHaveLength(41);
    expect(layout.nodes.every((node) => node.h >= 6)).toBe(true);
    expect((last?.y ?? 0) + (last?.h ?? 0)).toBeLessThanOrEqual(HEIGHT - PAD + 1e-6);
    // The rest is still the biggest.
    expect(nodeOf(layout, REST_ID).h).toBe(Math.max(...layout.nodes.map((node) => node.h)));
  });

  it('with more groups than a bar of 6 px fits, they share the room and stay inside the box', () => {
    const items = Array.from({ length: 150 }, (_, index) => item(`g${index}`, 1));
    const layout = layoutFlow({ items, total: 200 * GB, width: WIDTH, height: HEIGHT });
    const last = layout.nodes[layout.nodes.length - 1];

    expect(layout.nodes.every((node) => node.h > 0 && Number.isFinite(node.y))).toBe(true);
    expect((last?.y ?? 0) + (last?.h ?? 0)).toBeLessThanOrEqual(HEIGHT - PAD + 1e-6);
  });

  it('survives a box smaller than its padding', () => {
    const layout = layoutFlow({ items: [item('a', 10)], total: 20 * GB, width: 40, height: 10 });

    expect(layout.nodes.every((node) => node.h >= 0 && Number.isFinite(node.y) && node.path.length > 0)).toBe(
      true,
    );
    expect(layout.nodes.every((node) => !node.path.includes('NaN'))).toBe(true);
  });

  describe('the two sides', () => {
    const items = [item('a', 100, 'freed'), item('b', 60, 'stays'), item('c', 40, 'freed')];

    it('sends a ribbon to the side its group is on, in the order of the bars, and counts the bytes per side', () => {
      const layout = layoutFlow({ items, total: 400 * GB, width: WIDTH, height: HEIGHT });

      expect(layout.freed?.size).toBe(140 * GB);
      expect(layout.stays?.size).toBe(260 * GB);
      expect(nodeOf(layout, 'a').ty).toBe(PAD);
      expect(nodeOf(layout, 'c').ty).toBeCloseTo(PAD + nodeOf(layout, 'a').th, 6);
      expect(nodeOf(layout, 'b').ty).toBeGreaterThan(nodeOf(layout, 'c').ty);
    });

    it('sets the bars on the right apart by a gap and keeps them inside the box', () => {
      const layout = layoutFlow({ items, total: 400 * GB, width: WIDTH, height: HEIGHT });
      const freed = layout.freed;
      const stays = layout.stays;

      expect(freed?.y).toBe(PAD);
      expect((stays?.y ?? 0) - ((freed?.y ?? 0) + (freed?.h ?? 0))).toBeCloseTo(SIDE_GAP, 6);
      expect((stays?.y ?? 0) + (stays?.h ?? 0)).toBeLessThanOrEqual(HEIGHT - PAD + 1e-6);
    });

    it('squeezes the ribbons on the right when two groups leave too little room for the gap, never past the box', () => {
      const layout = layoutFlow({
        items: [item('a', 50, 'freed')],
        total: 100 * GB,
        width: WIDTH,
        height: 120,
      });

      expect(layout.freed && layout.stays).toBeTruthy();
      expect((layout.stays?.y ?? 0) + (layout.stays?.h ?? 0)).toBeLessThanOrEqual(120 - PAD + 1e-6);
      expect(nodeOf(layout, 'a').th).toBeLessThan(nodeOf(layout, 'a').h);
    });

    it('with nothing selected only the stay bar exists, and with everything selected only the freed one', () => {
      const none = layoutFlow({ items: [item('a', 100)], total: 100 * GB, width: WIDTH, height: HEIGHT });
      const all = layoutFlow({
        items: [item('a', 60, 'freed'), item('b', 40, 'freed')],
        total: 100 * GB,
        width: WIDTH,
        height: HEIGHT,
      });

      expect(none.freed).toBeNull();
      expect(none.stays?.y).toBe(PAD);
      expect(all.stays).toBeNull();
      expect(all.freed?.size).toBe(100 * GB);
    });

    it('never frees a kept group, even when it is said to be on the freed side, and never lets it be clicked', () => {
      const layout = layoutFlow({
        items: [item('steam', 50, 'freed', 'keep')],
        total: 100 * GB,
        width: WIDTH,
        height: HEIGHT,
      });

      expect(nodeOf(layout, 'steam').side).toBe('stays');
      expect(nodeOf(layout, 'steam').clickable).toBe(false);
      expect(layout.freed).toBeNull();
    });

    it('passes the progress and the clickable flag on', () => {
      const layout = layoutFlow({
        items: [{ ...item('a', 10, 'freed'), progress: 'done', clickable: false }, item('b', 10, 'freed')],
        total: 100 * GB,
        width: WIDTH,
        height: HEIGHT,
      });

      expect(nodeOf(layout, 'a')).toMatchObject({ progress: 'done', clickable: false });
      expect(nodeOf(layout, 'b')).toMatchObject({ progress: undefined, clickable: true });
    });
  });

  describe('the ribbons', () => {
    it('start at the bar and end at the right column, inside the box, with no broken numbers', () => {
      const layout = layoutFlow({
        items: [item('a', 100, 'freed'), item('b', 50)],
        total: 400 * GB,
        width: WIDTH,
        height: HEIGHT,
      });

      expect(layout.endX).toBe(WIDTH - 148);
      expect(layout.barX).toBe(250);

      for (const node of layout.nodes) {
        const numbers = (node.path.match(/-?\d+(\.\d+)?/g) ?? []).map(Number);

        expect(node.path.startsWith(`M${layout.barX + BAR_WIDTH},${node.y}`)).toBe(true);
        expect(node.path.endsWith('Z')).toBe(true);
        expect(Math.min(...numbers)).toBeGreaterThanOrEqual(0);
        expect(Math.max(...numbers)).toBeLessThanOrEqual(Math.max(WIDTH, HEIGHT));
      }
    });

    it('uses the narrow label column in a narrow box, and never lets the bars cross', () => {
      const narrow = layoutFlow({ items: [item('a', 10)], total: 100 * GB, width: 500, height: HEIGHT });
      const tiny = layoutFlow({ items: [item('a', 10)], total: 100 * GB, width: 200, height: HEIGHT });

      expect(narrow.barX).toBe(188);
      expect(tiny.barX).toBeLessThanOrEqual(200 * 0.4);
      expect(tiny.endX).toBeGreaterThan(tiny.barX + BAR_WIDTH);
    });
  });
});

describe('blending', () => {
  const base = [item('a', 100, 'stays'), item('b', 50, 'stays')];
  const before = layoutFlow({ items: base, total: 400 * GB, width: WIDTH, height: HEIGHT });
  const after = layoutFlow({
    items: [item('a', 100, 'freed'), item('b', 50, 'stays')],
    total: 400 * GB,
    width: WIDTH,
    height: HEIGHT,
  });

  it('can blend layouts of the same groups in the same box, and not after a resize or a new group', () => {
    expect(canBlend(before, after)).toBe(true);
    expect(
      canBlend(before, layoutFlow({ items: base, total: 400 * GB, width: WIDTH + 1, height: HEIGHT })),
    ).toBe(false);
    expect(
      canBlend(
        before,
        layoutFlow({ items: [...base, item('c', 5)], total: 400 * GB, width: WIDTH, height: HEIGHT }),
      ),
    ).toBe(false);
  });

  it('starts at the old ribbons, ends at the new ones, and moves the ends in between', () => {
    expect(blendLayouts(before, after, 0).nodes.map((node) => node.ty)).toEqual(
      before.nodes.map((node) => node.ty),
    );
    expect(blendLayouts(before, after, 1)).toBe(after);

    const half = blendLayouts(before, after, 0.5);
    const a = nodeOf(half, 'a');

    expect(a.ty).toBeCloseTo((nodeOf(before, 'a').ty + nodeOf(after, 'a').ty) / 2, 6);
    expect(a.y).toBe(nodeOf(after, 'a').y);
    expect(a.side).toBe('freed');
    expect(half.stays?.y).toBeGreaterThan(PAD);
  });

  it('gives the new layout when the two cannot be blended', () => {
    const resized = layoutFlow({ items: base, total: 400 * GB, width: WIDTH + 10, height: HEIGHT });

    expect(blendLayouts(before, resized, 0.5)).toBe(resized);
  });
});
