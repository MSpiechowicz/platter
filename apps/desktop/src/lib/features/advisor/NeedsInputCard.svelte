<script lang="ts">
  import { delayed } from '../../state/delayed.svelte';
  import type {
    AdvisorLabelsDto,
    AdvisorStatusDto,
    LabelDto,
    NodeId,
    ScanId,
    UserVerdict,
  } from '../../api/types';
  import Bar from '../../components/ui/Bar.svelte';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import Tag from '../../components/ui/Tag.svelte';
  import { formatCount } from '../../format';
  import type { AdvisorStore } from '../../state/advisor.svelte';
  import type { RulesStore } from '../../state/rules.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { visibleText } from '../overview/display';
  import { labelReadiness, needsInput, noScanText, progressText } from './model';

  interface Props {
    /** The finished scan, and whether a cleanup has outdated it. Undefined without a scan. */
    scan: { id: ScanId; outdated: boolean } | undefined;
    labels: AdvisorLabelsDto | undefined;
    loading: boolean;
    /** A scan is running, so there is no finished one yet. */
    scanning?: boolean;
    status: AdvisorStatusDto | undefined;
    store: AdvisorStore;
    rules: RulesStore;
    onretry: () => void;
  }

  let { scan, labels, loading, scanning = false, status, store, rules, onretry }: Props = $props();

  // A spinner that appeared stays at least a second, so the list does not replace it after a flash.
  const pendingShown = delayed(() => store.labelsPending);
  const readingShown = delayed(() => !labels && loading);

  const items = $derived(needsInput(labels));

  // What the user decided in this card, until the labels are read again and say so themselves (`decided`).
  let decisions = $state<Record<NodeId, UserVerdict>>({});
  let busy = $state<Record<NodeId, boolean>>({});
  let refusals = $state<Record<NodeId, string | undefined>>({});

  $effect(() => {
    void labels;
    decisions = {};
  });

  const decisionOf = (item: LabelDto): UserVerdict | undefined => item.decided ?? decisions[item.node];
  const waiting = $derived(items.filter((item) => decisionOf(item) === undefined));
  const running = $derived(
    scan !== undefined && status?.labelling?.scan === scan.id ? status.labelling : undefined,
  );
  const readiness = $derived(
    labelReadiness({ status, enabled: store.enabled, outdated: scan?.outdated ?? false, labels }),
  );
  const stopped = $derived(
    labels?.stopped === 'cancelled'
      ? 'Stopped before every folder was labelled.'
      : labels?.stopped === 'timeBudget'
        ? 'Stopped at the time limit, so some folders were not labelled.'
        : undefined,
  );

  function noun(count: number): string {
    return count === 1 ? '1 folder' : `${formatCount(count)} folders`;
  }

  function run(): void {
    if (scan && readiness.ready) {
      void store.label(scan.id);
    }
  }

  async function decide(item: LabelDto, verdict: UserVerdict): Promise<void> {
    if (!scan || busy[item.node]) {
      return;
    }

    busy[item.node] = true;
    refusals[item.node] = undefined;

    const refusal = await rules.add(scan.id, item.node, verdict);

    busy[item.node] = false;

    if (refusal === undefined) {
      decisions[item.node] = verdict;
    } else {
      refusals[item.node] = refusal;
    }
  }
</script>

<Card
  title="Needs your input"
  hint={waiting.length > 0 && !running ? noun(waiting.length) : undefined}
  class="needs"
>
  {#if !scan}
    <p class="empty">{noScanText(scanning)}</p>
  {:else if running}
    <div class="empty-state">
      <p class="msg" role="status">Labelling folders: {progressText(running.done, running.total)}</p>
      <Bar value={running.total > 0 ? running.done / running.total : 0} label="Labelling progress" />
      <div class="actions">
        <Button onclick={() => store.cancelLabelling()}>Cancel</Button>
      </div>
    </div>
  {:else if store.labelsPending || pendingShown.value}
    <div class="wait">
      <Loading when={store.labelsPending || pendingShown.value} label="Reading the labels…" />
    </div>
  {:else if items.length > 0}
    <div class="items">
      {#each items as item (item.node)}
        {@const decided = decisionOf(item)}
        {@const path = visibleText(item.path)}
        <div class="item">
          <div class="p">
            <span class="path" title={path}>{path}</span>
            <b class="num">{fmt.size(item.allocated)}</b>
          </div>
          <small>{visibleText(item.reason)}</small>

          {#if decided}
            <div class="acts decided">
              <Tag kind={decided}>{decided}</Tag>
              <span>{decided === 'keep' ? 'Your rule: always keep.' : 'Your rule: safe to clean.'}</span>
            </div>
          {:else}
            <div class="acts">
              <Button
                size="sm"
                disabled={busy[item.node]}
                aria-label="Always keep {path}"
                onclick={() => decide(item, 'keep')}>Always keep</Button
              >
              <Button
                size="sm"
                disabled={busy[item.node]}
                aria-label="Safe to clean {path}"
                onclick={() => decide(item, 'safe')}>Safe to clean</Button
              >
            </div>

            {#if refusals[item.node]}
              <p class="refusal" role="alert">{visibleText(refusals[item.node] as string)}</p>
            {/if}
          {/if}
        </div>
      {/each}
    </div>

    {#if stopped}
      <div class="foot">
        <span>{stopped}</span>
        <Button disabled={!readiness.ready} onclick={run}>Label again</Button>
      </div>
    {/if}
  {:else if (!labels && loading) || readingShown.value}
    <div class="wait">
      <Loading when={(!labels && loading) || readingShown.value} label="Reading the labels…" />
    </div>
  {:else if !labels}
    <div class="empty-state" role="alert">
      <p class="msg">The labels could not be read.</p>
      <Button onclick={onretry}>Try again</Button>
    </div>
  {:else}
    <div class="empty-state">
      {#if labels.labelled > 0}
        <p class="msg">
          The model looked at {noun(labels.labelled)} and found nothing that needs your input.
        </p>
      {:else if labels.candidates > 0}
        <p class="msg">
          {noun(labels.candidates)} no rule covers can be labelled by the local model. It reads names, sizes and
          dates only. Anything it is unsure about shows up here.
        </p>
      {:else}
        <p class="msg">Nothing waiting. Folders the model is unsure about show up here.</p>
      {/if}

      {#if stopped}
        <p class="muted small">{stopped}</p>
      {/if}

      <div class="actions">
        <Button disabled={!readiness.ready} onclick={run}>
          {labels.labelled > 0 ? 'Label again' : 'Label folders'}
        </Button>
      </div>

      {#if !readiness.ready && readiness.reason}
        <p class="muted small">{readiness.reason}</p>
      {/if}

      {#if store.labelFailure}
        <p class="muted small" role="alert">{visibleText(store.labelFailure)}</p>
      {/if}
    </div>
  {/if}
</Card>

<style>
  .empty,
  .msg {
    color: var(--muted);
    font-size: 12.5px;
    line-height: 1.5;
  }

  .small {
    font-size: 12px;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: flex-start;
    align-self: stretch;
  }

  .actions {
    display: flex;
    gap: 8px;
  }

  /* The list takes what the column leaves and scrolls inside it; the scroll bar sits in the card's padding. */
  .items {
    flex: 1;
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
    margin-right: -16px;
  }

  /* Exactly as wide as the card's content, whatever the engine does with the scroll bar. */
  .items > .item {
    width: calc(100% - 16px);
    width: 100cqw;
  }

  :global(.needs) {
    container-type: inline-size;
  }

  @media (max-width: 1300px) {
    .items {
      flex: none;
      max-height: 420px;
    }
  }

  .item {
    padding: 11px 0;
    border-top: 1px solid var(--line);
    font-size: 13px;
  }

  .item:first-child {
    border-top: 0;
    padding-top: 0;
  }

  .item:last-child {
    padding-bottom: 0;
  }

  .p {
    font-family: var(--mono);
    font-size: 12.5px;
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }

  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .p b {
    flex: none;
    font-family: var(--font);
    font-size: 13px;
  }

  small {
    display: block;
    color: var(--muted);
    font-size: 12px;
    margin-top: 3px;
    line-height: 1.45;
  }

  .acts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
  }

  .decided {
    color: var(--muted);
    font-size: 12px;
  }

  .refusal {
    margin-top: 6px;
    color: var(--muted);
    font-size: 12px;
    line-height: 1.45;
  }

  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-top: 10px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
    font-size: 12px;
    color: var(--muted);
  }

  /* The spinner fills its positioned parent: this box, below the title, as tall as the card's usual content, so
     it stays inside the card even while the card holds nothing else. */
  .wait {
    position: relative;
    min-height: 120px;
  }
</style>
