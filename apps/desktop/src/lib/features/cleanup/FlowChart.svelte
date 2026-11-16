<script lang="ts">
  import { untrack } from 'svelte';
  import { motion } from '../../state/motion.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import {
    blendLayouts,
    canBlend,
    emptyLayout,
    layoutFlow,
    REST_ID,
    type FlowItem,
    type FlowLayout,
    type FlowNode,
  } from './flow';

  interface Props {
    items: readonly FlowItem[];
    /** What the whole chart stands for; what no group covers is drawn as "Everything else". */
    total: number;
    /** The caption and the figure of each bar on the right. A missing figure is said so. */
    freed: { caption: string; value: string | undefined };
    stays: { caption: string; value: string | undefined };
    /** A group was clicked: it moves to the other side. */
    ontoggle: (id: string) => void;
  }

  let { items, total, freed, stays, ontoggle }: Props = $props();

  /** The ribbons glide to their new places in this long when a group changes sides. */
  const GLIDE_MS = 380;
  /** A bar shorter than this has no room for its name; the whole row still has the name as its tooltip. */
  const LABEL_MIN = 11;

  // The chart is drawn at the pixel size of the box that holds it. The drawing is laid over the box and takes no
  // room of its own, so a new size can never change the box again.
  let width = $state(0);
  let height = $state(0);
  let hovered = $state<string | undefined>(undefined);

  const target = $derived(layoutFlow({ items, total, width, height }));
  let shown = $state.raw<FlowLayout>(emptyLayout(0, 0));

  $effect(() => {
    const next = target;
    const from = untrack(() => shown);

    if (motion.reduced || !canBlend(from, next)) {
      shown = next;
      return;
    }

    const start = performance.now();
    let frame = 0;

    const step = (now: number): void => {
      const progress = Math.max(0, Math.min(1, (now - start) / GLIDE_MS));

      shown = blendLayouts(from, next, 1 - Math.pow(1 - progress, 3));

      if (progress < 1) {
        frame = requestAnimationFrame(step);
      }
    };

    frame = requestAnimationFrame(step);

    return () => cancelAnimationFrame(frame);
  });

  const nameOf = (node: FlowNode): string => (node.id === REST_ID ? 'Everything else' : node.label);

  /** The row's text apart from its size: a kept group, and one that was not cleaned properly, say so. */
  function suffixOf(node: FlowNode): string {
    if (node.kind === 'keep') {
      return ' (kept)';
    }

    return node.progress === 'problem' ? ' (problem)' : '';
  }

  /** The caption and figure sit on the bar's middle, and never above the box. */
  const endTop = (bar: { y: number; h: number }): number => Math.max(0, bar.y + bar.h / 2 - 18);

  function opacityOf(node: FlowNode, hot: boolean): number {
    if (node.progress === 'done') {
      return hot ? 0.9 : 0.8;
    }

    if (node.side === 'freed') {
      return hot ? 0.7 : 0.5;
    }

    return hot ? 0.3 : 0.16;
  }
</script>

<div class="flow" bind:clientWidth={width} bind:clientHeight={height} role="group" aria-label="Cleanup chart">
  {#if shown.nodes.length > 0}
    <svg
      width={shown.width}
      height={shown.height}
      viewBox="0 0 {shown.width} {shown.height}"
      aria-hidden="true"
    >
      {#each shown.nodes as node (node.id)}
        {@const hot = hovered === node.id && node.clickable}

        <g class="node {node.kind}">
          {#if node.clickable}
            <!-- The mouse can click the ribbon too; the keyboard and screen readers use the row's button. -->
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <path
              class="ribbon hit"
              d={node.path}
              fill-opacity={opacityOf(node, hot)}
              onclick={() => ontoggle(node.id)}
              onpointerenter={() => (hovered = node.id)}
              onpointerleave={() => (hovered = undefined)}
            />
          {:else}
            <path class="ribbon" d={node.path} fill-opacity={opacityOf(node, false)} />
          {/if}

          <rect
            class="bar"
            x={shown.barX}
            y={node.y}
            width={shown.barW}
            height={node.h}
            rx="2"
            fill-opacity={node.kind === 'keep' ? 0.45 : 1}
          />
        </g>
      {/each}

      {#if shown.freed}
        <rect
          class="end freed"
          x={shown.endX}
          y={shown.freed.y}
          width={shown.barW}
          height={shown.freed.h}
          rx="2"
        />
      {/if}

      {#if shown.stays}
        <rect
          class="end stays"
          x={shown.endX}
          y={shown.stays.y}
          width={shown.barW}
          height={shown.stays.h}
          rx="2"
        />
      {/if}
    </svg>

    <div class="rows">
      {#each shown.nodes as node (node.id)}
        {@const text = `${nameOf(node)}${suffixOf(node)}`}
        {@const size = fmt.size(node.size)}
        {@const style = `top:${node.y - 2}px;height:${node.h + 4}px;width:${shown.barX + shown.barW}px;padding-right:${shown.barW + 12}px`}

        {#snippet row()}
          {#if node.h >= LABEL_MIN}
            <span
              class="name"
              class:fixed={!node.clickable}
              class:gone={node.progress === 'done'}
              class:hot={hovered === node.id && node.clickable}>{text}</span
            >
            <b class="num size">{size}</b>
          {/if}
        {/snippet}

        {#if node.clickable}
          <button
            type="button"
            class="row hit"
            role="checkbox"
            aria-checked={node.side === 'freed'}
            aria-label="{text}, {size}"
            title="{text}: {size}"
            {style}
            onclick={() => ontoggle(node.id)}
            onpointerenter={() => (hovered = node.id)}
            onpointerleave={() => (hovered = undefined)}
          >
            {@render row()}
          </button>
        {:else}
          <div class="row" title="{text}: {size}" {style}>
            {@render row()}
          </div>
        {/if}
      {/each}

      {#if shown.freed}
        <div class="endtext" style="top:{endTop(shown.freed)}px;left:{shown.endX + shown.barW + 12}px">
          <span class="cap freed">{freed.caption}</span>
          <b class="num">{freed.value ?? 'Not measured'}</b>
        </div>
      {/if}

      {#if shown.stays}
        <div class="endtext" style="top:{endTop(shown.stays)}px;left:{shown.endX + shown.barW + 12}px">
          <span class="cap">{stays.caption}</span>
          <b class="num">{stays.value ?? 'Not measured'}</b>
        </div>
      {/if}
    </div>
  {:else if width > 0}
    <p class="none">Nothing to show: the scan found no bytes.</p>
  {/if}
</div>

<style>
  /* The box takes the spare height of the card; the drawing is laid over it. */
  .flow {
    position: relative;
    flex: 1;
    min-height: 420px;
  }

  svg {
    position: absolute;
    inset: 0;
    display: block;
  }

  .safe {
    --c: var(--green);
  }

  .review {
    --c: var(--amber);
  }

  .keep {
    --c: var(--red);
  }

  .rest {
    --c: var(--slate);
  }

  .ribbon,
  .bar {
    fill: var(--c);
  }

  .ribbon {
    transition: fill-opacity 0.2s;
  }

  .hit {
    cursor: pointer;
  }

  /* The bars where the ribbons end. "Will be freed" mixes safe and review groups, so it is not green. */
  .end.freed {
    fill: var(--accent);
  }

  .end.stays {
    fill: var(--slate);
  }

  .rows {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  /* One row per group: the name and size end at the bar. A button, so the keyboard can move the group too. */
  .row {
    position: absolute;
    left: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    font-size: 12.5px;
    line-height: 1.2;
    text-align: right;
    white-space: nowrap;
    border-radius: 4px;
  }

  button.row {
    pointer-events: auto;
    cursor: pointer;
  }

  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: color 0.15s;
  }

  .name.fixed {
    color: var(--muted);
  }

  .name.gone {
    color: var(--dim);
    text-decoration: line-through;
  }

  .name.hot {
    color: #fff;
  }

  .size {
    flex: none;
    font-weight: 700;
  }

  .endtext {
    position: absolute;
    display: flex;
    flex-direction: column;
    gap: 1px;
    white-space: nowrap;
  }

  .endtext b {
    font-size: 20px;
    line-height: 1.2;
  }

  .cap {
    font-size: 11px;
    letter-spacing: 0.1em;
    font-weight: 600;
    color: var(--muted);
  }

  .cap.freed {
    color: var(--accent);
  }

  .none {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    color: var(--muted);
    font-size: 12.5px;
  }
</style>
