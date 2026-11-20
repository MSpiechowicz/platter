<script lang="ts">
  import { untrack } from 'svelte';
  import PageHeader from '../../components/layout/PageHeader.svelte';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import Pill from '../../components/ui/Pill.svelte';
  import { advisor } from '../../state/advisor.svelte';
  import { delayed } from '../../state/delayed.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { rules } from '../../state/rules.svelte';
  import type { ReadyScan } from '../../state/scan.svelte';
  import AdvisorWelcome from './AdvisorWelcome.svelte';
  import AskCard from './AskCard.svelte';
  import ModelCard from './ModelCard.svelte';
  import { pillText, primaryAction } from './model';
  import NeedsInputCard from './NeedsInputCard.svelte';
  import RulesCard from './RulesCard.svelte';
  import VerdictCard from './VerdictCard.svelte';

  interface Props {
    /** The finished scan; the page is there without one, with its scan cards saying so. */
    ready: ReadyScan | undefined;
    /** A scan is running: the cards that need one say it is not ready yet, not that there is none. */
    scanning?: boolean;
  }

  let { ready, scanning = false }: Props = $props();

  /** How long the first read waits for the model's status before the cards are shown without it. */
  const STATUS_WAIT_MS = 5000;

  const status = $derived(advisor.status);
  const action = $derived(primaryAction(status, advisor.enabled, fmt.size));
  const labelling = $derived(status?.labelling !== undefined);
  // A model that is not on disk gets one welcome with the one download action, in place of cards with no data.
  const welcome = $derived(status?.state === 'notDownloaded' || status?.state === 'downloading');

  // The labels of the scan on screen: read again when the scan changes or a cleanup outdates it.
  let settled = $state(false);
  let attempt = $state(0);

  $effect(() => {
    const id = ready?.scanId;

    void ready?.outdated;
    void attempt;

    if (id === undefined) {
      return;
    }

    untrack(() => {
      settled = false;
    });

    let current = true;

    advisor.loadLabels(id).then(() => {
      if (current) {
        settled = true;
      }
    });

    return () => {
      current = false;
    };
  });

  // The rules file is read again whenever the page opens: the backend notices an edit by hand at once.
  let rulesRead = $state(false);

  $effect(() => {
    void rules.load().then(() => {
      rulesRead = true;
    });
  });

  const labels = $derived(ready ? advisor.labelsOf(ready.scanId) : undefined);
  // A request of this page that a newer one replaced has not settled anything: the store says when the latest ends.
  const loadingLabels = $derived(ready !== undefined && (!settled || advisor.labelsLoading));

  /**
   * The page opens with one spinner in the middle, like the Overview, until the model's status, the rules and the
   * labels of the scan are all in. The cards are then shown together. Once shown they stay, and a card that reads
   * something again later shows its own spinner. A spinner that appeared stays at least a second (`delayed`), so the
   * cards wait for it too.
   */
  let opened = $state(false);

  // A status that never comes (the channel and the one-off read both failed) does not keep the page shut: the cards
  // can show a missing status, as they did before this spinner.
  let statusGivenUp = $state(false);

  $effect(() => {
    const timer = setTimeout(() => (statusGivenUp = true), STATUS_WAIT_MS);

    return () => clearTimeout(timer);
  });

  const firstReadDone = $derived(
    (status !== undefined || statusGivenUp) && rulesRead && (ready === undefined || settled),
  );
  const firstWait = delayed(() => !opened && !firstReadDone);

  $effect(() => {
    if (firstReadDone && !firstWait.value) {
      opened = true;
    }
  });

  function press(): void {
    if (action?.kind === 'download') {
      void advisor.download();
    } else if (action?.kind === 'cancel') {
      void advisor.cancelDownload();
    } else if (action?.kind === 'load') {
      void advisor.load();
    } else if (action?.kind === 'unload') {
      void advisor.unload();
    }
  }

  const disabled = $derived.by(() => {
    if (!action) {
      return true;
    }

    switch (action.kind) {
      case 'download':
        return advisor.asking;
      case 'load':
        return action.disabled || advisor.switching;
      case 'unload':
        return advisor.switching || labelling;
      case 'busy':
        return true;
      default:
        return false;
    }
  });
</script>

<div class="advisor-page">
  <PageHeader title="ADVISOR" sub="local model · runs offline in RAM">
    <!-- Exactly as wide as the right column below, so the header ends where the cards do. -->
    <div class="acts" class:alone={welcome}>
      <Pill
        dot
        grow={!welcome}
        idle={status?.state !== 'loaded'}
        busy={status?.state === 'loading' || status?.state === 'downloading'}>{pillText(status?.state)}</Pill
      >
      {#if action && !welcome}
        <Button
          variant={action.kind === 'download' ? 'primary' : 'default'}
          {disabled}
          title={action.kind === 'load' ? action.hint : undefined}
          onclick={press}
        >
          {action.label}
        </Button>
      {/if}
    </div>
  </PageHeader>

  {#if !opened}
    <Card class="waiting" aria-busy="true">
      <Loading when={!firstReadDone || firstWait.value} label="Reading the advisor…" />
    </Card>
  {:else if welcome && status}
    <AdvisorWelcome {status} store={advisor} />
  {:else}
    <div class="grid">
      <div class="col l">
        <ModelCard {status} store={advisor} />
        <VerdictCard
          hasScan={ready !== undefined}
          {scanning}
          {labels}
          loading={loadingLabels}
          onretry={() => attempt++}
        />
      </div>

      <div class="col c">
        <AskCard scan={ready?.scanId} {scanning} store={advisor} />
      </div>

      <div class="col r">
        <NeedsInputCard
          scan={ready ? { id: ready.scanId, outdated: ready.outdated } : undefined}
          {labels}
          loading={loadingLabels}
          {scanning}
          {status}
          store={advisor}
          {rules}
          onretry={() => attempt++}
        />
        <RulesCard scan={ready?.scanId} {scanning} store={advisor} {rules} />
      </div>
    </div>
  {/if}
</div>

<style>
  /* The welcome fills what is left of the content area (the page has the padding of the main area around it, and the banner of an outdated scan, when it shows). */
  .advisor-page {
    display: flex;
    flex-direction: column;
    min-height: calc(100vh - 40px - var(--banner-space, 0px));
  }

  /* The first read of the page: one card with the spinner in its middle, as the Overview has. */
  .advisor-page :global(.waiting) {
    min-height: 420px;
  }

  @media (max-width: 900px) {
    .advisor-page {
      min-height: calc(100vh - 32px - var(--banner-space, 0px));
    }
  }

  /* Exactly as wide as the right column below, so the header ends where the cards do. */
  .acts {
    display: flex;
    gap: 10px;
    width: 360px;
  }

  /* The welcome has only the pill: no column to end at. */
  .acts.alone {
    width: auto;
  }

  @media (max-width: 1300px) {
    .acts {
      width: auto;
    }
  }

  /* The grid of the approved mockup (docs/mockups/advisor.html), breakpoints included. */
  .grid {
    display: grid;
    grid-template-columns: 320px minmax(0, 1fr) 360px;
    gap: 16px;
    align-items: stretch;
  }

  .col {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
  }

  .col > :global(.card:last-child) {
    flex: 1;
  }

  /* The labels never decide the height of the page: the column takes the height the other two need. */
  .col.r {
    contain: size;
  }

  /* The conversation does not either: the card keeps the height of the columns beside it and its log scrolls. */
  .col.c {
    contain: size;
    min-height: 440px;
  }

  .col.c > :global(.card) {
    min-height: 0;
  }

  .col.r > :global(.card) {
    min-height: 0;
  }

  /* The first card takes what it needs, up to a share of the column; the rules take the rest and scroll inside. */
  .col.r > :global(.needs) {
    flex: 0 1 auto;
    max-height: 58%;
  }

  .col.r > :global(.rules) {
    flex: 1 1 0;
    min-height: 200px;
  }

  @media (max-width: 1300px) {
    .grid {
      grid-template-columns: 300px minmax(0, 1fr);
    }

    .col.r {
      grid-column: 1 / -1;
      flex-direction: row;
      contain: none;
    }

    .col.r > :global(.card) {
      flex: 1;
      max-height: none;
    }
  }

  @media (max-width: 800px) {
    .grid {
      grid-template-columns: 1fr;
    }

    /* One column has no neighbour to take the height from: the log has its own limit (see the Ask card). */
    .col.c {
      contain: none;
      min-height: 0;
    }

    .col.r {
      flex-direction: column;
    }
  }
</style>
