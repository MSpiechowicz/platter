<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /** A small accent dot in front of the text. */
    dot?: boolean;
    /** The dot is grey: the thing the pill describes is not active. */
    idle?: boolean;
    /** Fill the free width of a flex row. */
    grow?: boolean;
    children: Snippet;
  }

  let { dot = false, idle = false, grow = false, children }: Props = $props();
</script>

<div class="pill" class:grow>
  {#if dot}<span class="dot" class:idle></span>{/if}
  {@render children()}
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

  .grow {
    flex: 1;
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
    transform: translateY(-1px);
    flex: none;
  }

  .dot.idle {
    background: var(--dim);
  }
</style>
