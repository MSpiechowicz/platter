<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  interface Props {
    kind?: 'info' | 'error';
    children: Snippet;
  }

  let { kind = 'info', children }: Props = $props();
</script>

<div class="notice {kind}" role={kind === 'error' ? 'alert' : 'status'}>
  <Icon name={kind === 'error' ? 'alert' : 'info'} size={15} stroke={1.5} />
  <span>{@render children()}</span>
</div>

<style>
  .notice {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--card);
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--muted);
  }

  .notice :global(svg) {
    margin-top: 1px;
    color: var(--accent);
  }

  .error :global(svg) {
    color: var(--red);
  }
</style>
