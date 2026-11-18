<script lang="ts">
  import type { HistoryOverviewDto } from '../../api/types';
  import { describeError } from '../../api/client';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import Segmented from '../../components/ui/Segmented.svelte';
  import { tooltip, type TooltipContent } from '../../components/ui/tooltip.svelte';
  import { delayed } from '../../state/delayed.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { settings } from '../../state/settings.svelte';
  import { niceStep, ticksBetween, tickLabels } from '../history/chart';
  import { DAYS_FOR_GROWTH, formatDay, thinNote, whatText } from '../history/format';
  import { DEFAULT_RANGE, usageDays, type UsageDay } from './usageDays';

  interface Props {
    /** The history of the disk. Undefined until it has been read. */
    data?: HistoryOverviewDto;
    loading?: boolean;
    /** Why the history could not be read. */
    error?: unknown;
  }

  let { data, loading = false, error }: Props = $props();

  const RANGE_OPTIONS = [
    { value: '30', label: '30 d' },
    { value: '91', label: '3 mo' },
    { value: '182', label: '6 mo' },
  ] as const;

  let rangeKey = $state<string>(String(DEFAULT_RANGE));

  const range = $derived(Number(rangeKey));

  // A spinner that appeared stays at least a second, so the content does not replace it after a flash.
  const shown = delayed(() => !data && !error && loading);

  // What the card says in the place of the plot: the history cannot be used, or it is too short to say anything.
  const message = $derived.by((): string | undefined => {
    if (!data) {
      return undefined;
    }

    if (data.problem) {
      return data.problem;
    }

    if (data.thin) {
      return thinNote(Math.min(data.samples.length, DAYS_FOR_GROWTH - 1));
    }

    return undefined;
  });

  const model = $derived(
    data && !message
      ? usageDays({ samples: data.samples, cleanups: data.cleanups, today: data.today }, range)
      : undefined,
  );

  // The plot is drawn at the width it has: it follows the card.
  const FALLBACK_WIDTH = 640;
  const HEIGHT = 150;
  const PAD = { left: 56, right: 8, top: 14, bottom: 38 };
  /** A label of a cleanup is skipped when it would touch the one before it. */
  const GLYPH = 6.2;

  let box: HTMLDivElement | undefined = $state();
  let measured = $state(0);

  $effect(() => {
    if (!box) {
      return;
    }

    const element = box;

    measured = element.clientWidth;

    if (typeof ResizeObserver === 'undefined') {
      return;
    }

    const observer = new ResizeObserver((entries) => {
      measured = entries[0]?.contentRect.width ?? element.clientWidth;
    });

    observer.observe(element);

    return () => observer.disconnect();
  });

  const width = $derived(measured > 0 ? measured : FALLBACK_WIDTH);

  const floor = HEIGHT - PAD.bottom;

  interface Column {
    day: UsageDay;
    /** Left edge and width of the day's slot. */
    x: number;
    w: number;
    /** The rise above the zero line and the fall below it. A cleanup day has both. */
    bars: { x: number; y: number; w: number; h: number; fill: string }[];
    /** What the cleanups of the day freed, written under the plot. Left out when it would touch a neighbour. */
    freedText?: string;
    /** The date written under this day, with a guide line up through the plot. */
    dateText?: string;
  }

  // The ticks are round numbers of the unit the user chose, as on the History chart. Rises and falls share one scale
  // with one step, so the gridlines read +20, 0, −20, −40 and not a different step on each side. The zero line sits
  // where that scale puts it: lower when the rises are bigger, higher when the falls are.
  const unit = $derived(settings.units === 'GiB' ? 1024 ** 3 : 1e9);

  const scale = $derived.by(() => {
    const maxUp = model?.maxUp ?? 0;
    const maxDown = model?.maxDown ?? 0;
    const step = niceStep(0, maxUp + maxDown, unit, 6);
    const edge = (max: number): number => (max > 0 ? Math.ceil(max / step - 1e-9) * step : 0);
    const up = edge(maxUp);
    const down = edge(maxDown);

    return {
      up: ticksBetween(step, up, step),
      down: ticksBetween(step, down, step),
      /** Bytes for each pixel, over the whole height from the top tick to the bottom one. */
      perPixel: (up + down || step) / (floor - PAD.top),
      top: up,
    };
  });

  const zero = $derived(PAD.top + scale.top / scale.perPixel);

  const yUp = (bytes: number): number => zero - bytes / scale.perPixel;
  const yDown = (bytes: number): number => zero + bytes / scale.perPixel;

  interface Gridline {
    y: number;
    label: string;
  }

  const gridlines = $derived.by((): Gridline[] => {
    const up = tickLabels(scale.up, fmt.size, unit, settings.units);
    const down = tickLabels(scale.down, fmt.size, unit, settings.units);

    return [
      ...scale.up.map((value, i) => ({ y: yUp(value), label: `+${up[i] ?? ''}` })),
      ...scale.down.map((value, i) => ({ y: yDown(value), label: `−${down[i] ?? ''}` })),
    ];
  });

  /** Days apart of the dates under the plot, counted back from today, so there are about six of them. */
  const dateEvery = $derived(range <= 30 ? 7 : range <= 91 ? 14 : 28);

  const columns = $derived.by((): Column[] => {
    if (!model) {
      return [];
    }

    const slot = (width - PAD.left - PAD.right) / model.days.length;
    let labelEnd = -Infinity;

    return model.days.map((day, index): Column => {
      const x = PAD.left + index * slot;
      const column: Column = { day, x, w: slot, bars: [] };
      const back = model.days.length - 1 - index;

      if (back % dateEvery === 0) {
        column.dateText = back === 0 ? 'today' : formatDay(day.day, data?.today);
      }
      const bx = x + Math.max(slot * 0.15, 0.5);
      const bw = Math.max(slot * 0.7, 1);

      // A day that grew by nothing still shows a hairline, so that it is not taken for a gap.
      if (day.up > 0 || day.kind === 'grew') {
        const h = Math.max(zero - yUp(day.up), 1);

        column.bars.push({ x: bx, y: zero - h, w: bw, h, fill: 'var(--amber)' });
      }

      if (day.down > 0) {
        const h = Math.max(yDown(day.down) - zero, 1);

        column.bars.push({
          x: bx,
          y: zero,
          w: bw,
          h,
          fill: day.kind === 'cleanup' ? 'var(--green)' : 'var(--slate)',
        });
      }

      if (day.freed > 0) {
        const text = fmt.size(day.freed);
        const half = (text.length * GLYPH) / 2;
        const centre = x + slot / 2;

        if (centre - half > labelEnd) {
          column.freedText = text;
          labelEnd = centre + half + 4;
        }
      }

      return column;
    });
  });

  const signed = (bytes: number): string => `${bytes < 0 ? '−' : '+'}${fmt.size(Math.abs(bytes))}`;

  /** The first line about a day: its change, or on a cleanup day what was written and what was freed. */
  const changeText = (day: UsageDay): string | undefined => {
    if (day.kind === 'cleanup') {
      const freed = `−${fmt.size(day.down)} freed`;

      return day.delta === undefined ? freed : `+${fmt.size(day.up)} written · ${freed}`;
    }

    return day.delta === undefined ? undefined : signed(day.delta);
  };

  /** The tooltip of a day: the date, the change, the cleanup if there was one, and the used space. Undefined for a day that says nothing. */
  const tipOf = (day: UsageDay): TooltipContent | undefined => {
    const items: string[] = [];
    const change = changeText(day);

    if (change) {
      items.push(change);
    }

    if (day.cleanups.length > 0) {
      items.push(`Cleanup · ${whatText(day.what)}`);
    }

    if (day.used !== undefined) {
      items.push(`${fmt.size(day.used)} used`);
    }

    return items.length > 0 ? { title: formatDay(day.day, data?.today), items } : undefined;
  };

  /** The same days for a screen reader: the plot itself is a picture. */
  const listed = $derived(
    (model?.days ?? [])
      .filter((day) => day.kind !== undefined)
      .map((day) => {
        const cleanup = day.cleanups.length > 0 ? `. Cleanup · ${whatText(day.what)}` : '';

        return `${formatDay(day.day, data?.today)}: ${changeText(day) ?? ''}${cleanup}`;
      }),
  );

  const summary = $derived(
    model
      ? `Change in used space by day over the last ${range} days. Disk change ${signed(model.net)}, ${fmt.size(model.freed)} freed by cleanups.`
      : '',
  );
</script>

<Card title="Space day by day">
  {#snippet actions()}
    {#if model}
      <span class="legend">
        <span><i style="background:var(--amber)"></i>written</span>
        <span><i style="background:var(--green)"></i>cleanup</span>
        <span><i style="background:var(--slate)"></i>shrank</span>
      </span>
      <Segmented options={RANGE_OPTIONS} bind:value={rangeKey} label="Range" size="sm" />
    {/if}
  {/snippet}

  {#if data && !shown.value}
    {#if message}
      <p class="note">{message}</p>
    {:else if model}
      <div class="body">
        <div class="plot" bind:this={box}>
          {#if model.bars > 0}
            <svg
              class="chart"
              {width}
              height={HEIGHT}
              viewBox="0 0 {width} {HEIGHT}"
              role="img"
              aria-label={summary}
            >
              {#each columns as column (column.day.day)}
                {#if column === columns.at(-1)}
                  <!-- Today has no guide: the gridlines end at the right edge of the plot, which is where today ends. -->
                  <text class="date" x={width - PAD.right} y={HEIGHT - 4} text-anchor="end">today</text>
                {:else if column.dateText}
                  <line
                    class="guide"
                    x1={column.x + column.w / 2}
                    x2={column.x + column.w / 2}
                    y1={PAD.top}
                    y2={floor}
                  />
                  <text class="date" x={column.x + column.w / 2} y={HEIGHT - 4} text-anchor="middle"
                    >{column.dateText}</text
                  >
                {/if}
              {/each}

              {#each gridlines as line (line.label)}
                <line class="grid" x1={PAD.left} x2={width - PAD.right} y1={line.y} y2={line.y} />
                <text class="axis" x={PAD.left - 8} y={line.y + 3.5} text-anchor="end">{line.label}</text>
              {/each}

              <line class="zero" x1={PAD.left} x2={width - PAD.right} y1={zero} y2={zero} />
              <text class="axis" x={PAD.left - 8} y={zero + 3.5} text-anchor="end">0</text>

              {#each columns as column (column.day.day)}
                {#each column.bars as bar (bar.fill)}
                  <rect class="bar" x={bar.x} y={bar.y} width={bar.w} height={bar.h} rx="1" fill={bar.fill} />
                {/each}

                {#if column.freedText}
                  <text class="freed" x={column.x + column.w / 2} y={floor + 14} text-anchor="middle"
                    >{column.freedText}</text
                  >
                {/if}
              {/each}
            </svg>

            <ul class="sr">
              {#each listed as line (line)}
                <li>{line}</li>
              {/each}
            </ul>

            <!-- One target for each day, as tall as the plot: the SVG cannot carry the tooltip of the app. -->
            <div
              class="hits"
              style="left:{PAD.left}px;right:{PAD.right}px;top:{PAD.top}px;height:{floor - PAD.top}px"
            >
              {#each columns as column (column.day.day)}
                {@const content = tipOf(column.day)}
                {#if content}
                  <div
                    class="hit"
                    style="left:{column.x - PAD.left}px;width:{column.w}px"
                    use:tooltip={content}
                  ></div>
                {/if}
              {/each}
            </div>
          {:else}
            <p class="note">No two days in a row are recorded in this range.</p>
          {/if}
        </div>

        <div class="side">
          <!-- Read top to bottom as a sum: added − freed = net. The labels say the direction, so the figures have no sign. -->
          <div class="stat">
            <span>Written</span>
            <b class="num">{fmt.size(model.added)}</b>
            {#if range !== 30}
              <small>about {fmt.size(model.perMonth)} a month</small>
            {/if}
          </div>
          <div class="stat">
            <span>Freed by cleanups</span>
            <b class="num down">{fmt.size(model.freed)}</b>
          </div>
          <div class="stat">
            <span>{model.net > 0 ? 'Disk fuller by' : model.net < 0 ? 'Disk emptier by' : 'No change'}</span>
            <b class="num net" class:up={model.net > 0}>{fmt.size(Math.abs(model.net))}</b>
          </div>
        </div>
      </div>
    {/if}
  {:else if error && !shown.value}
    <p class="note">The history could not be read: {describeError(error)}</p>
  {:else}
    <div class="wait" aria-busy="true">
      <Loading when={loading || shown.value} label="Reading the history…" />
    </div>
  {/if}
</Card>

<style>
  .legend {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-right: 6px;
    font-size: 11px;
    line-height: 1;
    letter-spacing: 0;
    text-transform: none;
    font-weight: 400;
    color: var(--muted);
  }

  /* The swatches sit on the baseline, as in the legends of the other Overview cards. */
  .legend i {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 5px;
    border-radius: 2px;
  }

  .body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 210px;
    gap: 20px;
    align-items: stretch;
  }

  .plot {
    position: relative;
    min-width: 0;
    min-height: 120px;
  }

  .chart {
    display: block;
  }

  .chart text {
    font-family: var(--font);
    font-size: 10px;
    fill: var(--dim);
  }

  .chart .freed {
    fill: var(--green-text);
  }

  /* The zero line is drawn like the other gridlines: a solid one clashed with the bars that start on it. */
  .zero,
  .grid {
    stroke: var(--line);
    stroke-dasharray: 2 3;
  }

  .guide {
    stroke: var(--line);
    opacity: 0.6;
  }

  /* Read by a screen reader, not seen. */
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
    list-style: none;
  }

  .hits {
    position: absolute;
  }

  .hit {
    position: absolute;
    top: 0;
    bottom: 0;
  }

  .hit:hover {
    background: var(--hover);
  }

  .side {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 8px;
    padding-left: 20px;
    border-left: 1px solid var(--line);
  }

  .stat span {
    display: block;
    font-size: 12px;
    color: var(--muted);
  }

  .stat b {
    display: block;
    font-size: 20px;
    font-weight: 600;
    line-height: 1.2;
  }

  /* The Net figure is plain text unless it grew: green is for what was freed. */
  .stat b.net {
    color: var(--text);
  }

  .stat b.net.up {
    color: var(--amber);
  }

  .stat b.down {
    color: var(--green-text);
  }

  /* A footnote to the figure: smaller and dimmer than the label above it, so the two do not read as one style. */
  .stat small {
    display: block;
    margin-top: 1px;
    font-size: 11px;
    color: var(--dim);
  }

  .note {
    margin: 0;
    padding: 6px 0;
    color: var(--muted);
    font-size: 12.5px;
    line-height: 1.45;
  }

  .wait {
    position: relative;
    min-height: 150px;
  }

  @media (max-width: 800px) {
    .body {
      grid-template-columns: 1fr;
    }

    .side {
      flex-direction: row;
      flex-wrap: wrap;
      gap: 12px 20px;
      padding: 12px 0 0;
      border-left: 0;
      border-top: 1px solid var(--line);
    }
  }
</style>
