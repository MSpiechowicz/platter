<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HistoryCleanupDto } from '../../api/types';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import type { RestoreFailure } from '../../state/history.svelte';
  import { canRestore, formatDay, methodText, statusText, whatText } from './format';

  interface Props {
    cleanups: readonly HistoryCleanupDto[];
    today: string;
    /** The cleanup being restored; no other can be started meanwhile. */
    restoring: number | undefined;
    failures: Readonly<Record<number, RestoreFailure>>;
    onrestore: (cleanup: number) => void;
    /** What the card shows while the window holds no cleanup. */
    none: Snippet;
  }

  let { cleanups, today, restoring, failures, onrestore, none }: Props = $props();

  /** A failed Restore lists this many items, and counts the rest. */
  const SHOWN_FAILURES = 3;
</script>

<Card title="Cleanups">
  {#if cleanups.length === 0}
    {@render none()}
  {:else}
    <table>
      <thead>
        <tr>
          <th>Date</th>
          <th>What was cleaned</th>
          <th>Freed</th>
          <th>Method</th>
          <th class="status">Status</th>
        </tr>
      </thead>
      <tbody>
        {#each cleanups as cleanup (cleanup.id)}
          {@const failure = failures[cleanup.id]}
          <tr class:gone={cleanup.status === 'restored'}>
            <td class="num">{formatDay(cleanup.day, today)}</td>
            <td>{whatText(cleanup.what)}</td>
            <td><b class="num">{fmt.size(cleanup.freed)}</b></td>
            <td><span class="mono method">{methodText(cleanup.methods)}</span></td>
            <td class="status">
              <div class="state">
                <span class="muted">{statusText(cleanup.status)}</span>
                {#if canRestore(cleanup)}
                  <Button size="sm" disabled={restoring !== undefined} onclick={() => onrestore(cleanup.id)}>
                    {restoring === cleanup.id ? 'Restoring…' : 'Restore'}
                  </Button>
                {/if}
              </div>
              {#if failure}
                <div class="failure" role="alert">
                  <p>{failure.message}</p>
                  {#each failure.items.slice(0, SHOWN_FAILURES) as item (item.path)}
                    <p class="item">
                      <span class="mono">{item.path}</span>
                      {item.error ?? 'could not be put back'}
                    </p>
                  {/each}
                  {#if failure.items.length > SHOWN_FAILURES}
                    <p>and {failure.items.length - SHOWN_FAILURES} more.</p>
                  {/if}
                </div>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</Card>

<style>
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  th {
    text-align: left;
    font-size: 11px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--muted);
    font-weight: 600;
    padding: 0 10px 9px 0;
  }

  td {
    padding: 11px 10px 11px 0;
    border-top: 1px solid var(--line);
    vertical-align: middle;
  }

  th:last-child,
  td:last-child {
    padding-right: 0;
  }

  .status {
    text-align: right;
  }

  .state {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
  }

  .method {
    font-size: 12px;
    color: var(--muted);
  }

  tr.gone td {
    opacity: 0.45;
  }

  .failure {
    margin-top: 6px;
    font-size: 12px;
    line-height: 1.4;
    color: var(--red);
  }

  .failure p {
    margin: 0;
  }

  .item {
    color: var(--muted);
  }
</style>
