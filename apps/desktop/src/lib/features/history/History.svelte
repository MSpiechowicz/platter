<script lang="ts">
  import { untrack } from 'svelte';
  import type { ScanId } from '../../api/types';
  import PageHeader from '../../components/layout/PageHeader.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import Notice from '../../components/ui/Notice.svelte';
  import { cleanup as sharedCleanup, type CleanupStore } from '../../state/cleanup.svelte';
  import { delayed } from '../../state/delayed.svelte';
  import { history as shared, type HistoryStore } from '../../state/history.svelte';
  import { scan } from '../../state/scan.svelte';
  import { ui } from '../../state/ui.svelte';
  import { visibleText } from '../overview/display';
  import CleanupsTable from './CleanupsTable.svelte';
  import NoCleanups from './NoCleanups.svelte';
  import StatTiles from './StatTiles.svelte';
  import UsageChart from './UsageChart.svelte';

  interface Props {
    store?: HistoryStore;
    /** Holds the suggestions of the scan, which the card of an empty history talks about. */
    suggestions?: CleanupStore;
    /** The scan the suggestions belong to. Default: the scan that is ready. */
    scanId?: ScanId;
  }

  let {
    store = shared,
    suggestions = sharedCleanup,
    scanId = scan.state.status === 'ready' ? scan.state.scanId : undefined,
  }: Props = $props();

  // Read again each time the view opens. What was read before stays on screen meanwhile.
  $effect(() => {
    void store.load();
  });

  const data = $derived(store.data);

  // The Overview and the Cleanup view fetch the suggestions, so they are usually here already. Opened straight on
  // History, this fetches them once.
  $effect(() => {
    const id = scanId;

    if (id === undefined) {
      return;
    }

    untrack(() => {
      if (suggestions.scanId !== id) {
        suggestions.adopt(id);
      }

      void suggestions.ensureSuggestions(id);
    });
  });

  const safe = $derived(suggestions.safeItems.filter((item) => item.allocated > 0));
  const reviewCount = $derived(suggestions.cleanable.filter((item) => item.verdict === 'review').length);
  const known = $derived(suggestions.suggestionsKnown && suggestions.scanId === scanId);

  const subtitle = $derived.by(() => {
    if (!data) {
      return '';
    }

    const volume = data.volume ? ` · ${data.volume.label || data.volume.id}` : '';

    return `last ${data.days} days${volume}`;
  });

  // A spinner that appeared stays at least a second, so the content does not replace it after a flash.
  const shown = delayed(() => !store.data && !store.error && store.loading);
</script>

<div class="history">
  <PageHeader title="HISTORY" sub={subtitle} />

  {#if data && !shown.value}
    {#if data.problem}
      <Notice kind="error">{data.problem}</Notice>
    {:else}
      {#if store.error}
        <Notice kind="error"
          >The history could not be read again, so this may be out of date: {store.error}</Notice
        >
      {/if}
      <StatTiles {data} />
      <UsageChart
        samples={data.samples}
        without={data.without}
        cleanups={data.cleanups}
        today={data.today}
        thin={data.thin}
      />
      <CleanupsTable
        cleanups={data.cleanups}
        today={data.today}
        restoring={store.restoring}
        failures={store.failures}
        onrestore={(id) => void store.restore(id)}
      >
        {#snippet none()}
          <NoCleanups
            {known}
            safeBytes={safe.reduce((sum, item) => sum + item.allocated, 0)}
            kinds={safe.slice(0, 3).map((item) => visibleText(item.title))}
            {reviewCount}
            onreview={() => ui.go('cleanup')}
          />
        {/snippet}
      </CleanupsTable>
    {/if}
  {:else if store.error && !shown.value}
    <Notice kind="error">The history could not be read: {store.error}</Notice>
  {:else}
    <Card class="waiting" aria-busy="true">
      <Loading when={store.loading || shown.value} label="Reading the history…" />
    </Card>
  {/if}
</div>

<style>
  .history {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  /* The header keeps its own margin: the gap above is for the cards. */
  .history :global(.top) {
    margin-bottom: 2px;
  }

  .history :global(.waiting) {
    min-height: 420px;
  }
</style>
