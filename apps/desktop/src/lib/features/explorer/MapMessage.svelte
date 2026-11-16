<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from '../../components/ui/Icon.svelte';

  interface Props {
    title: string;
    icon?: IconName;
    kind?: 'info' | 'error';
    children?: Snippet;
  }

  let { title, icon = 'explorer', kind = 'info', children }: Props = $props();
</script>

<!-- Fills the map area when there is nothing to draw: an empty folder, an unreadable one, an error. -->
<div class="message" class:error={kind === 'error'} role={kind === 'error' ? 'alert' : 'status'}>
  <Icon name={icon} size={26} stroke={1.3} />
  <b>{title}</b>
  {#if children}<p>{@render children()}</p>{/if}
</div>

<style>
  .message {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 8px;
    padding: 24px;
    text-align: center;
    color: var(--muted);
    font-size: 12.5px;
  }

  .message :global(svg) {
    color: var(--dim);
  }

  .error :global(svg) {
    color: var(--red);
  }

  b {
    color: var(--text);
    font-size: 14px;
  }

  p {
    max-width: 360px;
    line-height: 1.5;
  }
</style>
