<script lang="ts">
  import { afterPaint } from '../../motion/afterPaint';
  import { motion } from '../../state/motion.svelte';

  interface Props {
    /** Filled share, 0 to 1. Ignored while `indeterminate`. */
    value?: number;
    /** A CSS colour or variable for the fill. */
    color?: string;
    size?: 'thin' | 'thick';
    /** The total is unknown: a segment slides along the track. */
    indeterminate?: boolean;
    label?: string;
  }

  let { value = 0, color = 'var(--accent)', size = 'thin', indeterminate = false, label }: Props = $props();

  // The fill starts empty and grows on first paint, unless motion is reduced.
  let armed = $state(motion.reduced);

  $effect(() => {
    if (armed) {
      return;
    }

    return afterPaint(() => (armed = true));
  });

  const percent = $derived(armed ? Math.max(0, Math.min(1, value)) * 100 : 0);
</script>

<div
  class="bar {size}"
  class:indeterminate
  role="progressbar"
  aria-label={label}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={indeterminate ? undefined : Math.round(Math.max(0, Math.min(1, value)) * 100)}
>
  <i style="--c:{color};{indeterminate ? '' : `width:${percent}%`}"></i>
</div>

<style>
  .bar {
    height: 4px;
    border-radius: 2px;
    background: var(--track);
    overflow: hidden;
    position: relative;
  }

  .thick {
    height: 14px;
    border-radius: 3px;
  }

  i {
    display: block;
    height: 100%;
    background: var(--c);
    transition: width 1.1s var(--ease-out);
  }

  .indeterminate i {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: 30%;
    border-radius: 2px;
    transition: none;
    animation: slide 1.1s ease-in-out infinite alternate;
  }

  @keyframes slide {
    from {
      left: 0;
    }
    to {
      left: 70%;
    }
  }

  /* With motion off the sliding segment would look like 30% progress: show a dim full track instead. */
  :global(:root[data-motion='reduce']) .indeterminate i {
    width: 100%;
    opacity: 0.35;
  }

  @media (prefers-reduced-motion: reduce) {
    .indeterminate i {
      width: 100%;
      opacity: 0.35;
    }
  }
</style>
