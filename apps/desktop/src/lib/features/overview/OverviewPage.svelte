<script lang="ts">
  import { delayed } from '../../state/delayed.svelte';
  import { api } from '../../api';
  import { baseName, formatCount, formatDuration } from '../../format';
  import PageHeader from '../../components/layout/PageHeader.svelte';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import Notice from '../../components/ui/Notice.svelte';
  import Pill from '../../components/ui/Pill.svelte';
  import { describeError } from '../../api/client';
  import { advisor } from '../../state/advisor.svelte';
  import { cleanup } from '../../state/cleanup.svelte';
  import { resource } from '../../state/resource.svelte';
  import { scan, type ReadyScan } from '../../state/scan.svelte';
  import { settings } from '../../state/settings.svelte';
  import { ui } from '../../state/ui.svelte';
  import AdvisorCard from './AdvisorCard.svelte';
  import AgeCard from './AgeCard.svelte';
  import CapacityCard from './CapacityCard.svelte';
  import DiskStackCard from './DiskStackCard.svelte';
  import FileTypesCard from './FileTypesCard.svelte';
  import LargestFoldersCard from './LargestFoldersCard.svelte';
  import { loadOverview } from './model';
  import ReclaimableCard from './ReclaimableCard.svelte';
  import ScannedAgo from './ScannedAgo.svelte';
  import { warningLines } from './scanWarnings';
  import UsageDaysCard from './UsageDaysCard.svelte';

  interface Props {
    ready: ReadyScan;
  }

  let { ready }: Props = $props();

  /** The volume when the scan root is that whole volume; a folder inside a volume does not count. */
  const wholeVolume = $derived(scan.volumeInfo?.exact ? scan.volumeInfo.volume : undefined);

  const overview = resource(() =>
    loadOverview(api, {
      scanId: ready.scanId,
      rootId: ready.rootId,
      total: wholeVolume?.total ?? ready.totals.allocated,
      free: wholeVolume?.available,
    }),
  );

  // Suggestions arrive on their own, so a slow or failed answer never holds back the rest of the page.
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

  const retrySuggestions = (): void => {
    attempt++;
  };

  // The history of the last half year feeds the day-by-day card. It is read on its own, so a slow or failed answer
  // never holds back the rest of the page, and read again once a cleanup has finished (its run is in the history).
  const HISTORY_DAYS = 182;

  const history = resource(() => {
    void cleanup.freedTotal;

    return api.historyOverview(HISTORY_DAYS);
  });

  const warnings = $derived(warningLines(ready.warnings));

  /** The whole status, for its tooltip when the pill has no room for all of it: when, to the minute. */
  const statusTitle = $derived(
    `Scanned ${new Date(ready.scannedAt * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })} · ` +
      `${formatCount(ready.totals.files)} files in ${formatDuration(ready.elapsedMs)}`,
  );

  const subtitle = $derived.by(() => {
    const options = scan.options;

    if (!options) {
      return '';
    }

    return wholeVolume
      ? `${baseName(wholeVolume.device)} · ${wholeVolume.fsType} · mounted at ${wholeVolume.mountPoint}`
      : options.root;
  });

  // A spinner that appeared stays at least a second, so the content does not replace it after a flash.
  const shown = delayed(() => !overview.data && !overview.error && overview.loading);
</script>

<div class="overview">
  <PageHeader title="STORAGE OVERVIEW" sub={subtitle}>
    <div class="status">
      <div class="topr">
        <Pill dot grow truncate title={statusTitle}>
          <ScannedAgo at={ready.scannedAt} /> · <b>{formatCount(ready.totals.files)} files</b> in {formatDuration(
            ready.elapsedMs,
          )}
        </Pill>
        <Button disabled={cleanup.busy} onclick={() => scan.rescan()}>Rescan</Button>
      </div>
      {#each warnings as line (line)}
        <p class="warning">{line}</p>
      {/each}
    </div>
  </PageHeader>

  {#if overview.data && !shown.value}
    {@const data = overview.data}
    <div class="grid">
      <div class="col l">
        <CapacityCard
          volume={wholeVolume}
          container={wholeVolume ? undefined : scan.volumeInfo?.volume}
          totals={ready.totals}
        />
        <FileTypesCard byCategory={data.summary.byCategory} />
        <ReclaimableCard
          suggestions={suggestions.data}
          loading={suggestions.loading}
          error={suggestions.error}
          onretry={retrySuggestions}
          scanned={data.summary.node.allocated}
          freed={cleanup.freedTotal}
          stale={ready.outdated}
        />
      </div>

      <div class="col c">
        <DiskStackCard
          slabs={data.slabs}
          total={wholeVolume?.total ?? ready.totals.allocated}
          onopen={(id, color) => ui.openInExplorer(id, color)}
        />
        <div class="row2">
          <AgeCard byAge={data.summary.byAge} total={data.summary.node.allocated} />
          <LargestFoldersCard
            folders={data.summary.largestDirs}
            paths={data.paths}
            colors={data.folderColors}
            onopen={(id, color) => ui.openInExplorer(id, color)}
          />
        </div>
      </div>

      <div class="col r">
        <AdvisorCard
          suggestions={suggestions.data}
          scanId={ready.scanId}
          store={cleanup}
          loading={suggestions.loading}
          error={suggestions.error}
          onretry={retrySuggestions}
          onopen={(id) => ui.openInExplorer(id)}
        />
      </div>
    </div>

    <div class="usage">
      <UsageDaysCard data={history.data} loading={history.loading} error={history.error} />
    </div>
  {:else if overview.error && !shown.value}
    <Notice kind="error">The scan could not be read: {describeError(overview.error)}</Notice>
  {:else}
    <Card class="waiting" aria-busy="true">
      <Loading when={overview.loading || shown.value} label="Reading the scan…" />
    </Card>
  {/if}
</div>

<style>
  /*
   * The grid of the approved mockup (docs/mockups/a-deck.html): the cards of the disk on the left, the picture of
   * the disk in the middle, the Advisor on the right. Its breakpoints are kept as they are there.
   */
  .overview {
    /* The header controls are as wide as the Advisor column below them. */
    --advisor-col: 380px;
  }

  .status {
    width: var(--advisor-col);
  }

  .topr {
    display: flex;
    gap: 10px;
  }

  /* A scan warning: one muted line under the status pill, aligned with its left edge. */
  .warning {
    margin: 6px 0 0;
    color: var(--muted);
    font-size: 12px;
    line-height: 1.4;
  }

  .grid {
    display: grid;
    grid-template-columns: 270px minmax(0, 1fr) var(--advisor-col);
    gap: 16px;
    align-items: stretch;
  }

  /* The day-by-day card spans the whole width under the grid, at every breakpoint. */
  .usage {
    margin-top: 16px;
  }

  .col {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
  }

  /* The last card on the left and the stack in the middle take the spare height, so all columns end together. */
  .col.l > :global(.card:last-child),
  .col.c > :global(.card:first-child) {
    flex: 1;
  }

  /* The Advisor never decides the height of the page: it fills whatever the other columns need. */
  .col.r {
    position: relative;
    min-height: 420px;
  }

  .col.r > :global(.card) {
    position: absolute;
    inset: 0;
  }

  .row2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }

  .row2 > :global(*) {
    min-width: 0;
  }

  .overview :global(.waiting) {
    min-height: 420px;
  }

  /* Narrower: the three cards of the disk sit in a row above the stack, and the Advisor spans both. */
  @media (max-width: 1560px) {
    .overview {
      --advisor-col: 360px;
    }

    .grid {
      grid-template-columns: minmax(0, 1fr) var(--advisor-col);
    }

    .col.l {
      flex-direction: row;
    }

    .col.l > :global(.card) {
      flex: 1;
      min-width: 0;
    }

    .col.c {
      grid-column: 1;
      grid-row: 2;
    }

    .col.r {
      grid-column: 2;
      grid-row: 1 / 3;
    }
  }

  @media (max-width: 1000px) {
    .overview {
      --advisor-col: 100%;
    }

    .grid {
      grid-template-columns: 1fr;
    }

    .col.c {
      grid-row: auto;
    }

    .col.r {
      grid-column: 1;
      grid-row: auto;
      min-height: 0;
    }

    .col.r > :global(.card) {
      position: relative;
    }
  }

  @media (max-width: 700px) {
    .row2 {
      grid-template-columns: 1fr;
    }

    .col.l {
      flex-direction: column;
    }
  }
</style>
