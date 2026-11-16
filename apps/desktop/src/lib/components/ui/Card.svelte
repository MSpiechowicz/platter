<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';

  interface Props extends HTMLAttributes<HTMLElement> {
    title?: string;
    /** Small grey note on the right of the title. */
    hint?: string;
    /** Take the spare height of a flex column. */
    grow?: boolean;
    children: Snippet;
  }

  let { title, hint, grow = false, class: className = '', children, ...rest }: Props = $props();

  const headingId = $props.id();
</script>

<section class="card {className}" class:grow aria-labelledby={title ? headingId : undefined} {...rest}>
  {#if title}
    <h3 id={headingId}>
      {title}
      {#if hint}
        <span class="sp"></span>
        <span class="hint">{hint}</span>
      {/if}
    </h3>
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

  h3 {
    font-size: 11px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--muted);
    font-weight: 600;
    margin-bottom: 12px;
    display: flex;
    align-items: center;
    gap: 8px;
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
</style>
