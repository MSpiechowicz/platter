<script lang="ts">
  import Card from '../../components/ui/Card.svelte';
  import { countUp } from '../../motion/countUp.svelte';
  import type { CleanupStore } from '../../state/cleanup.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { STALE_TEXT } from '../overview/cleanupText';
  import FlowChart from './FlowChart.svelte';
  import { afterCleanup, chartItems, runBytes, type Basis } from './model';

  interface Props {
    store: CleanupStore;
    basis: Basis;
  }

  let { store, basis }: Props = $props();

  const phase = $derived(store.phase);
  const locked = $derived(store.busy || store.outdated);
  /** After a cleanup the sizes are the scan's from before it: they stay on screen, greyed, until a rescan. */
  const stale = $derived(store.outdated);
  const marked = $derived(store.markedGroup);

  const items = $derived(
    chartItems({
      suggestions: store.suggestions,
      selected: store.selected,
      cleaned: store.cleaned,
      groups: store.groups,
      marked,
      ran: phase.kind === 'running' || phase.kind === 'done',
      locked,
    }),
  );

  /** The disk is named only when the whole disk was scanned; for a folder nothing is said about the disk around it. */
  const subject = $derived(basis.capacity === undefined ? 'Folder' : 'Disk');

  // What the hero, the bars and the numbers say depends on how far the flow is. Nothing is shown as freed that the
  // backend did not report: until it does, the sizes are the scan's own, and the words say so.
  const planned = $derived.by(() => {
    if (phase.kind === 'running' || phase.kind === 'done') {
      return runBytes(store.suggestions, store.groups, undefined, marked);
    }

    // A plan of what the user marked by hand stands alone: the selection is not part of it.
    return phase.kind === 'review' && marked ? marked.allocated : store.selectedBytes;
  });

  const hero = $derived.by(() => {
    if (phase.kind === 'running') {
      return { label: 'Cleaned so far', bytes: runBytes(store.suggestions, store.groups, 'done', marked) };
    }

    if (phase.kind === 'done') {
      return { label: 'Freed', bytes: phase.result.freed };
    }

    return { label: 'Selected to free', bytes: planned };
  });

  // "Freed" is what the backend measured after the run; "Selected to free" is a figure of the old scan.
  const heroStale = $derived(stale && hero.label === 'Selected to free');

  /** How much is taken off the disk in the numbers on the right; undefined when that is not known. */
  const removed = $derived(phase.kind === 'done' ? phase.result.freed : planned);
  const after = $derived(removed === undefined ? undefined : afterCleanup(basis, removed));

  const afterText = $derived.by(() => {
    if (!after) {
      return 'Not measured';
    }

    const used = fmt.size(after.used);

    return after.share === undefined ? `${used} scanned` : `${used} used · ${Math.round(after.share * 100)}%`;
  });

  const freed = $derived.by(() => {
    if (phase.kind === 'done') {
      return phase.result.freed === undefined
        ? { caption: 'CLEANED', value: fmt.size(runBytes(store.suggestions, store.groups, 'done', marked)) }
        : { caption: 'FREED', value: fmt.size(phase.result.freed) };
    }

    return { caption: phase.kind === 'running' ? 'CLEANING' : 'WILL BE FREED', value: fmt.size(planned) };
  });

  const stays = $derived({
    caption: 'STAYS ON DISK',
    value: after ? fmt.size(after.used) : undefined,
  });

  const animated = countUp(() => hero.bytes ?? 0, 300);
</script>

<Card class="viz" aria-label="What a cleanup would free">
  <div class="hero">
    <div class:stale={heroStale} title={heroStale ? STALE_TEXT : undefined}>
      <div class="lab">{hero.label}</div>
      <b class="num" class:unknown={hero.bytes === undefined}>
        {hero.bytes === undefined ? 'Not measured' : fmt.size(animated.value)}
      </b>
    </div>

    <div class="s" class:stale title={stale ? STALE_TEXT : undefined}>
      <div class="lab">{subject} after cleanup</div>
      <b class="num">{afterText}</b>
    </div>
  </div>

  <FlowChart {items} total={basis.used} {freed} {stays} {stale} ontoggle={(id) => store.toggle(id)} />

  <div class="legend">
    <span><i style="background:var(--green)"></i>safe</span>
    <span><i style="background:var(--amber)"></i>needs review</span>
    <span><i style="background:var(--red)"></i>kept</span>
    <span><i style="background:var(--slate)"></i>everything else</span>
    <span class="sp"></span>
    {#if !locked}
      <span>Click a ribbon to move it between the two sides</span>
    {/if}
  </div>
</Card>

<style>
  .hero {
    display: flex;
    align-items: flex-end;
    gap: 36px;
    flex-wrap: wrap;
    margin-bottom: 22px;
    padding-bottom: 18px;
    border-bottom: 1px solid var(--line);
  }

  .lab {
    font-size: 11px;
    letter-spacing: 0.12em;
    color: var(--muted);
    text-transform: uppercase;
    margin-bottom: 4px;
  }

  .hero b {
    display: block;
    font-size: 38px;
    line-height: 1;
  }

  /* "Not measured" is a note, not a figure: smaller, on the same baseline. */
  .hero b.unknown {
    font-size: 24px;
    line-height: 38px;
  }

  .s {
    margin-left: auto;
    text-align: right;
  }

  .s b {
    font-size: 20px;
  }

  .legend {
    display: flex;
    gap: 18px;
    flex-wrap: wrap;
    margin-top: 16px;
    font-size: 12px;
    color: var(--muted);
  }

  .legend i {
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
