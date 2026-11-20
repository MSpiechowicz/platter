<script lang="ts">
  import Button from '../../components/ui/Button.svelte';
  import { formatAgo } from '../../format';
  import { fmt } from '../../state/fmt.svelte';
  import { motion } from '../../state/motion.svelte';

  /*
   * What the Advisor shows in place of its list while the scan is out of date: a stack of disks, the outline of the
   * old scan lifting off the top one, and a few words. Rescan is the banner's alone. The one button it may hold is
   * Undo, while the last cleanup can still be put back: the footer that used to offer it is not shown now, so the
   * words about how an undo went are here too. The illustration is drawn with the same shapes as the one of the
   * model download (AdvisorWelcome).
   */
  interface Props {
    /** What the last cleanup of this scan freed, in bytes; undefined when it is not known (after a restart or an undo). */
    freed?: number;
    /** When that cleanup ended, in milliseconds since the epoch. */
    endedAt?: number;
    /** Offers Undo, and is called by a click on it. Left out when nothing can be put back. */
    onundo?: () => void;
    /** How an undo went, as plain lines: what was put back, what was not. */
    undone?: string[];
  }

  let { freed, endedAt, onundo, undone = [] }: Props = $props();

  let report: HTMLElement | undefined = $state();

  // Undo goes away once clicked. The keyboard moves to the lines that say how it went, not to the top of the page.
  function undo(): void {
    report?.focus({ preventScroll: true });
    onundo?.();
  }

  /** The words change by the minute, so it looks again once a minute. */
  const REFRESH_MS = 60_000;

  let now = $state(Date.now());

  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), REFRESH_MS);

    return () => clearInterval(timer);
  });

  const reduced = $derived(motion.reduced);

  // The words fade in one after the other. What appears later (an Undo that comes back after a refusal) just shows.
  const INTRO_MS = 2_600;

  let fresh = $state(true);

  $effect(() => {
    const timer = setTimeout(() => (fresh = false), INTRO_MS);

    return () => clearTimeout(timer);
  });

  const cleaned = $derived(
    freed !== undefined && freed > 0 && endedAt !== undefined ? { freed, endedAt } : undefined,
  );

  // The disk stack, bottom layer first: the y of its top face. One side's height apart, so each layer hides the one
  // below and only one seam shows between them.
  const LAYERS = [128, 96, 64];

  // The outline of the old scan rests this far above the top disk's lid. The `lift` keyframes start the same distance down.
  const LIFT = 24;
  const GHOST_Y = Math.min(...LAYERS) - LIFT;
</script>

<div class="hero">
  <svg
    class="illu"
    class:enter={!reduced}
    viewBox="0 0 240 190"
    role="img"
    aria-label="A stack of disks with the dashed outline of the old scan above it"
  >
    {#each LAYERS as y, layer (y)}
      <g class="disk" style="--i:{layer}">
        <path class="side" d="M42 {y}v28a78 22 0 0 0 156 0v-28" />
        <ellipse class="top" cx="120" cy={y} rx="78" ry="22" />
        <ellipse class="core" cx="120" cy={y} rx="34" ry="9" />
        <path class="slot" d="M62 {y + 28.7}A78 22 0 0 0 100 {y + 35.3}" />
        <circle class="led" cx="168" cy={y + 31.3} r="3.4" />
      </g>
    {/each}

    <g class="ghost-wrap">
      <ellipse class="ghost" cx="120" cy={GHOST_Y} rx="78" ry="22" />
      <path class="ghost" d="M42 {GHOST_Y}v14M198 {GHOST_Y}v14" />
    </g>

    <g transform="translate(198 {GHOST_Y - 6})">
      <g class="badge">
        <circle class="badge-core" r="18" />
        <path class="mark" d="M-7 -1a7.5 7.5 0 0 1 13 -4.5M7 1a7.5 7.5 0 0 1 -13 4.5" />
        <path class="mark" d="M6.5 -10v5h-5M-6.5 10v-5h5" />
      </g>
    </g>
  </svg>

  <div class="words" class:enter={!reduced} class:fresh>
    <h4 class="one">The scan is out of date</h4>

    <p class="two">
      {#if cleaned}
        You cleaned {fmt.size(cleaned.freed)}
        {formatAgo(now - cleaned.endedAt)}. These suggestions came from the scan before that, so nothing can
        be cleaned from them.
      {:else}
        A cleanup changed the disk after this scan. These suggestions came from before it, so nothing can be
        cleaned from them.
      {/if}
    </p>

    <p class="note three">Rescan from the banner above to see what is left.</p>

    <!-- The live region stays in place either way, so a screen reader announces the lines when they come. -->
    <div class="report" role="status" tabindex="-1" bind:this={report}>
      {#each undone as line, index (index)}
        <p class="detail">{line}</p>
      {/each}
    </div>

    {#if onundo}
      <div class="acts four">
        <Button size="sm" onclick={undo}>Undo</Button>
      </div>
    {/if}
  </div>
</div>

<style>
  .hero {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 0 8px 12px;
  }

  .words {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-width: 0;
  }

  h4 {
    margin-top: 14px;
    font-size: 18px;
    line-height: 1.25;
    font-weight: 700;
  }

  p {
    margin-top: 6px;
    max-width: 290px;
    color: var(--muted);
    font-size: 13.5px;
    line-height: 1.5;
  }

  .note {
    margin-top: 12px;
    color: var(--dim);
    font-size: 12px;
  }

  /* It takes the keyboard after Undo only to keep it on the card: no ring on words that cannot be used. */
  .report:focus {
    outline: none;
  }

  .detail {
    margin-top: 6px;
    font-size: 12px;
    color: var(--dim);
    overflow-wrap: anywhere;
  }

  .acts {
    margin-top: 12px;
  }

  /* The illustration: flat fills and strokes, no blur and no glow. */
  .illu {
    width: 200px;
    height: auto;
    max-width: 100%;
  }

  .side {
    fill: var(--card2);
    stroke: var(--rim);
    stroke-width: 1.5;
  }

  .top {
    fill: var(--face);
    stroke: var(--rim);
    stroke-width: 1.5;
  }

  .core {
    fill: none;
    stroke: var(--rim);
    stroke-width: 1.5;
  }

  .slot {
    fill: none;
    stroke: var(--rim);
    stroke-width: 2;
    stroke-linecap: round;
  }

  .led {
    fill: var(--dim);
  }

  /* The outline of the old scan. */
  .ghost {
    fill: none;
    stroke: var(--amber);
    stroke-opacity: 0.7;
    stroke-width: 1.5;
    stroke-dasharray: 4 5;
  }

  .badge-core {
    fill: var(--amber);
  }

  .mark {
    fill: none;
    stroke: var(--on-accent);
    stroke-width: 2.2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .badge {
    transform-box: fill-box;
    transform-origin: center;
  }

  /*
   * Once, about two seconds, and then it holds: the disks drop in from the bottom, the outline appears on the top
   * one and lifts off it, the badge lands, the words fade up. Every keyframe has a `from` only, so the style at rest
   * is the last frame, and so it is when the app turns animations off.
   */
  .enter .disk {
    animation: drop 0.45s cubic-bezier(0.2, 0.8, 0.3, 1) both;
    animation-delay: calc(var(--i) * 0.13s);
  }

  .enter .ghost-wrap {
    animation:
      ghost-in 0.3s ease-out 0.55s both,
      lift 0.7s cubic-bezier(0.3, 0.7, 0.2, 1) 0.95s both;
  }

  .enter .badge {
    animation: pop 0.45s cubic-bezier(0.3, 1.6, 0.5, 1) 1.6s both;
  }

  .enter .one,
  .enter .two,
  .enter .three,
  .enter.fresh .four {
    animation: fade 0.4s ease-out both;
  }

  .enter .one {
    animation-delay: 1.75s;
  }

  .enter .two {
    animation-delay: 1.85s;
  }

  .enter .three {
    animation-delay: 1.95s;
  }

  .enter.fresh .four {
    animation-delay: 2.05s;
  }

  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-22px);
    }
  }

  @keyframes ghost-in {
    from {
      opacity: 0;
    }
  }

  @keyframes lift {
    from {
      transform: translateY(24px);
    }
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.3);
    }
  }

  @keyframes fade {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
</style>
