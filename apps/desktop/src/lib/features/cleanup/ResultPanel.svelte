<script lang="ts">
  import Bar from '../../components/ui/Bar.svelte';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import type { CleanupStore } from '../../state/cleanup.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { footerStatus } from '../overview/cleanupText';
  import { resultBars, type Basis } from './model';

  interface Props {
    store: CleanupStore;
    basis: Basis;
  }

  let { store, basis }: Props = $props();

  // Only shown once a cleanup has ended; `result` is that cleanup's own report.
  const result = $derived(store.phase.kind === 'done' ? store.phase.result : undefined);
  const bars = $derived(resultBars(basis, result?.freed));
  const status = $derived(footerStatus(store));
  const canUndo = $derived(store.actions.includes('undo'));

  const percent = (share: number): string => `${Math.round(share * 100)}%`;
  // A bar's share of the whole volume, or of what the scan counted for a folder.
  const unit = $derived(basis.capacity === undefined ? 'scanned' : 'used');
</script>

<Card title="Result">
  {#if result}
    <div class="big num">
      {#if result.kind === 'failed'}
        <span class="note">Cleaning failed</span>
      {:else if result.freed === undefined}
        <span class="note">Freed: not measured</span>
      {:else}
        {fmt.size(result.freed)}
        <span class="unit">freed</span>
      {/if}
    </div>

    <div class="ba">
      <div>
        <div class="row">
          <span>Before</span>
          <span class="num">{fmt.size(bars.before.used)} {unit} · {percent(bars.before.share)}</span>
        </div>
        <Bar size="thick" value={bars.before.share} color="var(--slate)" label="Before the cleanup" />
      </div>

      <div>
        <div class="row">
          <span>After</span>
          {#if bars.after}
            <span class="num">{fmt.size(bars.after.used)} {unit} · {percent(bars.after.share)}</span>
          {:else}
            <span>Not measured</span>
          {/if}
        </div>
        <Bar
          size="thick"
          value={bars.after?.share ?? 0}
          color="var(--accent)"
          label={bars.after ? 'After the cleanup' : 'After the cleanup: not measured'}
        />
      </div>
    </div>

    <p class="text" class:attention={status.attention} role="status">{status.text}</p>

    {#each status.details as line, index (index)}
      <p class="detail">{line}</p>
    {/each}

    <!-- Undo only when the backend can put something back; "Scan again" keeps its place either way. -->
    <div class="btns">
      {#if canUndo}
        <Button onclick={() => void store.undo()}>Undo</Button>
      {/if}

      <Button variant="primary" onclick={() => void store.rescan()}>Scan again</Button>
    </div>
  {/if}
</Card>

<style>
  .big {
    font-size: 38px;
    font-weight: 700;
    line-height: 1;
  }

  .unit {
    font-size: 14px;
    font-weight: 400;
    color: var(--muted);
  }

  .note {
    font-size: 22px;
    color: var(--muted);
  }

  .ba {
    display: grid;
    gap: 14px;
    margin-top: 20px;
    font-size: 12.5px;
    color: var(--muted);
  }

  .row {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 6px;
  }

  .text {
    margin-top: 18px;
    font-size: 13px;
    line-height: 1.45;
    color: var(--muted);
  }

  .text.attention {
    color: var(--amber);
  }

  .detail {
    margin-top: 3px;
    font-size: 11.5px;
    color: var(--dim);
    overflow-wrap: anywhere;
  }

  .btns {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    margin-top: auto;
    padding-top: 14px;
  }

  /* "Scan again" stays in the right column, so it does not move when Undo goes away. */
  .btns > :global(:last-child) {
    grid-column: 2;
  }

  .btns :global(.btn) {
    padding: 9px 8px;
  }
</style>
