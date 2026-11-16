<script lang="ts">
  import { tip } from './tooltip.svelte';

  let el: HTMLDivElement | undefined = $state();
  let left = $state(0);
  let top = $state(0);
  let shown = $state(false);

  const MARGIN = 8;

  // Place the tip above its target, or below when there is no room, and keep it inside the window.
  $effect(() => {
    const target = tip.target;

    if (!target || !el) {
      shown = false;
      return;
    }

    const anchor = target.getBoundingClientRect();
    const width = el.offsetWidth;
    const height = el.offsetHeight;
    const above = anchor.top - height - MARGIN;

    left = Math.max(
      MARGIN,
      Math.min(window.innerWidth - width - MARGIN, anchor.left + anchor.width / 2 - width / 2),
    );
    top = above < MARGIN ? anchor.bottom + MARGIN : above;
    shown = true;
  });
</script>

{#if tip.target}
  <div class="tip" class:show={shown} role="tooltip" bind:this={el} style="left:{left}px;top:{top}px">
    {#if tip.content.title}<b>{tip.content.title}</b>{/if}
    {#each tip.content.items ?? [] as item, index (index)}<span class="item">{item}</span>{/each}
    {#if tip.content.text}{tip.content.text}{/if}
    {#if tip.content.note}<small>{tip.content.note}</small>{/if}
  </div>
{/if}

<style>
  .tip {
    position: fixed;
    z-index: 70;
    max-width: 260px;
    background: var(--card2);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 10px 12px;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--muted);
    box-shadow: 0 8px 24px #0008;
    opacity: 0;
    transition: opacity 0.15s;
    pointer-events: none;
  }

  .show {
    opacity: 1;
  }

  b {
    display: block;
    color: var(--text);
    margin-bottom: 4px;
  }

  .item {
    display: block;
    padding-left: 12px;
    position: relative;
  }

  .item::before {
    content: '';
    position: absolute;
    left: 0;
    top: calc(0.725em - 1.5px);
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent);
  }

  small {
    display: block;
    margin-top: 7px;
    padding-top: 7px;
    border-top: 1px solid var(--line);
    font-size: 11.5px;
    color: var(--dim);
  }
</style>
