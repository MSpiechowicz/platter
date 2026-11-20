<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';

  interface Props extends HTMLAttributes<HTMLElement> {
    title?: string;
    /** Small grey note on the right of the title. */
    hint?: string;
    /** A warning pill in the place of the hint, for a card whose figures should not be trusted as they are. */
    tag?: string;
    /** Take the spare height of a flex column. */
    grow?: boolean;
    /** Buttons on the right of the title; the header keeps the height it has without them. */
    actions?: Snippet;
    children: Snippet;
  }

  let { title, hint, tag, grow = false, actions, class: className = '', children, ...rest }: Props = $props();

  const headingId = $props.id();
</script>

<section class="card {className}" class:grow aria-labelledby={title ? headingId : undefined} {...rest}>
  {#if title}
    <div class="head">
      <h3 id={headingId}>
        {title}
        {#if tag}
          <span class="sp"></span>
          <span class="tag">{tag}</span>
        {:else if hint}
          <span class="sp"></span>
          <span class="hint">{hint}</span>
        {/if}
      </h3>
      {#if actions}
        <div class="actions">{@render actions()}</div>
      {/if}
    </div>
  {/if}
  {@render children()}
</section>

<style>
  .card {
    background: var(--card);
    border: 1px solid var(--line);
    border-radius: var(--r);
    padding: 16px;
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    animation: rise 0.5s var(--ease-out) both;
  }

  .grow {
    flex: 1;
  }

  /* An open dropdown must paint above the next card. */
  .card:has(:global(.dd.open)) {
    z-index: 40;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 12px;
  }

  h3 {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--muted);
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  /* A button is taller than the title: it reaches into the card's padding, so every card's title sits at the same place. */
  .actions {
    display: flex;
    gap: 8px;
    margin-block: -10px;
  }

  .sp {
    flex: 1;
  }

  .hint {
    font-size: 10.5px;
    letter-spacing: 0.08em;
    color: var(--dim);
    white-space: nowrap;
    font-weight: 500;
  }

  .tag {
    padding: 0 6px;
    border: 1px solid color-mix(in srgb, var(--amber) 40%, transparent);
    border-radius: 5px;
    color: var(--amber);
    font-size: 10px;
    letter-spacing: 0.1em;
    font-weight: 700;
    text-transform: uppercase;
    white-space: nowrap;
  }
</style>
