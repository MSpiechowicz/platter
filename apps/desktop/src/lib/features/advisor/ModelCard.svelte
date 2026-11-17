<script lang="ts">
  import Bar from '../../components/ui/Bar.svelte';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Notice from '../../components/ui/Notice.svelte';
  import type { AdvisorStatusDto } from '../../api/types';
  import { fmt } from '../../state/fmt.svelte';
  import type { AdvisorStore } from '../../state/advisor.svelte';
  import { visibleText } from '../overview/display';
  import {
    deviceText,
    downloadShare,
    onDiskText,
    ramShare,
    ramText,
    receivedText,
    SWITCHED_OFF_HINT,
    speedText,
    unloadAfterText,
    videoText,
  } from './model';

  interface Props {
    status: AdvisorStatusDto | undefined;
    store: AdvisorStore;
  }

  let { status, store }: Props = $props();

  const ram = $derived(status ? ramText(status, fmt.size) : undefined);
  /** On a graphics card the model is not in RAM: its video memory takes the RAM line's place. */
  const video = $derived(status ? videoText(status, fmt.size) : undefined);
  const device = $derived(status ? visibleText(deviceText(status)) : '—');
  const deviceShown = $derived(status ? visibleText(deviceText(status, true)) : '—');
  const problem = $derived(store.failure ?? status?.problem);
  /** A download in progress keeps its Cancel button: the slot has no room for a problem next to it. */
  const showProblem = $derived(problem !== undefined && status?.state !== 'downloading');
</script>

<Card title="Model">
  {#if status}
    <div class="kv">
      <div class="row"><span>Name</span><span>{visibleText(status.model.name)}</span></div>
      <div class="row"><span>Format</span><span>{visibleText(status.model.format)}</span></div>
      <div class="row"><span>On disk</span><span class="num">{onDiskText(status, fmt.size)}</span></div>
      <div class="row"><span>Speed</span><span class="num">{speedText(status.tokensPerSecond)}</span></div>
      <div class="row"><span>Runs on</span><span title={device}>{deviceShown}</span></div>
      <div class="row">
        <span>Unloads after</span><span>{unloadAfterText(status.options.unloadAfter)}</span>
      </div>
    </div>

    <!-- One slot for what depends on the state, so the card keeps its height in every state, a problem included. -->
    <div class="slot">
      {#if showProblem && problem}
        <Notice kind="error"
          ><span class="clip" title={visibleText(problem)}>{visibleText(problem)}</span></Notice
        >
      {:else if status.state === 'loaded' && video}
        <p class="muted small">{video}</p>
      {:else if status.state === 'loaded' && ram}
        <div
          class="ram"
          role="meter"
          aria-label="RAM in use"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(ramShare(status) * 100)}
        >
          <i style="width:{ramShare(status) * 100}%"></i>
        </div>
        <p class="muted small">{ram}</p>
      {:else if status.state === 'downloading' && status.download}
        <Bar value={downloadShare(status.download)} label="Download progress" />
        <div class="prow">
          <span class="muted small num">{receivedText(status.download, fmt.size)}</span>
          <Button onclick={() => store.cancelDownload()}>Cancel</Button>
        </div>
      {:else if status.state === 'downloading'}
        <p class="muted small">Starting the download…</p>
      {:else if status.state === 'notDownloaded' && status.download}
        <p class="muted small">
          {receivedText(status.download, fmt.size)} received. The download continues where it stopped.
        </p>
      {:else if status.state === 'notDownloaded'}
        <p class="muted small">The model is a one-time download of {fmt.size(status.model.bytes)}.</p>
      {:else if status.state === 'loading'}
        <!-- Where the RAM bar goes once it is loaded: a segment slides along it while the model loads. -->
        <div class="loadbar"><Bar indeterminate size="thick" label="Loading the model" /></div>
      {:else if status.state === 'onDisk'}
        <p class="muted small">
          {store.enabled ? 'Not in RAM. It loads when it is needed.' : SWITCHED_OFF_HINT}
        </p>
      {:else if status.state === 'unavailable'}
        <p class="muted small">The local model is not available in this build.</p>
      {/if}
    </div>

    <p class="note">The model download is Platter's only network call.</p>
  {:else}
    <p class="muted small">Checking the model…</p>
  {/if}
</Card>

<style>
  .kv {
    display: flex;
    flex-direction: column;
    gap: 7px;
    font-size: 13px;
  }

  /* The label keeps its line and the value gives way (cut at the end), so a long card name moves nothing. */
  .row {
    display: flex;
    justify-content: space-between;
    gap: 10px;
  }

  .row span:first-child {
    color: var(--muted);
    white-space: nowrap;
  }

  .row span:last-child {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .slot {
    min-height: 56px;
    padding-top: 14px;
  }

  .small {
    font-size: 12px;
  }

  .ram {
    height: 8px;
    border-radius: 2px;
    background: var(--track);
    overflow: hidden;
    margin-bottom: 6px;
  }

  .loadbar {
    margin-bottom: 6px;
  }

  .loadbar :global(.bar) {
    height: 8px;
  }

  .ram i {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .prow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-top: 8px;
    height: 30px;
  }

  .note {
    margin-top: 8px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
    font-size: 12px;
    color: var(--muted);
  }

  /* One line, cut at the end (the whole text is in the tooltip): a long problem must not make the card taller. */
  .slot :global(.notice > span) {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
