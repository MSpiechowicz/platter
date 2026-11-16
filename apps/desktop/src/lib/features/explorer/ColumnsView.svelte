<script lang="ts">
  import { onMount } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { COLLAPSED_NODE_ID } from '../../api/client';
  import type { NodeDto, NodeId } from '../../api/types';
  import Loading from '../../components/ui/Loading.svelte';
  import { hooks } from '../../hooks';
  import { fmt } from '../../state/fmt.svelte';
  import { motion } from '../../state/motion.svelte';
  import type { ColorMap } from './colors';
  import type { Column, ColumnsLayout } from './layout/columns';
  import { formatShare } from './layout/scale';
  import { fitTitle } from './layout/text';
  import { tileMarked } from './marking';
  import MapMessage from './MapMessage.svelte';
  import { LabelLayer } from './three/labelLayer';
  import type { ColumnsScene, SceneFrame } from './three/scene';

  /** `.lb b` is this wide, in pixels. */
  const LABEL_WIDTH = 200;

  interface Props {
    /** The columns to draw: made by the page, which also tells how many items they leave out. */
    layout: ColumnsLayout;
    colors: ColorMap;
    selected: NodeId | null;
    hovered: NodeId | null;
    onchoose: (id: NodeId) => void;
    onhover: (id: NodeId | null) => void;
    onclear: () => void;
    /** The "smaller items" column was chosen: the node it stands for and the items merged into it. */
    ongroup: (node: NodeDto, members: readonly NodeId[]) => void;
    /** The folders the user marked for cleanup: the label of a column that stands for one has a dashed border. */
    marked?: ReadonlySet<NodeId>;
    /** The columns (by the id of their first folder) that are safe to clean: a green label border and a SAFE tag. */
    safe?: ReadonlySet<NodeId>;
    /** The 3D engine could not start; the page falls back to the flat map. */
    onfail: (message: string) => void;
  }

  let {
    layout,
    colors,
    selected,
    hovered,
    onchoose,
    onhover,
    onclear,
    ongroup,
    marked = new SvelteSet(),
    safe = new SvelteSet(),
    onfail,
  }: Props = $props();

  let host: HTMLDivElement | undefined = $state();
  let connectors: SVGSVGElement | undefined = $state();
  let scene = $state.raw<ColumnsScene | undefined>();
  let dragging = $state(false);

  const layer = new LabelLayer();

  /** The tooltip: the whole label, the size, and the true share of a column drawn larger than it is. */
  function tip(column: Column): string {
    const share = column.boosted ? ` · ${formatShare(column.share)} of this folder` : '';
    const states = [
      ...(safe.has(column.node.id) ? ['safe to clean'] : []),
      ...(tileMarked(column, marked) ? ['marked for cleanup'] : []),
    ];

    return [`${column.label} · ${fmt.size(column.node.allocated)}${share}`, ...states].join(' · ');
  }

  /** The label of a column: a chain is written whole when it fits, else by its first folder, cut at the end if need be. */
  const labelOf = (column: Column): string =>
    fitTitle(
      (column.chain ?? [column.node]).map((link) => link.name),
      LABEL_WIDTH,
      'label',
    ) || column.node.name;

  // three.js is a chunk of its own: nothing of it is loaded until the 3D view is first asked for.
  onMount(() => {
    let cancelled = false;
    let created: ColumnsScene | undefined;

    if (connectors) {
      layer.attach(connectors);
    }

    (async () => {
      try {
        const { ColumnsScene } = await import('./three/scene');

        if (cancelled || !host) {
          return;
        }

        created = new ColumnsScene(
          host,
          {
            onHover: (id) => onhover(id),
            onPick: (id) => (id === null ? onclear() : pick(id)),
            onDrag: (active) => (dragging = active),
            onFrame: drawLabels,
          },
          { keepDrawing: hooks.static, animate: !motion.reduced },
        );
        scene = created;
      } catch {
        if (!cancelled) {
          onfail(
            'The 3D view needs WebGL, and this machine could not start it. Showing the flat map instead.',
          );
        }
      }
    })();

    return () => {
      cancelled = true;
      scene = undefined;
      created?.dispose();
    };
  });

  /** A column is chosen like a row of the list; the "smaller items" column selects its items and is never opened. */
  function pick(id: NodeId): void {
    const column = layout.columns.find((candidate) => candidate.top === id);

    if (id === COLLAPSED_NODE_ID && column) {
      ongroup(column.node, column.members);
    } else {
      onchoose(id);
    }
  }

  function drawLabels(frameInfo: SceneFrame): void {
    layer.update(frameInfo);

    if (hooks.static && host) {
      // For screenshots and tests: where the block landed inside the card, and the card's size.
      host.dataset.fit = [...frameInfo.block, frameInfo.width, frameInfo.height].map(Math.round).join(',');
    }
  }

  $effect(() => {
    scene?.setLayout(layout, colors);
  });

  $effect(() => {
    scene?.setHighlight(selected, hovered);
  });

  $effect(() => {
    layer.configure(selected, hovered, colors);
    scene?.invalidate();
  });

  /** Registers a label with the layer, which places it every frame. */
  function track(element: HTMLElement, id: NodeId): { destroy: () => void } {
    return { destroy: layer.track(id, element) };
  }
</script>

<div class="v3" class:drag={dragging} bind:this={host}>
  <svg class="lead" bind:this={connectors} aria-hidden="true"></svg>

  <div class="labels">
    {#each layout.columns as column (column.top)}
      {#if column.top === COLLAPSED_NODE_ID}
        <button
          type="button"
          class="lb"
          class:sel={column.top === selected}
          class:dim={column.top !== selected}
          aria-pressed={column.top === selected}
          title={tip(column)}
          use:track={column.top}
          onclick={() => pick(column.top)}
        >
          <b>{column.node.name}</b><span class="num">{fmt.size(column.node.allocated)}</span>
        </button>
      {:else}
        <button
          type="button"
          class="lb"
          class:sel={column.top === selected}
          class:dim={selected !== null && column.top !== selected}
          class:safe={safe.has(column.node.id)}
          class:mk={tileMarked(column, marked)}
          aria-pressed={column.top === selected}
          aria-label={tip(column)}
          title={tip(column)}
          use:track={column.top}
          onclick={() => onchoose(column.top)}
          onfocus={() => onhover(column.top)}
          onblur={() => onhover(null)}
        >
          <b>{labelOf(column)}</b><span class="num"
            >{#if safe.has(column.node.id)}<i class="tag">SAFE</i> ·
            {/if}{fmt.size(column.node.allocated)}</span
          >
        </button>
      {/if}
    {/each}
  </div>

  <div class="how">
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 8a5.5 5.5 0 1 0 1.7-4M2.5 2.5V5H5" /></svg>
    Drag to rotate · scroll to zoom
  </div>

  <Loading when={scene === undefined} label="Preparing the 3D view…" />
  {#if layout.columns.length === 0}
    <MapMessage title="Nothing to draw here">
      The items in this folder take no space, so there are no columns. The list on the right still shows them.
    </MapMessage>
  {/if}
</div>

<style>
  .v3 {
    position: absolute;
    inset: 0;
    cursor: grab;
    user-select: none;
    touch-action: none;
  }

  .v3.drag {
    cursor: grabbing;
  }

  .v3 :global(canvas) {
    display: block;
  }

  .lead,
  .labels {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
  }

  .lead {
    width: 100%;
    height: 100%;
  }

  /* Placed by the label layer every frame: left and top are its bottom centre. */
  .lb {
    position: absolute;
    left: 0;
    top: 0;
    transform: translate(-50%, -100%);
    white-space: nowrap;
    font-size: 12px;
    line-height: 1.25;
    color: #fff;
    background: #0a0e17d9;
    border: 1px solid #ffffff22;
    border-radius: 5px;
    padding: 3px 7px;
    text-align: center;
    transition: opacity 0.2s;
    visibility: hidden;
  }

  .lb b {
    display: block;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--mono);
    font-weight: 600;
  }

  .lb span {
    color: #c3cbd9;
  }

  .lb.dim {
    opacity: 0.35;
  }

  .lb.safe {
    border-color: var(--green) !important;
  }

  .lb.sel {
    border-color: #fff !important;
  }

  /* Marked for cleanup: a dashed border, over the others. */
  .lb.mk {
    border-style: dashed;
    border-color: #fff !important;
  }

  .lb .tag {
    font-style: normal;
    font-weight: 700;
    font-size: 10.5px;
    letter-spacing: 0.06em;
    color: var(--green);
  }

  .lb:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .how {
    position: absolute;
    left: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: var(--dim);
    pointer-events: none;
  }

  .how svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
