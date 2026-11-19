<script lang="ts">
  import type { HistoryCleanupDto } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import { visibleText } from '../overview/display';
  import { cleanedMost, timesText } from './cleanedMost';
  import { formatDay } from './format';

  interface Props {
    cleanups: readonly HistoryCleanupDto[];
    today: string;
    /** The days the history window reaches back. */
    days: number;
  }

  let { cleanups, today, days }: Props = $props();

  const rows = $derived(cleanedMost(cleanups));
</script>

<Card title="What you clean most" hint={`LAST ${days} DAYS`} grow class="most">
  <!-- A scrolling region must take the keyboard, or its rows cannot be reached without a mouse. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  {#if rows.length === 0}
    <p class="empty">Nothing cleaned in the last {days} days. What you clean most shows up here.</p>
  {:else}
    <div class="scroll" role="region" tabindex="0" aria-label="Items cleaned most">
      <ul>
        {#each rows as row (row.name)}
          <li>
            <div class="line">
              <span class="name">{visibleText(row.name)}</span>
              <span class="muted">{timesText(row.count)}</span>
            </div>
            <div class="bar" aria-hidden="true"><i style="width: {row.share * 100}%"></i></div>
            <div class="last">last on {formatDay(row.last, today)}</div>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</Card>

<style>
  .empty {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--muted);
  }

  /* Positioned, so anything visually hidden inside is clipped by the region and does not stretch the page. */
  .scroll {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    /* As in the Advisor: the scroll bar sits in the card's padding, and the rows keep the card's full width. */
    margin-right: -16px;
    padding-right: 16px;
  }

  /* The card is the box `cqw` is measured against. */
  :global(.most) {
    container-type: inline-size;
  }

  ul {
    /* Exactly as wide as the card's content, whatever the engine does with the scroll bar (as in the Advisor). */
    width: calc(100% - 16px);
    width: 100cqw;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li + li {
    margin-top: 12px;
  }

  .line {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: 13px;
  }

  .name {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .muted {
    flex: none;
  }

  .bar {
    height: 4px;
    margin-top: 5px;
    border-radius: 2px;
    background: var(--track);
    overflow: hidden;
  }

  .bar i {
    display: block;
    height: 100%;
    border-radius: 2px;
    background: var(--green);
  }

  .last {
    margin-top: 3px;
    font-size: 11.5px;
    color: var(--dim);
  }
</style>
