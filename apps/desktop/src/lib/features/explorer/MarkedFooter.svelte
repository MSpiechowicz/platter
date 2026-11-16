<script lang="ts">
  import Button from '../../components/ui/Button.svelte';
  import { countUp } from '../../motion/countUp.svelte';
  import type { Action, CleanupStore } from '../../state/cleanup.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { footerStatus } from '../overview/cleanupText';
  import { countText, hasStatus } from './markedText';

  interface Props {
    /** The one cleanup flow of the app: the footer only shows it and passes the user's clicks on. */
    store: CleanupStore;
  }

  let { store }: Props = $props();

  const ACTION_LABELS: Record<Action, string> = { cancel: 'Cancel', undo: 'Undo', rescan: 'Rescan' };

  const items = $derived(store.outermostMarks.length);
  const total = countUp(() => store.markedBytes, 300);
  const status = $derived(footerStatus(store));
  // While something is going on, or has just been said, the status takes the place of the total.
  const talking = $derived(hasStatus(store));
  const small = $derived(countText(items, store.marks.size - items));

  function run(action: Action): void {
    if (action === 'cancel') {
      void store.cancel();
    } else if (action === 'undo') {
      void store.undo();
    } else {
      void store.rescan();
    }
  }
</script>

<div class="total">
  <div class="trow">
    {#if talking}
      <div class="status" class:attention={status.attention} role="status">
        <div class="sbody">
          <p class="msg">{status.text}</p>

          {#each status.details as line, index (index)}
            <p class="detail">{line}</p>
          {/each}
        </div>
      </div>
    {:else}
      <div class="sel">
        <div class="label">MARKED FOR CLEANUP</div>
        <b class="num">{fmt.size(total.value)}</b>
        <small>{small}</small>
      </div>
    {/if}

    {#if status.actions.length > 0}
      <div class="acts">
        {#each status.actions as action (action)}
          <button type="button" class="act" onclick={() => run(action)}>{ACTION_LABELS[action]}</button>
        {/each}
      </div>
    {:else}
      <Button variant="primary" disabled={!store.canCleanMarked} onclick={() => void store.cleanMarked()}>
        {items > 0 ? `Clean (${items})` : 'Clean'}
      </Button>
    {/if}
  </div>
</div>

<style>
  .total {
    margin-top: 12px;
    padding-top: 14px;
    border-top: 1px solid var(--line);
  }

  /* A fixed height: the text can change as it likes, nothing around it moves. */
  .trow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    height: 72px;
  }

  .sel {
    min-width: 0;
  }

  .label {
    font-size: 11px;
    letter-spacing: 0.12em;
    color: var(--muted);
  }

  .sel b {
    display: block;
    font-size: 22px;
    line-height: 1.3;
  }

  .sel small {
    display: block;
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .trow > :global(.btn) {
    flex: none;
  }

  .status {
    flex: 1;
    align-self: stretch;
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow-y: auto;
    font-size: 12.5px;
    line-height: 1.4;
    color: var(--muted);
  }

  .status.attention {
    color: var(--amber);
  }

  /* Auto margins centre the text while it fits, and give way to scrolling when it does not. */
  .sbody {
    margin-block: auto;
  }

  .detail {
    margin-top: 3px;
    font-size: 11.5px;
    color: var(--dim);
    overflow-wrap: anywhere;
  }

  .acts {
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 6px;
  }

  .act {
    padding: 0;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--accent);
  }

  .act:hover {
    color: var(--accent-hover);
    text-decoration: underline;
  }
</style>
