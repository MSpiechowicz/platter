<script lang="ts">
  import type { SummaryDto } from '../../api/types';
  import Bar from '../../components/ui/Bar.svelte';
  import Card from '../../components/ui/Card.svelte';
  import { tooltip } from '../../components/ui/tooltip.svelte';
  import { percentText } from '../../format';
  import { fmt } from '../../state/fmt.svelte';
  import { typeRows, type TypeRow } from './fileTypes';

  interface Props {
    byCategory: SummaryDto['byCategory'];
  }

  let { byCategory }: Props = $props();

  const rows = $derived(typeRows(byCategory));
  const largest = $derived(rows[0]?.allocated ?? 1);

  const names = (row: TypeRow): string[] =>
    (row.folded ?? []).map((entry) => `${entry.label}: ${fmt.size(entry.allocated)}`);
</script>

<Card title="By file type">
  <div class="types">
    {#each rows as row (row.key)}
      {#if row.folded}
        <div
          class="type"
          role="group"
          aria-label="{row.label}: {names(row).join(', ')}"
          use:tooltip={{ title: row.label, items: names(row) }}
        >
          {@render cells(row)}
        </div>
      {:else}
        <div class="type" title="{percentText(row.share)} of the scanned space">
          {@render cells(row)}
        </div>
      {/if}
    {:else}
      <p class="muted">Nothing was found.</p>
    {/each}
  </div>
</Card>

{#snippet cells(row: TypeRow)}
  <span class="name">{row.label}</span>
  <Bar value={row.allocated / largest} color={row.color} label="{row.label} share" />
  <span class="value num">{fmt.size(row.allocated)}</span>
{/snippet}

<style>
  /* Compact rows (name, bar, size) like the mockup; the card is as tall as its rows need. */
  .types {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .type {
    display: grid;
    grid-template-columns: 104px minmax(0, 1fr) 56px;
    gap: 8px;
    align-items: center;
    padding: 4px 0;
    font-size: 12.5px;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .value {
    text-align: right;
    white-space: nowrap;
    color: var(--muted);
  }

  /* Side by side with the other top cards the card may be taller than its rows: they spread out like the mockup. */
  @media (max-width: 1560px) and (min-width: 1001px) {
    .types {
      flex: 1;
      justify-content: space-between;
    }
  }

  @media (max-width: 700px) {
    .types {
      flex: none;
      justify-content: flex-start;
    }
  }
</style>
