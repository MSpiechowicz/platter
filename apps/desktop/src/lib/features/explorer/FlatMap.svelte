<script lang="ts">
  import { untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { COLLAPSED_NODE_ID } from '../../api/client';
  import type { NodeDto, NodeId } from '../../api/types';
  import { fmt } from '../../state/fmt.svelte';
  import { motion } from '../../state/motion.svelte';
  import { tileStyle, type ColorMap } from './colors';
  import { linksOf, type ChainedLod } from './layout/chains';
  import { membersOf, tileRadius } from './layout/legibility';
  import { formatShare } from './layout/scale';
  import {
    groupByTop,
    isCollapsed,
    layoutTreemap,
    mergedTooltip,
    tileLabel,
    tileTitle,
    type Tile,
  } from './layout/treemap';
  import MapMessage from './MapMessage.svelte';
  import { tileMarked } from './marking';
  import { moveFocus } from './roving';
  import type { Frame } from './state/explorer.svelte';

  interface Props {
    frame: Frame;
    /** What is drawn: the folder's tree with every chain of single-child folders as one node. */
    model: ChainedLod;
    colors: ColorMap;
    selected: NodeId | null;
    hovered: NodeId | null;
    onchoose: (id: NodeId) => void;
    onhover: (id: NodeId | null) => void;
    onclear: () => void;
    /** The "smaller items" tile was chosen: the node it stands for and the items merged into it. */
    ongroup: (node: NodeDto, members: readonly NodeId[]) => void;
    /** The folders the user marked for cleanup: a tile that stands for one gets a dashed outline. */
    marked?: ReadonlySet<NodeId>;
    /** The tiles (by the id of their first folder) that are the topmost safe ones: a green outline and a SAFE tag. */
    safe?: ReadonlySet<NodeId>;
    /** Whether some tile is drawn larger than its true share, so the legend can say so. */
    onboost?: (boosted: boolean) => void;
  }

  let {
    frame,
    model,
    colors,
    selected,
    hovered,
    onchoose,
    onhover,
    onclear,
    ongroup,
    marked = new SvelteSet(),
    safe = new SvelteSet(),
    onboost,
  }: Props = $props();

  const ZOOM_IN_MS = 450;
  const ZOOM_OUT_MS = 350;
  const EASE = 'cubic-bezier(0.2, 0.8, 0.2, 1)';

  let box: HTMLDivElement | undefined = $state();
  let svg: SVGSVGElement | undefined = $state();
  let width = $state(0);
  let height = $state(0);

  // The map is laid out at the size it is drawn at, so a tile edge is always on a whole pixel.
  $effect(() => {
    if (!box) {
      return;
    }

    const observer = new ResizeObserver((entries) => {
      const size = entries[0]?.contentRect;

      width = size?.width ?? 0;
      height = size?.height ?? 0;
    });

    observer.observe(box);

    return () => observer.disconnect();
  });

  const tiles = $derived(layoutTreemap(model, width, height));
  const groups = $derived(groupByTop(tiles));

  $effect(() => {
    onboost?.(tiles.some((tile) => tile.boosted));
  });

  // Opening a folder zooms in from its tile, going up zooms out.
  let previous: { focus: NodeId; tops: Map<NodeId, Tile> } | null = null;

  $effect(() => {
    const next = {
      focus: frame.focus,
      // A chain tile is the folder the user opens, and also every folder of the chain.
      tops: new Map(
        tiles
          .filter((tile) => tile.depth === 1)
          .flatMap((tile) => linksOf(tile).map((link): [NodeId, Tile] => [link.id, tile])),
      ),
    };

    untrack(() => {
      const before = previous;

      previous = next;

      if (before && before.focus !== next.focus && svg && width > 0 && !motion.reduced) {
        zoom(svg, before.tops.get(next.focus));
      }
    });
  });

  function zoom(target: SVGSVGElement, from: Tile | undefined): void {
    if (from) {
      target.animate(
        [
          {
            transformOrigin: '0 0',
            transform: `translate(${from.x0}px, ${from.y0}px) scale(${(from.x1 - from.x0) / width}, ${(from.y1 - from.y0) / height})`,
            opacity: 0.35,
          },
          { transformOrigin: '0 0', transform: 'none', opacity: 1 },
        ],
        { duration: ZOOM_IN_MS, easing: EASE },
      );
    } else {
      target.animate(
        [
          { transformOrigin: '50% 50%', transform: 'scale(1.12)', opacity: 0 },
          { transformOrigin: '50% 50%', transform: 'none', opacity: 1 },
        ],
        { duration: ZOOM_OUT_MS, easing: EASE },
      );
    }
  }

  // Keyboard: one folder is in the tab order, the arrow keys move between folders.
  let current = $state<NodeId | null>(null);

  const tabbable = $derived.by(() => {
    const ids = groups.map((group) => group.top);

    if (current !== null && ids.includes(current)) {
      return current;
    }

    return selected !== null && ids.includes(selected) ? selected : ids[0];
  });

  const SAFE_TAG = 'SAFE';

  /** What is said of a tile besides its size, for the tooltip and the screen reader: colour is never the only cue. */
  function states(tile: Tile): string[] {
    return [
      ...(safe.has(tile.node.id) ? ['safe to clean'] : []),
      ...(tileMarked(tile, marked) ? ['marked for cleanup'] : []),
    ];
  }

  /** What the tooltip says: the whole path of a chain, the size, and the true share of a tile drawn larger than it is. */
  function tooltip(tile: Tile): string {
    if (isCollapsed(tile.node)) {
      return mergedTooltip(tile, fmt.size(tile.node.allocated));
    }

    const share = tile.boosted ? ` · ${formatShare(tile.share)} of this folder` : '';

    return [`${tileTitle(tile)} · ${fmt.size(tile.node.allocated)}${share}`, ...states(tile)].join(' · ');
  }

  function onkeydown(event: KeyboardEvent, id: NodeId): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      pick(id);
      return;
    }

    moveFocus(event, [...(svg?.querySelectorAll<SVGGElement>('g.top[tabindex]') ?? [])], 'both');
  }

  function choose(event: MouseEvent, id: NodeId): void {
    event.stopPropagation();
    pick(id);
  }

  /** A folder or file is chosen like in the list; the "smaller items" tile selects its items and is never opened. */
  function pick(id: NodeId): void {
    const first = groups.find((group) => group.top === id)?.tiles[0];

    if (id === COLLAPSED_NODE_ID && first) {
      ongroup(first.node, membersOf(first.node));
    } else {
      onchoose(id);
    }
  }
</script>

<div class="box" bind:this={box}>
  {#if tiles.length === 0 && width > 0}
    <MapMessage title="Nothing to draw here">
      The items in this folder take no space, so there is no map of them. The list on the right still shows
      them.
    </MapMessage>
  {/if}

  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <svg
    bind:this={svg}
    class:has-sel={selected !== null}
    {width}
    {height}
    aria-label="Map of the open folder"
    onclick={onclear}
  >
    {#snippet tileSet(set: Tile[])}
      {#each set as tile (tile.key)}
        {@const isSafe = safe.has(tile.node.id)}
        {@const size = fmt.size(tile.node.allocated)}
        {@const label = tileLabel(tile, isSafe ? `${SAFE_TAG} · ${size}` : size)}
        {@const style = tileStyle(tile, colors)}
        <!-- A "smaller items" tile has its folder's colour, a step quieter, and a dashed outline. -->
        <rect
          class:first={tile.depth === 1}
          class:merged={style.merged}
          class:safe={isSafe}
          class:marked={tileMarked(tile, marked)}
          x={tile.x0}
          y={tile.y0}
          width={tile.x1 - tile.x0}
          height={tile.y1 - tile.y0}
          rx={tileRadius(tile.x1 - tile.x0, tile.y1 - tile.y0)}
          fill={style.fill}
          stroke={style.stroke}
          stroke-opacity=".55"
          stroke-dasharray={style.merged ? '3 3' : undefined}
        >
          {#if style.merged}<title>{tooltip(tile)}</title>{/if}
        </rect>
        {#if label?.where === 'header'}
          <text class="nm" x={tile.x0 + 8} y={tile.y0 + 17}>{label.name}</text>
          {#if label.size}<text class="sz" x={tile.x1 - 8} y={tile.y0 + 17} text-anchor="end"
              >{#if isSafe}<tspan class="tag">{SAFE_TAG}</tspan>{label.size.slice(
                  SAFE_TAG.length,
                )}{:else}{label.size}{/if}</text
            >{/if}
        {:else if label}
          <text class="nm" x={tile.x0 + 8} y={tile.y0 + 19}>{label.name}</text>
          {#if label.size}<text class="sz" x={tile.x0 + 8} y={tile.y0 + 35}
              >{#if isSafe}<tspan class="tag">{SAFE_TAG}</tspan>{label.size.slice(
                  SAFE_TAG.length,
                )}{:else}{label.size}{/if}</text
            >{/if}
        {/if}
      {/each}
    {/snippet}

    {#each groups as group (group.top)}
      {@const first = group.tiles[0]}
      {#if first}
        <g
          class="top pick"
          class:sel={group.top === selected}
          class:hot={group.top === hovered}
          role="button"
          tabindex={group.top === tabbable ? 0 : -1}
          aria-pressed={group.top === selected}
          aria-label={[`${tileTitle(first)}, ${fmt.size(first.node.allocated)}`, ...states(first)].join(', ')}
          onclick={(event) => choose(event, group.top)}
          onkeydown={(event) => onkeydown(event, group.top)}
          onfocus={() => (current = group.top)}
          onmouseenter={() => onhover(group.top)}
          onmouseleave={() => onhover(null)}
        >
          <!-- Small tiles have no room for text: the tooltip says what they are. -->
          <title>{tooltip(first)}</title>
          {@render tileSet(group.tiles)}
        </g>
      {/if}
    {/each}
  </svg>
</div>

<style>
  .box {
    position: absolute;
    inset: 0;
  }

  svg {
    position: absolute;
    inset: 0;
    display: block;
    transform-origin: 0 0;
  }

  .top {
    transition: opacity 0.2s;
  }

  .pick {
    cursor: pointer;
  }

  .top rect {
    transition: filter 0.15s;
  }

  .hot rect {
    filter: brightness(var(--hot-brightness));
  }

  /* Everything but the selected folder steps back. */
  .has-sel .top:not(.sel) {
    opacity: var(--step-back);
  }

  /* Safe to clean: a green outline. Green means safe and nothing else. */
  .top rect.safe {
    stroke: var(--green);
    stroke-width: 2;
    stroke-opacity: 1;
  }

  .sel > rect.first {
    stroke: var(--strong);
    stroke-width: 2;
    stroke-opacity: 1;
  }

  /* Marked for cleanup: a dashed outline, over the others. */
  .top rect.marked {
    stroke: var(--strong);
    stroke-width: 2;
    stroke-opacity: 1;
    stroke-dasharray: 6 4;
  }

  .pick:focus-visible {
    outline: none;
  }

  :global(:root:not([data-input='pointer'])) .pick:focus-visible > rect.first {
    stroke: var(--accent);
    stroke-width: 2.5;
    stroke-opacity: 1;
  }

  text {
    font-family: var(--font);
    fill: var(--strong);
    pointer-events: none;
  }

  text.nm {
    font-family: var(--mono);
    font-weight: 600;
    font-size: 13px;
  }

  text.sz {
    font-size: 12px;
    opacity: 0.8;
    font-variant-numeric: tabular-nums;
  }

  text .tag {
    fill: var(--green);
    font-weight: 700;
    font-size: 10.5px;
    letter-spacing: 0.06em;
  }
</style>
