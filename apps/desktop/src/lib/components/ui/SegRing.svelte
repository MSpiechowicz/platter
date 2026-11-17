<script lang="ts">
  import type { Snippet } from 'svelte';
  import { afterPaint } from '../../motion/afterPaint';
  import { motion } from '../../state/motion.svelte';

  interface Props {
    /** Filled share, 0 to 1. */
    value: number;
    size?: number;
    segments?: number;
    color?: string;
    /** Centred over the ring. */
    children?: Snippet;
  }

  let { value, size = 190, segments = 56, color = 'var(--accent)', children }: Props = $props();

  // Segments light up one after another on first paint, unless motion is reduced.
  let armed = $state(motion.reduced);

  $effect(() => {
    if (armed) {
      return;
    }

    return afterPaint(() => (armed = true));
  });

  const lit = $derived(armed ? Math.round(Math.max(0, Math.min(1, value)) * segments) : 0);
</script>

<div class="ring" style="width:{size}px;height:{size}px">
  <svg viewBox="0 0 128 128" width={size} height={size} aria-hidden="true">
    <circle
      cx="64"
      cy="64"
      r="60"
      fill="none"
      style="stroke:{color}"
      stroke-opacity=".25"
      stroke-dasharray="2 6"
    >
      {#if !motion.reduced}
        <animateTransform
          attributeName="transform"
          type="rotate"
          from="0 64 64"
          to="360 64 64"
          dur="40s"
          repeatCount="indefinite"
        />
      {/if}
    </circle>
    {#each { length: segments } as _, i (i)}
      <line
        x1="64"
        y1="12"
        x2="64"
        y2="24"
        stroke-width="4.5"
        transform="rotate({(i * 360) / segments} 64 64)"
        style="stroke:{i < lit ? color : 'var(--raised)'};transition-delay:{i * 14}ms"
      />
    {/each}
    <circle cx="64" cy="64" r="36" fill="none" style="stroke:{color}" stroke-opacity=".3" />
  </svg>
  {#if children}<div class="mid">{@render children()}</div>{/if}
</div>

<style>
  .ring {
    position: relative;
    display: grid;
    place-items: center;
  }

  svg {
    grid-area: 1 / 1;
    display: block;
  }

  line {
    transition: stroke 0.4s;
  }

  .mid {
    grid-area: 1 / 1;
    text-align: center;
  }
</style>
