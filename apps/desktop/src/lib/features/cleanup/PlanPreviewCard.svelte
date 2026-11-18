<script lang="ts">
  import { untrack } from 'svelte';
  import type { Api } from '../../api/client';
  import { describeError } from '../../api/client';
  import type { CleanupPlanDto, ScanId } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import Tag from '../../components/ui/Tag.svelte';
  import type { CleanupStore } from '../../state/cleanup.svelte';
  import { delayed } from '../../state/delayed.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { settings } from '../../state/settings.svelte';
  import { shownPath, visibleText } from '../overview/display';
  import { OUTCOME_NAMES, OUTCOME_NOTES, previewOf, requestOf, type FirstPreview } from './preview';

  /*
   * What the selection would do, from a plan the backend makes only to describe it: nothing is changed on the disk.
   * The backend keeps a few plans and drops the oldest, so a preview is asked for only while the flow is idle: a plan
   * that waits for the user's confirmation, or runs, is never in danger of being dropped by one.
   */
  interface Props {
    store: CleanupStore;
    scanId: ScanId;
    api: Pick<Api, 'planCleanup'>;
    /** The preview the page made while it waited: the card starts from it, and asks again only for another request. */
    initial?: FirstPreview;
  }

  let { store, scanId, api, initial }: Props = $props();

  // Only the first value counts: later plans come from this card's own requests.
  const first = untrack(() => initial);

  /** The preview is asked for this long after the selection last changed. */
  const DEBOUNCE_MS = 300;
  /** The card is never shorter than this; a window too short for it scrolls the page instead. */
  const MIN_HEIGHT = 240;
  /** The width at which the page is a single column. */
  const SINGLE_COLUMN = '(max-width: 1000px)';
  /** The bottom padding of the page, when the page has none to read. */
  const PAGE_PADDING = 20;

  // The plan on screen. It stays while a newer one is asked for, so the card does not blink at every click.
  let plan = $state.raw<CleanupPlanDto | undefined>(first?.plan);
  let failure = $state<string | undefined>(first?.failure);
  let loading = $state(false);

  // The key of the request that the plan (or the failure) on screen answers. A request with the same key is not asked.
  // Only a plan counts as answered: a failed first preview is asked again, so a passing error does not stay.
  let answered: string | undefined = first?.plan ? first.key : undefined;

  const selected = $derived(store.selectedItems);
  const preview = $derived(plan ? previewOf(plan, store.suggestions) : undefined);

  // What a request is about, by value: the selection reloaded as the same ids is not a new question.
  const key = $derived(requestOf(selected, settings).key);

  // A spinner that appeared stays at least a second, so the table does not replace it after a flash.
  const shown = delayed(() => !plan && !failure && loading);

  $effect(() => {
    const asked = key;
    const request = untrack(() => requestOf(selected, settings));

    // A cleanup ran or was cancelled: the plan describes a disk that is gone, and nothing more is asked.
    if (store.outdated) {
      plan = undefined;
      failure = undefined;
      answered = undefined;

      return;
    }

    // Not idle (a plan is under review or runs, a cleanup is done): nothing is asked, and what is on screen stays.
    if (store.phase.kind !== 'idle') {
      return;
    }

    if (request.ids.length === 0) {
      plan = undefined;
      failure = undefined;
      answered = undefined;

      return;
    }

    // What is on screen already answers this request (the page's first preview, or the selection is back as it was).
    if (asked === answered) {
      return;
    }

    loading = true;

    // Set when the selection changes again or the flow leaves idle: the answer to this request no longer counts.
    let stale = false;

    const timer = setTimeout(async () => {
      try {
        const answer = await api.planCleanup(scanId, request.ids, request.options);

        if (!stale) {
          plan = answer;
          failure = undefined;
          answered = asked;
        }
      } catch (error) {
        if (!stale) {
          plan = undefined;
          failure = describeError(error);
          answered = undefined;
        }
      } finally {
        if (!stale) {
          loading = false;
        }
      }
    }, DEBOUNCE_MS);

    return () => {
      stale = true;
      clearTimeout(timer);

      // No spinner outlives its request. A request that follows this one sets it again at once.
      loading = false;
    };
  });

  const items = (total: number): string => `${total} item${total === 1 ? '' : 's'}`;

  // The card fills the page down to its bottom padding, so the page has no scroll bar of its own and the table scrolls
  // inside the card. `cap` is that height. A short table makes a short card, and a single column is not capped at all
  // (the same width the style sheet below makes the page one column at).
  let content = $state<HTMLElement>();
  let cap = $state<number>();

  /** The distance from the top of the page, whatever the page is scrolled to and whatever the card is animated by. */
  function documentTop(element: HTMLElement): number {
    let top = 0;

    for (
      let current: Element | null = element;
      current instanceof HTMLElement;
      current = current.offsetParent
    ) {
      top += current.offsetTop;
    }

    return top;
  }

  function measure(): void {
    const card = content?.parentElement;

    if (!card) {
      return;
    }

    if (window.matchMedia(SINGLE_COLUMN).matches) {
      cap = undefined;

      return;
    }

    const padding = parseFloat(getComputedStyle(card.closest('main') ?? document.body).paddingBottom);
    const room = window.innerHeight - (Number.isNaN(padding) ? PAGE_PADDING : padding) - documentTop(card);

    cap = Math.max(MIN_HEIGHT, Math.floor(room));
  }

  // The top of the card moves with whatever is above it, and the card gives way: that keeps the page as tall as the
  // window, so only what is above is watched.
  $effect(() => {
    const card = content?.parentElement;

    if (!card) {
      return;
    }

    measure();
    window.addEventListener('resize', measure);

    if (typeof ResizeObserver === 'undefined') {
      return () => window.removeEventListener('resize', measure);
    }

    const grid = card.parentElement;
    const observer = new ResizeObserver(measure);

    // The elements above the card, and the grid itself: it is as tall as they are.
    const watch = (): void => {
      observer.disconnect();

      if (grid) {
        observer.observe(grid);
      }

      for (const above of [...before(card), ...(grid ? before(grid) : [])]) {
        observer.observe(above);
      }
    };

    // What is above the card can be replaced (the Advisor for the result): the new elements are watched too.
    const changes = new MutationObserver(() => {
      watch();
      measure();
    });

    watch();

    if (grid) {
      changes.observe(grid, { childList: true });
    }

    return () => {
      window.removeEventListener('resize', measure);
      observer.disconnect();
      changes.disconnect();
    };
  });

  /** The elements that come before `element` in its parent. */
  function before(element: Element): Element[] {
    const siblings = [...(element.parentElement?.children ?? [])];

    return siblings.slice(0, siblings.indexOf(element));
  }
</script>

<Card
  title="What happens when you clean"
  hint={preview ? `${items(preview.items).toUpperCase()} · ${fmt.size(preview.bytes)}` : undefined}
  class="plan-preview"
  style={cap === undefined ? undefined : `max-height: ${cap}px`}
>
  <div class="content" bind:this={content}>
    {#if selected.length === 0}
      <p class="note">Select items in the Advisor to see what cleaning them does.</p>
    {:else if preview && !shown.value}
      <div
        class="bar"
        role="img"
        aria-label={preview.totals
          .filter((total) => total.bytes > 0)
          .map((total) => `${OUTCOME_NAMES[total.outcome]}: ${fmt.size(total.bytes)}`)
          .join(', ')}
      >
        {#each preview.totals.filter((total) => total.bytes > 0) as total (total.outcome)}
          <i class={total.outcome} style="flex-grow: {total.bytes}"></i>
        {/each}
      </div>

      <div class="legend">
        {#each preview.totals.filter((total) => total.bytes > 0) as total (total.outcome)}
          <div class="entry {total.outcome}">
            <span class="swatch"></span>
            <div>
              <div>
                <span class="muted">{OUTCOME_NAMES[total.outcome]}</span>
                <b class="num">{fmt.size(total.bytes)}</b>
              </div>
              <div class="rule">{OUTCOME_NOTES[total.outcome]}</div>
            </div>
          </div>
        {/each}
      </div>

      {#if preview.rows.length > 0}
        <!-- A scrolling region must take the keyboard, or its rows cannot be reached without a mouse. -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="scroll" role="region" tabindex="0" aria-label="What happens when you clean, by item">
          <table aria-label="What happens to each item">
            <thead>
              <tr>
                <th>Item</th>
                <th>What Platter does</th>
                <th>How it comes back</th>
                <th class="size">Size</th>
              </tr>
            </thead>
            <tbody>
              {#each preview.rows as row (row.ruleId)}
                <tr class={row.outcome}>
                  <td class="item">
                    <span class="sr">{OUTCOME_NAMES[row.outcome]}: </span>
                    <div>{visibleText(row.title)}</div>
                    {#if row.path}
                      <div class="mono path">{shownPath(row.path)}</div>
                    {/if}
                  </td>
                  <td>
                    <div class="does">
                      {#if row.command}
                        <span class="mono command">{visibleText(row.command)}</span>
                      {:else}
                        <span>{row.does}</span>
                      {/if}
                      {#if row.needsElevation}
                        <Tag>admin password</Tag>
                      {/if}
                    </div>
                  </td>
                  <td class="muted">{row.reason ? visibleText(row.reason) : ''}</td>
                  <td class="size num">{fmt.size(row.bytes)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else}
        <p class="note">Nothing from this selection can be cleaned.</p>
      {/if}

      {#if preview.refused > 0}
        <p class="note refused">{items(preview.refused)} will be left alone</p>
      {/if}
    {:else if failure && !shown.value}
      <p class="note" role="alert">{visibleText(failure)}</p>
    {:else if store.outdated}
      <!-- After a cleanup or an undo the scan no longer matches the disk, so nothing can be planned on it. -->
      <p class="note">The scan is out of date. Scan again to see what cleaning would do.</p>
    {:else}
      <div class="wait"><Loading when={loading || shown.value} label="Planning the cleanup…" /></div>
    {/if}
  </div>
</Card>

<style>
  /* The card spans the whole grid of the page and gives way to the window (see `cap`). */
  :global(.plan-preview) {
    grid-column: 1 / -1;
    min-height: 240px;
  }

  /* In a single column the card is one among the others, as tall as its table. */
  @media (max-width: 1000px) {
    :global(.plan-preview) {
      min-height: 0;
    }
  }

  .content {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* For screen readers only: the colour of a row says nothing to them. */
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .note {
    margin: 0;
    font-size: 12.5px;
    color: var(--muted);
  }

  .refused {
    margin-top: 10px;
  }

  .wait {
    position: relative;
    min-height: 140px;
  }

  /* One colour for each outcome, on the bar, the legend and the left edge of the rows. */
  .bar i.trash,
  .legend .trash .swatch {
    background: var(--cyan);
  }

  .bar i.deleted,
  .legend .deleted .swatch {
    background: var(--amber);
  }

  .bar i.tool,
  .legend .tool .swatch {
    background: var(--violet);
  }

  tr.trash td.item {
    border-left-color: var(--cyan);
  }

  tr.deleted td.item {
    border-left-color: var(--amber);
  }

  tr.tool td.item {
    border-left-color: var(--violet);
  }

  .bar {
    display: flex;
    gap: 2px;
    height: 12px;
    border-radius: 4px;
    overflow: hidden;
    background: var(--track);
    flex: none;
  }

  .bar i {
    flex-basis: 0;
    flex-shrink: 0;
  }

  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 28px;
    margin: 10px 0 14px;
    font-size: 13px;
    flex: none;
  }

  /* The swatch has a column of its own, so the note starts exactly under the name whatever the swatch is. */
  .entry {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: 8px;
  }

  .swatch {
    width: 8px;
    height: 8px;
    margin-top: 5px;
    border-radius: 2px;
  }

  .legend b {
    margin-left: 4px;
  }

  .rule {
    font-size: 11.5px;
    color: var(--dim);
  }

  /* The table's header stays put, and the rows scroll under it. */
  /* Positioned, so the visually hidden outcome names (absolute) are clipped by it and do not stretch the page. */
  .scroll {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }

  table {
    width: 100%;
    border-collapse: separate;
    border-spacing: 0;
    font-size: 13px;
  }

  th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--card);
    text-align: left;
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
    font-weight: 500;
    padding: 6px 12px 6px 0;
  }

  th:first-child {
    padding-left: 11px;
  }

  td {
    padding: 8px 12px 8px 0;
    border-top: 1px solid var(--line);
    vertical-align: top;
  }

  td.item {
    padding-left: 8px;
    border-left: 3px solid transparent;
  }

  .path {
    font-size: 11px;
    color: var(--dim);
  }

  .does {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 8px;
  }

  .command {
    font-size: 12px;
  }

  .size {
    text-align: right;
    padding-right: 8px;
    white-space: nowrap;
  }

  th.size {
    padding-right: 8px;
  }
</style>
