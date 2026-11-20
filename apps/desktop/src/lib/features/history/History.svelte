<script lang="ts">
  import { untrack } from 'svelte';
  import type { ScanId } from '../../api/types';
  import { fillHeight } from '../../components/layout/fillHeight.svelte';
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
  import CleanedMostCard from './CleanedMostCard.svelte';
  import CleanupsTable from './CleanupsTable.svelte';
  import NoCleanups from './NoCleanups.svelte';
  import ReadyNowCard from './ReadyNowCard.svelte';
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

  /**
   * A cleanup ran on the scan: the suggestions describe the disk from before it. The scan store says it from the
   * first frame. The suggestions say it once they have adopted the scan.
   */
  const stale = $derived(
    suggestions.outdated ||
      (scan.state.status === 'ready' && scan.state.scanId === scanId && scan.state.outdated),
  );

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

      // It never fails: when the suggestions cannot be read, they stay unknown, and the page stops waiting for them.
      void suggestions.ensureSuggestions(id).then(() => {
        asked = id;
      });
    });
  });

  /** The scan whose suggestions were asked for and answered, or not read. */
  let asked = $state<ScanId>();

  const safe = $derived(suggestions.safeItems.filter((item) => item.allocated > 0));
  const reviewCount = $derived(suggestions.cleanable.filter((item) => item.verdict === 'review').length);
  const reviewBytes = $derived(
    suggestions.cleanable
      .filter((item) => item.verdict === 'review')
      .reduce((sum, item) => sum + item.allocated, 0),
  );
  const safeBytes = $derived(safe.reduce((sum, item) => sum + item.allocated, 0));
  const known = $derived(suggestions.suggestionsKnown && suggestions.scanId === scanId);

  const subtitle = $derived.by(() => {
    if (!data) {
      return '';
    }

    const volume = data.volume ? ` · ${data.volume.label || data.volume.id}` : '';

    return `last ${data.days} days${volume}`;
  });

  // The page appears all at once: it waits for the history, and for the suggestions of the scan that is ready (unless
  // they cannot be read). Only the first wait of a visit counts: a later read keeps what is on screen, and the spinner
  // does not come back.
  const pending = $derived(
    (!data && !store.error) || (!store.error && scanId !== undefined && !known && asked !== scanId),
  );
  let released = $state(false);

  $effect(() => {
    if (!pending) {
      released = true;
    }
  });

  const waiting = $derived(pending && !released);

  // A spinner that appeared stays at least a second, so the content does not replace it after a flash.
  const shown = delayed(() => waiting);

  // The row under the chart fills the page down to its bottom padding: the table and the list scroll inside their cards.
  /** The row is never shorter than this; a window too short for it scrolls the page instead. */
  const MIN_HEIGHT = 240;
  let row = $state<HTMLElement>();
  const fill = fillHeight(() => row, MIN_HEIGHT);
</script>

<div class="history">
  <PageHeader title="HISTORY" sub={subtitle} />

  {#if data && !waiting && !shown.value}
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
      <!-- With no cleanups there is nothing to scroll: the row is as tall as its cards, not stretched to the bottom. -->
      <div
        class="split"
        class:empty={data.cleanups.length === 0}
        bind:this={row}
        style={fill.cap === undefined || data.cleanups.length === 0 ? undefined : `height: ${fill.cap}px`}
      >
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
              {safeBytes}
              kinds={safe.slice(0, 3).map((item) => visibleText(item.title))}
              {reviewCount}
              {stale}
              onreview={() => ui.go('cleanup')}
            />
          {/snippet}
        </CleanupsTable>

        <div class="side">
          <CleanedMostCard cleanups={data.cleanups} today={data.today} days={data.days} />
          <ReadyNowCard
            {known}
            scanned={scanId !== undefined}
            {safeBytes}
            {reviewBytes}
            {stale}
            ongo={() => ui.go('cleanup')}
          />
        </div>
      </div>
    {/if}
  {:else if store.error && !waiting && !shown.value}
    <Notice kind="error">The history could not be read: {store.error}</Notice>
  {:else}
    <Card class="waiting" aria-busy="true">
      <Loading when={store.loading || waiting || shown.value} label="Reading the history…" />
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

  .split {
    display: grid;
    grid-template-columns: minmax(0, 1.6fr) minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr);
    gap: 16px;
    min-height: 240px;
  }

  .side {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-height: 0;
  }

  /* The list gives way and scrolls inside, so "Ready to clean now" under it stays within the row. */
  .side > :global(.most) {
    min-height: 0;
  }

  .split.empty {
    min-height: 0;
  }

  /* In a single column the cards are as tall as what they hold. */
  @media (max-width: 1000px) {
    .split {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: none;
      min-height: 0;
    }
  }

  .history :global(.waiting) {
    min-height: 420px;
  }
</style>
