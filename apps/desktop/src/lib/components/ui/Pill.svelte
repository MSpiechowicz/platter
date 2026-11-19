<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /** A small accent dot in front of the text. */
    dot?: boolean;
    /** The dot is grey: the thing the pill describes is not active. */
    idle?: boolean;
    /** Fill the free width of a flex row. */
    grow?: boolean;
    /** Something is under way: the dot pulses. */
    busy?: boolean;
    /** The text is cut with "…" when the pill has no room for all of it, and only then. Give `title` the whole. */
    truncate?: boolean;
    /** The tooltip: the whole text, for a pill that may be cut. */
    title?: string;
    children: Snippet;
  }

  let {
    dot = false,
    idle = false,
    grow = false,
    busy = false,
    truncate = false,
    title = undefined,
    children,
  }: Props = $props();
</script>

<div class="pill" class:grow class:truncate {title}>
  {#if dot}<span class="dot" class:idle class:busy></span>{/if}
  <span class="t">{@render children()}</span>
</div>

<style>
  .pill {
    border: 1px solid var(--line);
    border-radius: 7px;
    padding: 7px 12px;
    font-size: 12px;
    color: var(--muted);
    display: flex;
    gap: 8px;
    align-items: center;
    white-space: nowrap;
  }

  /* A pill that fills a row centres what it says, so the text does not sit at one end of a wide box. */
  /* The text's box is trimmed to its capitals and baseline, so the letters (not the font's ascender space) are
     centred in the pill and on the dot. */
  .t {
    text-box: trim-both cap alphabetic;
  }

  .grow {
    flex: 1;
    justify-content: center;
  }

  /* A pill that may be cut: it can shrink below its text, and the text ends in "…". The trimmed box would clip
     the letters' descenders once the overflow is hidden, so it keeps its full line box. */
  .truncate {
    min-width: 0;
  }

  .truncate .t {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    text-box: normal;
  }

  .pill :global(b) {
    color: var(--text);
    font-weight: 600;
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    flex: none;
  }

  .dot.idle {
    background: var(--dim);
  }

  .dot.busy {
    background: var(--accent);
    animation: pulse 1.1s ease-in-out infinite;
  }

  @keyframes pulse {
    0%,
    100% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 60%, transparent);
      opacity: 1;
    }
    50% {
      box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent) 0%, transparent);
      opacity: 0.55;
    }
  }

  :global(:root[data-motion='reduce']) .dot.busy {
    animation: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .dot.busy {
      animation: none;
    }
  }
</style>
