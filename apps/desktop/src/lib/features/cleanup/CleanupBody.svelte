<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '../../api';
  import { describeError } from '../../api/client';
  import type { ScanId } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import { advisor } from '../../state/advisor.svelte';
  import { cleanup } from '../../state/cleanup.svelte';
  import { delayed } from '../../state/delayed.svelte';
  import { resource } from '../../state/resource.svelte';
  import { scan, type ReadyScan } from '../../state/scan.svelte';
  import { settings } from '../../state/settings.svelte';
  import { ui } from '../../state/ui.svelte';
  import AdvisorList from '../overview/AdvisorList.svelte';
  import ChartCard from './ChartCard.svelte';
  import { basisOf } from './model';
  import PlanPreviewCard from './PlanPreviewCard.svelte';
  import { requestOf, type FirstPreview } from './preview';
  import ResultPanel from './ResultPanel.svelte';

  interface Props {
    ready: ReadyScan;
  }

  let { ready }: Props = $props();

  // The suggestions are the same ones the Overview shows, and they go into the same store: a cleanup started in
  // either view is on screen in both.
  let attempt = $state(0);

  // The labels of the local model, or switching it on or off, change the suggestions, and so do the size units of the
  // settings (the reasons write sizes in them): they are fetched again, but not while a cleanup runs.
  const stamp = $derived(cleanup.stampFor(advisor.suggestionsVersion));
  const units = $derived(cleanup.unitsFor(settings.units));

  const suggestions = resource(() => {
    void attempt;
    void stamp;

    return api.getSuggestions(ready.scanId, units);
  });

  // The store learns of a new scan here too, not only from the list: after "Scan again" the list is not on screen
  // (the result is), and the result of the old scan must not stay in front of the new one.
  $effect.pre(() => {
    const id = ready.scanId;
    const data = suggestions.data;

    untrack(() => cleanup.load(id, data));
  });

  /** The volume when the scan root is that whole volume; a folder inside a volume does not count. */
  const wholeVolume = $derived(scan.volumeInfo?.exact ? scan.volumeInfo.volume : undefined);
  const basis = $derived(basisOf(wholeVolume, ready.totals.allocated));
  const finished = $derived(cleanup.phase.kind === 'done');

  // The page shows nothing until what it is made of is here: the suggestions and the first preview of the selection
  // they bring, so the chart, the Advisor and the preview appear together. A scan is waited for once: later reloads of
  // the suggestions, and a cleanup, never bring the spinner back.
  let settledFor = $state<ScanId | undefined>(undefined);
  let first = $state.raw<FirstPreview | undefined>(undefined);
  let startedFor: ScanId | undefined;

  // A failed list does not hold the page back: the Advisor offers its retry. Only an idle flow waits: coming back to
  // the page during or after a cleanup shows that cleanup (the chart, the result) at once, never the spinner.
  // The flow is read once, when the page opens: a review cancelled while the suggestions load must not bring a late
  // spinner over a page already on screen.
  const openedIdle = untrack(() => cleanup.phase.kind === 'idle');
  const waiting = $derived(settledFor !== ready.scanId && !suggestions.error && openedIdle);

  // A spinner that appeared stays at least a second, so the content does not replace it after a flash.
  const shown = delayed(() => waiting);

  $effect(() => {
    const id = ready.scanId;

    if (settledFor === id || startedFor === id) {
      return;
    }

    if (suggestions.error) {
      settledFor = id;

      return;
    }

    // The suggestions are in the store by now (see the effect above), and with them the selection.
    if (suggestions.data) {
      startedFor = id;
      untrack(() => void firstPreview(id));
    }
  });

  /** The plan of the selection, asked for at once. Nothing to ask for (no selection, no idle flow) is ready too. */
  async function firstPreview(id: ScanId): Promise<void> {
    const request = requestOf(cleanup.selectedItems, settings);

    if (request.ids.length > 0 && !cleanup.outdated && cleanup.phase.kind === 'idle') {
      let answer: FirstPreview;

      try {
        answer = { key: request.key, plan: await api.planCleanup(id, request.ids, request.options) };
      } catch (error) {
        answer = { key: request.key, failure: describeError(error) };
      }

      // An answer for a scan that is no longer on screen is dropped.
      if (id === ready.scanId) {
        first = answer;
      }
    }

    settledFor = id;
  }
</script>

{#if waiting || shown.value}
  <Card class="cleanup-waiting" aria-busy="true">
    <Loading when={waiting || shown.value} label="Reading the scan…" />
  </Card>
{:else}
  <ChartCard store={cleanup} {basis} />

  <!-- The panel never decides the height of the page: it fills whatever the chart needs. -->
  <div class="pcol">
    {#if finished}
      <ResultPanel store={cleanup} {basis} />
    {:else}
      <Card title="Advisor" hint={advisor.loaded ? 'LOCAL MODEL' : 'RULES'} class="advisor">
        <AdvisorList
          suggestions={suggestions.data}
          scanId={ready.scanId}
          store={cleanup}
          loading={suggestions.loading}
          error={suggestions.error}
          onretry={() => attempt++}
          onopen={(id) => ui.openInExplorer(id)}
        />
      </Card>
    {/if}
  </div>

  <!-- After a cleanup the result panel tells what happened, and a preview of the old selection would only confuse. A new
       scan starts the card afresh: a plan of the old one means nothing here. -->
  {#if !finished}
    {#key ready.scanId}
      <PlanPreviewCard store={cleanup} scanId={ready.scanId} {api} initial={first} />
    {/key}
  {/if}
{/if}

<style>
  /* One card in place of the whole page, as wide as the grid. */
  :global(.cleanup-waiting) {
    grid-column: 1 / -1;
    min-height: 420px;
  }

  .pcol {
    position: relative;
    min-height: 480px;
  }

  .pcol > :global(.card) {
    position: absolute;
    inset: 0;
  }

  /* In a single column the panel is a card among the others, and its list gets a height of its own. */
  @media (max-width: 1000px) {
    .pcol {
      min-height: 0;
    }

    .pcol > :global(.card) {
      position: relative;
    }
  }
</style>
