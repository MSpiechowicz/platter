<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /** `safe`, `review` and `keep` are the cleanup verdicts; `neutral` is a plain label. */
    kind?: 'neutral' | 'safe' | 'review' | 'keep';
    children: Snippet;
  }

  let { kind = 'neutral', children }: Props = $props();
</script>

<span class="tag {kind}">{@render children()}</span>

<style>
  .tag {
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 2px 6px;
    border-radius: 4px;
    white-space: nowrap;
    border: 1px solid currentColor;
    color: var(--muted);
  }

  .safe {
    color: var(--green);
  }

  .review {
    color: var(--amber);
  }

  .keep {
    color: var(--red);
  }
</style>
