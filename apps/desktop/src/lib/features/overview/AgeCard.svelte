<script lang="ts">
  import type { AgeBucket, SummaryDto } from '../../api/types';
  import { percentText } from '../../format';
  import Card from '../../components/ui/Card.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { STALE_TEXT } from './cleanupText';

  interface Props {
    byAge: SummaryDto['byAge'];
    /** Everything that was scanned; the share is relative to it. */
    total: number;
    /** A cleanup ran since the scan: the chart describes the disk from before it, and is greyed. */
    stale?: boolean;
  }

  let { byAge, total, stale = false }: Props = $props();

  const LABELS: Record<AgeBucket, string> = {
    week: '< 1 wk',
    month: '1–4 wk',
    quarter: '1–3 mo',
    year: '3–12 mo',
    twoYears: '1–2 yr',
    older: '2+ yr',
  };

  /** Files not touched for over a year are drawn in slate. */
  const STALE = new Set<AgeBucket>(['twoYears', 'older']);

  const largest = $derived(Math.max(1, ...byAge.map((entry) => entry.allocated)));
  const untouched = $derived(
    byAge.filter((entry) => STALE.has(entry.bucket)).reduce((sum, entry) => sum + entry.allocated, 0),
  );
</script>

<Card title="Space by last modified" tag={stale ? 'OLD SCAN' : undefined} grow>
  <div class="lg">
    <span><i style="background:var(--cyan)"></i>changed within a year</span>
    <span><i style="background:var(--slate)"></i>untouched for over a year</span>
  </div>

  <div class="age" class:stale title={stale ? STALE_TEXT : undefined}>
    {#each byAge as entry, i (entry.bucket)}
      <div class="col">
        <b class="num">{fmt.size(entry.allocated)}</b>
        <div class="trk">
          <i
            style="height:{(entry.allocated / largest) * 100}%;--c:{STALE.has(entry.bucket)
              ? 'var(--slate)'
              : 'var(--cyan)'};animation-delay:{i * 70}ms"
          ></i>
        </div>
        <span>{LABELS[entry.bucket]}</span>
      </div>
    {/each}
  </div>

  <div class="sum" class:stale title={stale ? STALE_TEXT : undefined}>
    <b class="num">{fmt.size(untouched)}</b>
    <div>
      Untouched for over a year
      <small>{percentText(total > 0 ? untouched / total : 0)} of scanned space</small>
    </div>
  </div>
</Card>

<style>
  .lg {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    margin: -4px 0 12px;
    font-size: 11px;
    color: var(--muted);
  }

  .lg i {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    margin-right: 5px;
  }

  .age {
    flex: 1;
    display: flex;
    gap: 10px;
    min-height: 150px;
  }

  .col {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 5px;
    min-width: 0;
    font-size: 11.5px;
    color: var(--muted);
  }

  /* The space from a figure to the tallest bar matches the space from the labels to the line under the chart. */
  .col b {
    margin-bottom: 9px;
    color: var(--text);
    font-size: 12.5px;
    white-space: nowrap;
  }

  .trk {
    flex: 1;
    width: 100%;
    display: flex;
    align-items: flex-end;
  }

  .trk i {
    display: block;
    width: 100%;
    background: var(--c);
    border-radius: 2px 2px 0 0;
    transform-origin: bottom;
    animation: grow 0.8s var(--ease-out) both;
  }

  .col span {
    white-space: nowrap;
  }

  .sum {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-top: 14px;
    padding-top: 16px;
    border-top: 1px solid var(--line);
  }

  .sum b {
    font-size: 24px;
    line-height: 1;
    white-space: nowrap;
  }

  .sum div {
    font-size: 12.5px;
    line-height: 1.5;
  }

  small {
    display: block;
    font-size: 12px;
    color: var(--muted);
  }
</style>
