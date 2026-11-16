<script lang="ts">
  import PageHeader from '../../components/layout/PageHeader.svelte';
  import { cleanup } from '../../state/cleanup.svelte';
  import type { ReadyScan, RunningScan } from '../../state/scan.svelte';
  import ScanInfoPanel from '../scan/ScanInfoPanel.svelte';
  import CleanupBody from './CleanupBody.svelte';
  import { stepOf } from './model';
  import ScanStep from './ScanStep.svelte';
  import StepBar from './StepBar.svelte';

  interface Props {
    /** The scan the view is about: still running (the Scan step) or ready (Review, then Clean). */
    scan: RunningScan | ReadyScan;
  }

  let { scan }: Props = $props();

  // The step follows the stores: the scan, and the cleanup flow that the Overview shares.
  const step = $derived(stepOf(scan.status, cleanup.phase.kind));
  const complete = $derived(scan.status === 'ready' && cleanup.phase.kind === 'done');
</script>

<div class="cleanup">
  <PageHeader title="CLEANUP" sub="scan · review · clean">
    <div class="steps">
      <StepBar {step} {complete} />
    </div>
  </PageHeader>

  <div class="wrap">
    {#if scan.status === 'scanning'}
      <ScanStep running={scan} />
      <ScanInfoPanel root={scan.options.root} />
    {:else}
      <CleanupBody ready={scan} />
    {/if}
  </div>
</div>

<style>
  .cleanup {
    /* The step bar is as wide as the panel below it. */
    --panel-col: 420px;
  }

  .steps {
    width: var(--panel-col);
  }

  .wrap {
    display: grid;
    grid-template-columns: minmax(0, 1fr) var(--panel-col);
    gap: 16px;
    align-items: stretch;
  }

  .wrap > :global(.viz) {
    min-height: 620px;
  }

  @media (max-width: 1000px) {
    .cleanup {
      --panel-col: 100%;
    }

    .wrap {
      grid-template-columns: 1fr;
    }

    .wrap > :global(.viz) {
      min-height: 460px;
    }
  }
</style>
