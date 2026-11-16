<script lang="ts">
  import { formatCount } from '../../format';
  import PageHeader from '../../components/layout/PageHeader.svelte';
  import Bar from '../../components/ui/Bar.svelte';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import { scan, type RunningScan } from '../../state/scan.svelte';
  import ScanInfoPanel from './ScanInfoPanel.svelte';

  interface Props {
    running: RunningScan;
  }

  let { running }: Props = $props();
</script>

<PageHeader title="SCANNING" sub={running.options.root} />

<div class="wrap">
  <Card title="Scanning" class="viz" aria-busy="true">
    <div class="scanbox">
      <b class="num">{formatCount(running.progress.files)}</b>
      <div class="muted">files read</div>
      <div class="cap"><Bar size="thick" indeterminate label="Scan in progress" /></div>
      <div class="tick mono" title={running.progress.currentPath}>{running.progress.currentPath}</div>
      <Button disabled={running.cancelling} onclick={() => scan.cancel()}>
        {running.cancelling ? 'Cancelling…' : 'Cancel'}
      </Button>
    </div>
  </Card>

  <ScanInfoPanel root={running.options.root} />
</div>

<style>
  .wrap {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 420px;
    gap: 16px;
    align-items: stretch;
  }

  .wrap :global(.viz) {
    min-height: 620px;
  }

  /* The block is centred in the whole card, both ways. */
  .scanbox {
    position: absolute;
    inset: 0 0 6px;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 14px;
    text-align: center;
  }

  b {
    font-size: 38px;
    line-height: 1;
  }

  .cap {
    width: min(420px, 70vw);
  }

  .tick {
    font-size: 12px;
    color: var(--dim);
    height: 18px;
    line-height: 18px;
    max-width: min(520px, 80vw);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  @media (max-width: 1100px) {
    .wrap {
      grid-template-columns: 1fr;
    }

    .wrap :global(.viz) {
      min-height: 460px;
    }
  }
</style>
