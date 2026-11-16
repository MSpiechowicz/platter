<script lang="ts">
  import type { AdvisorLabelsDto, StepDto } from '../../api/types';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import { formatCount, formatDuration } from '../../format';
  import { noScanText, percentText, splitParts, type SplitPart } from './model';
  import WhatModelSees from './WhatModelSees.svelte';

  interface Props {
    /** False without a finished scan: the counts belong to a scan. */
    hasScan: boolean;
    /** A scan is running, so the counts are not there yet. */
    scanning?: boolean;
    /** The counts of the scan; undefined while they load or when they could not be loaded. */
    labels: AdvisorLabelsDto | undefined;
    loading: boolean;
    onretry: () => void;
  }

  let { hasScan, scanning = false, labels, loading, onretry }: Props = $props();

  interface Step {
    title: string;
    text: string;
    unit: string;
    data: StepDto;
  }

  const steps = $derived<Step[]>(
    labels
      ? [
          {
            title: 'Built-in rules',
            text: 'Known places: caches, build folders, trash.',
            unit: 'folders',
            data: labels.rules,
          },
          {
            title: 'Local model',
            text: 'The rest, from name, size and dates only.',
            unit: 'folders',
            data: labels.model,
          },
          {
            title: 'Safety guard',
            text: 'System and protected paths always win.',
            unit: 'left for you',
            data: labels.guard,
          },
        ]
      : [],
  );

  const parts = $derived(labels ? splitParts(labels) : []);

  const COLORS: Record<SplitPart['key'], string> = {
    rules: 'var(--accent)',
    model: 'var(--violet)',
    guard: 'var(--amber)',
  };

  const NAMES: Record<SplitPart['key'], string> = { rules: 'rules', model: 'model', guard: 'left for you' };

  /** Shown only once the model has labelled something in this scan: a dash said nothing. */
  const modelTime = $derived(labels && labels.labelled > 0 ? formatDuration(labels.modelMs) : undefined);
</script>

<Card title="How a verdict is made" hint="LAST SCAN">
  {#if !hasScan}
    <p class="empty">{noScanText(scanning)}</p>
  {:else if labels}
    <ol class="pipe">
      {#each steps as step, index (step.title)}
        <li class="st">
          <i aria-hidden="true">{index + 1}</i>
          <div class="body">
            <b>{step.title}</b>
            <small>{step.text}</small>
          </div>
          <div class="cn">
            <b class="num">{formatCount(step.data.folders)}</b>
            <small>{step.unit}</small>
          </div>
        </li>
      {/each}
    </ol>

    <div class="splitwrap">
      <p class="cap">Who decided, by folders</p>
      <div class="split" role="img" aria-label="Folders by who decided">
        {#each parts as part (part.key)}
          <i style="width:{part.share * 100}%;background:{COLORS[part.key]}"></i>
        {/each}
      </div>
      <div class="splitl">
        {#each parts as part (part.key)}
          <span><i style="background:{COLORS[part.key]}"></i>{NAMES[part.key]} {percentText(part.share)}</span
          >
        {/each}
        {#if modelTime}
          <span class="sp"></span>
          <span class="num">model time {modelTime}</span>
        {/if}
      </div>
    </div>
  {:else if loading}
    <Loading when={loading} label="Reading the scan…" />
  {:else}
    <div class="retry" role="alert">
      <p class="empty">The counts could not be read.</p>
      <Button onclick={onretry}>Try again</Button>
    </div>
  {/if}

  <WhatModelSees />
</Card>

<style>
  .empty {
    font-size: 12.5px;
    color: var(--muted);
  }

  .retry {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
  }

  .pipe {
    list-style: none;
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  .st {
    display: grid;
    grid-template-columns: 26px minmax(0, 1fr) auto;
    gap: 12px;
    align-items: start;
    position: relative;
    padding-bottom: 18px;
  }

  .st:last-child {
    padding-bottom: 0;
  }

  .st:not(:last-child)::before {
    content: '';
    position: absolute;
    left: 12.5px;
    top: 28px;
    bottom: 2px;
    width: 1px;
    background: var(--line);
  }

  .st > i {
    width: 26px;
    height: 26px;
    border-radius: 50%;
    border: 1px solid var(--accent);
    color: var(--accent);
    display: grid;
    place-items: center;
    font-style: normal;
    font-size: 12px;
    font-weight: 600;
  }

  .st b {
    display: block;
    font-size: 13.5px;
    line-height: 26px;
  }

  .st small {
    display: block;
    color: var(--muted);
    font-size: 12px;
    line-height: 1.4;
  }

  .cn {
    text-align: right;
  }

  .cn b {
    font-size: 17px;
  }

  .cap {
    font-size: 12px;
    color: var(--muted);
    margin-bottom: 8px;
  }

  .splitwrap {
    margin-top: 20px;
  }

  .split {
    display: flex;
    height: 8px;
    border-radius: 2px;
    overflow: hidden;
    gap: 2px;
    background: var(--track);
  }

  .split i {
    display: block;
    height: 100%;
    min-width: 3px;
  }

  .splitl {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    white-space: nowrap;
    font-size: 12px;
    color: var(--muted);
    margin-top: 8px;
  }

  .splitl i {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    margin-right: 6px;
  }

  .sp {
    flex: 1;
  }
</style>
