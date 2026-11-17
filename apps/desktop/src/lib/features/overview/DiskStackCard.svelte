<script lang="ts">
  import Card from '../../components/ui/Card.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { formatShare } from '../../format';
  import type { NodeId } from '../../api/types';
  import type { Slab } from './slabs';
  import { layoutStack, STACK_VIEWBOX, type PlacedSlab } from './stack';

  interface Props {
    slabs: Slab[];
    /** What 100% is: the volume size, or the scanned size. */
    total: number;
    /** A slab was opened; `color` is the colour of its folder. */
    onopen: (nodeId: NodeId, color: string) => void;
  }

  let { slabs, total, onopen }: Props = $props();

  const layout = $derived(layoutStack(slabs, total));

  /** Index of the slab the pointer or keyboard focus is on; the others dim. */
  let hot = $state<number | null>(null);

  const mix = (color: string, percent: number): string =>
    `color-mix(in srgb, ${color} ${percent}%, var(--bg))`;
</script>

<Card title="What is on the disk" hint="layer thickness = space used" grow>
  <div class="frame">
    <svg
      class="stack"
      class:hov={hot !== null}
      viewBox="0 0 {STACK_VIEWBOX.width} {STACK_VIEWBOX.height}"
      style="max-width:{STACK_VIEWBOX.width}px"
      role="group"
      aria-label="Layers of the disk, biggest at the bottom"
    >
      <polygon points={layout.base} fill="none" stroke="var(--line)" />

      {#each layout.slabs as placed (placed.slab.key)}
        {@const slab = placed.slab}
        {#if slab.nodeId !== undefined}
          {@const nodeId = slab.nodeId}
          <g
            class="slab link"
            class:hot={hot === placed.index}
            style="--i:{placed.index}"
            role="button"
            tabindex="0"
            aria-label="{slab.name}, {fmt.size(slab.size)}. Open in the Explorer"
            onmouseenter={() => (hot = placed.index)}
            onmouseleave={() => (hot = null)}
            onfocus={() => (hot = placed.index)}
            onblur={() => (hot = null)}
            onclick={() => onopen(nodeId, slab.color)}
            onkeydown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                onopen(nodeId, slab.color);
              }
            }}
          >
            {@render body(placed)}
          </g>
        {:else}
          <g
            class="slab"
            class:hot={hot === placed.index}
            style="--i:{placed.index}"
            role="img"
            aria-label="{slab.name}, {fmt.size(slab.size)}"
            onmouseenter={() => (hot = placed.index)}
            onmouseleave={() => (hot = null)}
          >
            {@render body(placed)}
          </g>
        {/if}
      {/each}
    </svg>
  </div>
</Card>

{#snippet body(placed: PlacedSlab)}
  {@const slab = placed.slab}
  {#if slab.free}
    {#each [placed.left, placed.right, placed.top] as face (face)}
      <polygon points={face} fill="var(--wash-1)" stroke={slab.color} stroke-dasharray="4 4" />
    {/each}
  {:else}
    <polygon points={placed.left} style="fill:{mix(slab.color, 55)}" />
    <polygon points={placed.right} style="fill:{mix(slab.color, 36)}" />
    <polygon points={placed.top} style="fill:{mix(slab.color, 88)}" />
  {/if}

  <polyline
    points="{placed.leader.x1},{placed.leader.y} {placed.leader.x2},{placed.leader.y}"
    fill="none"
    style="stroke:{slab.color}"
  />
  <circle cx={placed.leader.x1} cy={placed.leader.y} r="3" style="fill:{slab.color}" />

  <text class="nm" x={placed.text.x} y={placed.text.y - 6} text-anchor={placed.text.anchor}>
    <title>{slab.name}</title>
    {placed.name}
  </text>
  <text class="sz" x={placed.text.x} y={placed.text.y + 11} text-anchor={placed.text.anchor}>
    {fmt.size(slab.size)} · {formatShare(slab.share)}
  </text>
  {#if placed.inside !== ''}
    <text class="in" x={placed.text.x} y={placed.text.y + 27} text-anchor={placed.text.anchor}
      >{placed.inside}</text
    >
  {/if}
{/snippet}

<style>
  /*
   * The stack is the main visual of the page and sets the height of the top area: it is drawn at its natural size
   * (1 viewBox unit = 1 px) and only scales down, proportionally, when the card is narrower than the drawing.
   * When the column next to it is taller, the card grows and the drawing stays centred in it.
   */
  .frame {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .stack {
    display: block;
    width: 100%;
    height: auto;
    /* A long name may reach a few units past the viewBox; the card's padding has room for it. */
    overflow: visible;
  }

  .slab {
    transition: opacity 0.2s;
    animation: drop 0.7s cubic-bezier(0.2, 0.9, 0.3, 1.1) both;
    animation-delay: calc(var(--i) * 110ms);
    outline: none;
  }

  .link {
    cursor: pointer;
  }

  .hov .slab:not(.hot) {
    opacity: 0.3;
  }

  /* The focus ring of an SVG group is the highlight itself: the others dim. */
  :global(:root:not([data-input='pointer'])) .slab:focus-visible .nm {
    text-decoration: underline;
  }

  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-70px);
    }
  }

  text {
    font-family: var(--font);
    fill: var(--text);
  }

  .nm {
    font-family: var(--mono);
    font-size: 14px;
    font-weight: 600;
  }

  .sz {
    font-size: 13px;
    fill: var(--muted);
  }

  .in {
    font-size: 11.5px;
    fill: var(--dim);
  }
</style>
