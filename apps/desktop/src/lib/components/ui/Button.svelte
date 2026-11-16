<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  interface Props extends HTMLButtonAttributes {
    variant?: 'default' | 'primary';
    size?: 'md' | 'sm';
    children: Snippet;
  }

  let {
    variant = 'default',
    size = 'md',
    type = 'button',
    class: className = '',
    children,
    ...rest
  }: Props = $props();
</script>

<button class="btn {variant} {size} {className}" {type} {...rest}>{@render children()}</button>

<style>
  .btn {
    padding: 8px 14px;
    border-radius: 7px;
    border: 1px solid var(--line);
    background: var(--card2);
    font-weight: 600;
    font-size: 13px;
    white-space: nowrap;
    transition:
      border-color 0.15s,
      background 0.15s;
  }

  .btn:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .primary {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
  }

  .primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .sm {
    padding: 5px 8px;
    font-size: 12px;
  }

  .btn:disabled {
    opacity: 0.4;
  }
</style>
