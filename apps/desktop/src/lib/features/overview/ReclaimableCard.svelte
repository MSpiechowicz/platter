<script lang="ts">
  import { delayed } from '../../state/delayed.svelte';
  import type { SuggestionsDto } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { biggestWins, reclaimSegments, VERDICT_COLORS } from './advisor';
  import { STALE_TEXT } from './cleanupText';
  import RetryNote from './RetryNote.svelte';

  interface Props {
    /** Undefined until the first answer arrives. */
    suggestions: SuggestionsDto | undefined;
    loading: boolean;
    error: unknown;
    onretry: () => void;
    /** The space the scan covers: what the bar's full width stands for. */
    scanned: number;
    /** Bytes freed by the cleanups of this session; 0 before any. Not part of the bar, which shows the scan. */
    freed: number;
    /** A cleanup ran since the scan: the bar, its rows and the wins describe the disk from before it, and are greyed. */
    stale?: boolean;
  }

  let { suggestions, loading, error, onretry, scanned, freed, stale = false }: Props = $props();

  const staleTitle = $derived(stale ? STALE_TEXT : undefined);

  const segments = $derived(reclaimSegments(suggestions, scanned));
  const wins = $derived(biggestWins(suggestions));

  // A spinner that appeared stays at least a second, so the content does not replace it after a flash.
  const shown = delayed(() => !suggestions && !error && loading);
</script>

<Card title="Reclaimable" class="reclaimable">
  {#if suggestions && !shown.value}
    <div
      class="rec"
      class:stale
      title={staleTitle}
      role="img"
      aria-label={segments.map((segment) => `${segment.label}: ${fmt.size(segment.allocated)}`).join(', ')}
    >
      <div class="fill">
        {#each segments as segment (segment.verdict)}
          <i style="width:{segment.share * 100}%;background:{segment.color}"></i>
        {/each}
      </div>
    </div>

    <div class="rows">
      {#each segments as segment (segment.verdict)}
        <div class="recr" class:stale title={staleTitle}>
          <i style="background:{segment.color}"></i>
          <span>{segment.label}</span>
          <span class="num">{fmt.size(segment.allocated)}</span>
        </div>
      {/each}

      <div class="recr">
        <i style="background:var(--accent)"></i>
        <span>Already freed</span>
        <span class="num">{fmt.size(freed)}</span>
      </div>

      {#if wins.length > 0}
        <h4 class="recsub">Biggest wins</h4>

        {#each wins as win (win.id)}
          <div class="recr" class:stale title={staleTitle}>
            <i style="background:{VERDICT_COLORS[win.verdict]}"></i>
            <span class="name">{win.title}</span>
            <span class="num">{fmt.size(win.allocated)}</span>
          </div>
        {/each}
      {/if}
    </div>
  {:else if error && !shown.value}
    <RetryNote {onretry} />
  {:else}
    <div class="wait"></div>
    <Loading when={loading || shown.value} label="Looking for things to clean…" />
  {/if}
</Card>

<style>
  .rec {
    height: 10px;
    border-radius: 2px;
    overflow: hidden;
    background: var(--track);
    margin-bottom: 12px;
  }

  .fill {
    display: flex;
    height: 100%;
    transform-origin: left;
    animation: fillx 0.8s var(--ease-out) both;
  }

  .fill i {
    display: block;
    height: 100%;
  }

  @keyframes fillx {
    from {
      transform: scaleX(0);
    }
  }

  .rows {
    flex: 1;
  }

  .recr {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    font-size: 12.5px;
    padding: 4px 0;
  }

  .recr i {
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }

  .recr span:last-child {
    color: var(--muted);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .recsub {
    font-size: 11px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--dim);
    font-weight: 600;
    margin: 8px 0 4px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
  }

  /* Keeps the card from collapsing while the answer is awaited, so the spinner has room. */
  .wait {
    min-height: 200px;
  }
</style>
