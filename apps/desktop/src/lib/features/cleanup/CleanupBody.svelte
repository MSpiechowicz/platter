<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { api } from '../../api';
  import { hooks } from '../../hooks';
  import Card from '../../components/ui/Card.svelte';
  import { cleanup } from '../../state/cleanup.svelte';
  import { resource } from '../../state/resource.svelte';
  import { scan, type ReadyScan } from '../../state/scan.svelte';
  import { ui } from '../../state/ui.svelte';
  import AdvisorList from '../overview/AdvisorList.svelte';
  import ChartCard from './ChartCard.svelte';
  import { basisOf } from './model';
  import ResultPanel from './ResultPanel.svelte';

  interface Props {
    ready: ReadyScan;
  }

  let { ready }: Props = $props();

  // The suggestions are the same ones the Overview shows, and they go into the same store: a cleanup started in
  // either view is on screen in both.
  let attempt = $state(0);

  const suggestions = resource(() => {
    void attempt;

    return api.getSuggestions(ready.scanId);
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

  // Screenshots only (`?clean=`, see hooks.ts): the mock plays the click that a person would make.
  let hooked = false;

  $effect(() => {
    if (hooked || !hooks.clean || !suggestions.data) {
      return;
    }

    hooked = true;

    void tick().then(() =>
      untrack(() => {
        if (hooks.clean === 'review') {
          cleanup.select('all');
          void cleanup.cleanSelected();
        } else {
          void cleanup.cleanSafe();
        }
      }),
    );
  });
</script>

<ChartCard store={cleanup} {basis} />

<!-- The panel never decides the height of the page: it fills whatever the chart needs. -->
<div class="pcol">
  {#if finished}
    <ResultPanel store={cleanup} {basis} />
  {:else}
    <Card title="Advisor" hint="RULES" class="advisor">
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

<style>
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
