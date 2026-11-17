<script lang="ts">
  import type { HistoryCleanupDto, HistorySampleDto } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { settings } from '../../state/settings.svelte';
  import { CHART, layoutChart, tickLabels } from './chart';
  import { DAYS_FOR_GROWTH, formatDay, methodText, thinNote, whatText } from './format';

  interface Props {
    samples: readonly HistorySampleDto[];
    without: readonly number[];
    cleanups: readonly HistoryCleanupDto[];
    today: string;
    /** Too few days to say how fast the disk grows: the card says so under the chart. */
    thin: boolean;
  }

  let { samples, without, cleanups, today, thin }: Props = $props();

  // The ticks are round numbers of the unit the user chose.
  const unit = $derived(settings.units === 'GiB' ? 1024 ** 3 : 1e9);
  const model = $derived(layoutChart({ samples, without, cleanups }, unit));
  const labels = $derived(
    model
      ? tickLabels(
          model.ticks.map((tick) => tick.value),
          fmt.size,
          unit,
          settings.units,
        )
      : [],
  );

  const markerTitle = (cleanup: HistoryCleanupDto): string => {
    const freed = cleanup.methods.every((method) => method.kind === 'trash')
      ? `${fmt.size(cleanup.freed)} to the trash`
      : `−${fmt.size(cleanup.freed)}`;

    return `${formatDay(cleanup.day, today)} · ${whatText(cleanup.what)} · ${freed} · ${methodText(cleanup.methods)}`;
  };
</script>

<Card title="Disk usage with and without cleanups">
  {#snippet actions()}
    <span class="legend">
      <span><i class="disk"></i>your disk</span>
      <span><i class="dash"></i>without cleanups</span>
      <span><i class="saved"></i>saved</span>
    </span>
  {/snippet}

  {#if model}
    <svg
      class="chart"
      viewBox="0 0 {CHART.width} {CHART.height}"
      role="img"
      aria-label="Used space of the disk by day, with and without the cleanups"
    >
      {#each model.ticks as tick, index (tick.value)}
        <line class="grid" x1={CHART.left} x2={CHART.right} y1={tick.y} y2={tick.y} />
        <text x={CHART.left - 10} y={tick.y + 4} text-anchor="end">{labels[index]}</text>
      {/each}

      {#each model.dates as date (date.x)}
        <text class="date" x={date.x} y={CHART.dateY} text-anchor="middle">{formatDay(date.day, today)}</text>
      {/each}

      {#if model.saved > 0}
        <path d={model.gapPath} class="gap" />
        <path d={model.withoutPath} class="without" />
      {/if}
      <path d={model.usedPath} class="used" />

      {#each model.markers as marker (marker.cleanup.id)}
        <g class="marker">
          <title>{markerTitle(marker.cleanup)}</title>
          <circle cx={marker.x} cy={marker.y} r="4" />
        </g>
      {/each}

      <circle class="dot disk" cx={model.used.x} cy={model.used.y} r="4" />
      <text class="value disk" x={model.used.x + 12} y={model.used.labelY + 4}
        >{fmt.size(model.used.value)}</text
      >
      <text x={model.used.x + 12} y={model.used.labelY + 19}>today</text>

      {#if model.saved > 0}
        <circle class="dot slate" cx={model.without.x} cy={model.without.y} r="4" />
        <text class="value" x={model.without.x + 12} y={model.without.labelY + 4}>
          {fmt.size(model.without.value)}
        </text>
        <text x={model.without.x + 12} y={model.without.labelY + 19}>without cleanups</text>

        {#if model.savedBracket}
          <line
            class="bracket"
            x1={model.savedBracket.x}
            x2={model.savedBracket.x}
            y1={model.savedBracket.y1}
            y2={model.savedBracket.y2}
          />
          <text
            class="saved-text"
            x={model.savedBracket.x - 10}
            y={model.savedBracket.textY + 4}
            text-anchor="end"
          >
            {fmt.size(model.saved)} saved
          </text>
        {/if}
      {/if}
    </svg>
  {:else}
    <p class="empty">The chart appears once a day is recorded.</p>
  {/if}

  {#if thin}
    <p class="note">{thinNote(Math.min(samples.length, DAYS_FOR_GROWTH - 1))}</p>
  {/if}
</Card>

<style>
  .legend {
    display: flex;
    align-items: center;
    gap: 16px;
    font-size: 12px;
    line-height: 1;
    letter-spacing: 0;
    text-transform: none;
    font-weight: 400;
    color: var(--muted);
  }

  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .legend i {
    display: block;
    flex: none;
    /* Centred on the lowercase letters of the label, not on its line box (which reaches up to capitals). */
    transform: translateY(1px);
  }

  .legend .disk {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--cyan);
  }

  .legend .dash {
    width: 14px;
    border-top: 2px dashed var(--slate);
  }

  .legend .saved {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--green);
    opacity: 0.5;
  }

  .chart {
    display: block;
    width: 100%;
    height: auto;
  }

  .chart text {
    font-family: var(--font);
    font-size: 11px;
    fill: var(--muted);
  }

  .chart .date {
    fill: var(--dim);
  }

  .grid {
    stroke: var(--line);
    stroke-dasharray: 2 5;
  }

  .gap {
    fill: var(--green);
    fill-opacity: 0.16;
  }

  .without {
    fill: none;
    stroke: var(--slate);
    stroke-width: 1.5;
    stroke-dasharray: 5 5;
  }

  .used {
    fill: none;
    stroke: var(--cyan);
    stroke-width: 2;
    stroke-linejoin: round;
  }

  .marker circle {
    fill: var(--green);
    stroke: var(--card);
    stroke-width: 1.5;
  }

  .dot.disk {
    fill: var(--cyan);
  }

  .dot.slate {
    fill: var(--slate);
  }

  .chart .value {
    fill: var(--text);
    font-size: 12px;
    font-weight: 600;
  }

  .chart .value.disk {
    fill: var(--cyan-text);
  }

  .bracket {
    stroke: var(--green);
    stroke-width: 2;
  }

  .chart .saved-text {
    fill: var(--green-text);
    font-size: 14px;
    font-weight: 600;
  }

  .empty {
    margin: 0;
    padding: 70px 0;
    text-align: center;
    color: var(--muted);
    font-size: 12.5px;
  }

  .note {
    margin: 10px 0 0;
    color: var(--muted);
    font-size: 12px;
    line-height: 1.45;
  }
</style>
