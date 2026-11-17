<script lang="ts">
  import { fade, fly } from 'svelte/transition';
  import { motion } from '../../state/motion.svelte';
  import { ui } from '../../state/ui.svelte';
  import Icon, { type IconName } from './Icon.svelte';

  const ICONS: Record<'ok' | 'info' | 'error', IconName> = { ok: 'check', info: 'info', error: 'alert' };
  const COLORS = { ok: 'var(--accent)', info: 'var(--accent)', error: 'var(--red)' };
</script>

<div class="host" aria-live="polite">
  {#each ui.toasts as toast (toast.id)}
    <div
      class="toast"
      role="status"
      style="--tc:{COLORS[toast.kind]}"
      in:fly={{ x: 24, duration: motion.reduced ? 0 : 350 }}
      out:fade={{ duration: motion.reduced ? 0 : 200 }}
    >
      <span class="ti"
        ><Icon name={ICONS[toast.kind]} size={toast.kind === 'error' ? 14 : 12} stroke={2} /></span
      >
      <div>
        <b>{toast.title}</b>
        {#if toast.description}<p>{toast.description}</p>{/if}
      </div>
      <button class="tx" type="button" aria-label="Close" onclick={() => ui.dismiss(toast.id)}>
        <Icon name="close" size={10} stroke={1.6} />
      </button>
    </div>
  {/each}
</div>

<style>
  .host {
    position: fixed;
    right: 22px;
    bottom: 22px;
    z-index: 60;
    display: flex;
    flex-direction: column;
    gap: 10px;
    pointer-events: none;
  }

  .toast {
    width: 340px;
    max-width: calc(100vw - 44px);
    display: grid;
    grid-template-columns: 22px minmax(0, 1fr) 20px;
    gap: 12px;
    align-items: start;
    background: var(--card2);
    border: 1px solid var(--line);
    color: var(--text);
    padding: 14px 14px 14px 16px;
    border-radius: 10px;
    box-shadow: 0 10px 30px var(--shadow-1);
    pointer-events: auto;
  }

  .ti {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--tc) 20%, transparent);
    color: var(--tc);
  }

  b {
    display: block;
    font-size: 13.5px;
    line-height: 22px;
  }

  p {
    margin: 2px 0 0;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--muted);
  }

  .tx {
    width: 20px;
    height: 22px;
    display: grid;
    place-items: center;
    color: var(--dim);
  }

  .tx:hover {
    color: var(--text);
  }
</style>
