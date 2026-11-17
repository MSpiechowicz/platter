<script lang="ts">
  interface Props {
    /** How far along, from 0 to `max`. */
    value: number;
    max: number;
    tone?: 'cyan' | 'slate';
    /** A gentle fade in and out, for a figure that is still being waited for. */
    pulse?: boolean;
    /** Names the ring for assistive tools. Without it the ring is decoration. */
    label?: string;
  }

  let { value, max, tone = 'cyan', pulse = false, label }: Props = $props();

  const RADIUS = 19;
  const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

  // The ring starts empty and fills a moment after it appears, so that the fill is seen.
  let armed = $state(false);

  $effect(() => {
    const timer = setTimeout(() => (armed = true), 120);

    return () => clearTimeout(timer);
  });

  const fraction = $derived(armed ? Math.min(1, Math.max(0, value / max)) : 0);
</script>

<svg
  class="ring {tone}"
  class:pulse
  width="46"
  height="46"
  viewBox="0 0 46 46"
  role={label ? 'progressbar' : undefined}
  aria-label={label}
  aria-valuemin={label ? 0 : undefined}
  aria-valuemax={label ? max : undefined}
  aria-valuenow={label ? value : undefined}
  aria-hidden={label ? undefined : true}
>
  <circle class="track" cx="23" cy="23" r={RADIUS} fill="none" stroke-width="5" />
  {#if value > 0}
    <circle
      class="fill"
      cx="23"
      cy="23"
      r={RADIUS}
      fill="none"
      stroke-width="5"
      stroke-dasharray={CIRCUMFERENCE}
      stroke-dashoffset={CIRCUMFERENCE * (1 - fraction)}
    />
  {/if}
</svg>

<style>
  .ring {
    flex: none;
    /* The fill starts at the top. */
    transform: rotate(-90deg);
  }

  .track {
    stroke: var(--track);
  }

  .fill {
    stroke: var(--cyan);
    stroke-linecap: round;
    transition: stroke-dashoffset 900ms var(--ease-out);
  }

  .slate .fill {
    stroke: var(--slate);
  }

  .pulse .fill {
    animation: pulse 2.4s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.55;
    }
  }
</style>
