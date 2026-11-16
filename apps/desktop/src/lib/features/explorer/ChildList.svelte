<script lang="ts">
  import { SvelteSet } from 'svelte/reactivity';
  import type { NodeDto, NodeId, SortKey } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import Dropdown from '../../components/ui/Dropdown.svelte';
  import { formatCount } from '../../format';
  import { fmt } from '../../state/fmt.svelte';
  import { moveFocus } from './roving';
  import { countLabels, matches, normalizeQuery, segments, SEARCH_LIMIT } from './search';

  interface Props {
    /** The folder shown; the not-counted rows fold up again when it changes. */
    folder: NodeId;
    /** The rows that have a size. */
    items: readonly NodeDto[];
    /** How many items have a size (the most it can be while pages are still on their way). */
    total: number;
    /** The rows with no size (not read, empty, another disk), by name. */
    uncounted: readonly NodeDto[];
    /** How many there are in all, or `undefined` while that is not known yet. */
    uncountedTotal?: number;
    sort: SortKey;
    selected: NodeId | null;
    /** Rows that belong to the selected "smaller items" tile: lit like a selected row, and scrolled to. */
    highlighted?: ReadonlySet<NodeId>;
    /** More rows must be loaded before the highlighted ones are all there. */
    wantMore?: boolean;
    hovered: NodeId | null;
    colorOf: (id: NodeId) => string;
    /** The list is on its way. */
    loading: boolean;
    error?: string;
    hasMore: boolean;
    loadingMore: boolean;
    /** The rows shown are not the page of the current folder and sort yet, so more cannot be asked for. */
    moreDisabled?: boolean;
    moreError: string | null;
    /** What the list is filtered by (applied, not what is being typed); `''` for no filter. */
    query?: string;
    /** The search is still loading pages in the background. */
    searching?: boolean;
    /** The folder has more items than the search looks through. */
    searchTruncated?: boolean;
    /** How many items of the folder are in all, counted or not: the search's progress is out of this. */
    allTotal?: number;
    /** How many of them are loaded. */
    loaded?: number;
    onquery?: (query: string) => void;
    onsort: (sort: SortKey) => void;
    onchoose: (id: NodeId) => void;
    onhover: (id: NodeId | null) => void;
    onmore: () => void;
    onretry: () => void;
  }

  let {
    folder,
    items,
    total,
    uncounted,
    uncountedTotal,
    sort,
    selected,
    highlighted = new SvelteSet(),
    wantMore = false,
    hovered,
    colorOf,
    loading,
    error,
    hasMore,
    loadingMore,
    moreDisabled = false,
    moreError,
    query = '',
    searching = false,
    searchTruncated = false,
    allTotal = 0,
    loaded = 0,
    onquery,
    onsort,
    onchoose,
    onhover,
    onmore,
    onretry,
  }: Props = $props();

  const SORTS = [
    { value: 'sizeDesc', label: 'Size' },
    { value: 'nameAsc', label: 'Name' },
    { value: 'filesDesc', label: 'Files' },
    { value: 'mtimeDesc', label: 'Modified' },
  ] satisfies { value: SortKey; label: string }[];

  // The Dropdown knows the four choices; the API's ascending size order is not offered.
  const sortValue = $derived(sort === 'sizeAsc' ? 'sizeDesc' : sort);

  // The filter: rows whose name contains the query, still in the list's order.
  const needle = $derived(normalizeQuery(query));
  const filtering = $derived(needle !== '');
  const shown = $derived(filtering ? items.filter((item) => matches(item.name, needle)) : items);
  const shownUncounted = $derived(
    filtering ? uncounted.filter((item) => matches(item.name, needle)) : uncounted,
  );
  const uncountedCount = $derived(filtering ? shownUncounted.length : (uncountedTotal ?? 0));

  const largest = $derived(items.reduce((most, item) => Math.max(most, item.allocated), 0) || 1);
  const footer = $derived(
    uncountedCount === 1 ? '1 item not counted' : `${formatCount(uncountedCount)} items not counted`,
  );
  const nothingFound = $derived(
    filtering && shown.length === 0 && shownUncounted.length === 0 && !searching && !loading,
  );

  // The count: the longest text that fits next to the title and the sort. Every text is measured in a hidden copy.
  const labels = $derived(countLabels(total, filtering ? shown.length : undefined));
  const GAP = 8;
  /** The space between the title and the count, and between the count and the sort; the middle one is spare. */
  const GAPS = 2;
  let head: HTMLDivElement | undefined = $state();
  let title: HTMLElement | undefined = $state();
  let sorter: HTMLElement | undefined = $state();
  let sizer: HTMLElement | undefined = $state();
  let room = $state(Number.POSITIVE_INFINITY);
  let widths = $state<number[]>([]);
  const count = $derived(
    labels.find((_, index) => (widths[index] ?? 0) <= room) ?? (labels.at(-1) as string),
  );

  function measure(): void {
    if (!head || !title || !sorter || !sizer || head.clientWidth === 0) {
      room = Number.POSITIVE_INFINITY;
      widths = [];

      return;
    }

    room = head.clientWidth - title.offsetWidth - sorter.offsetWidth - GAP * GAPS;
    widths = [...sizer.children].map((label) => (label as HTMLElement).offsetWidth);
  }

  $effect(() => {
    if (!head || typeof ResizeObserver === 'undefined') {
      return;
    }

    const observer = new ResizeObserver(measure);

    observer.observe(head);

    return () => observer.disconnect();
  });

  // The labels change with the numbers: measure again once they are on the page.
  $effect(() => {
    void labels;
    queueMicrotask(measure);
  });

  // The search field: what is typed is applied after a short pause.
  const DEBOUNCE_MS = 120;
  let field: HTMLInputElement | undefined = $state();
  let draft = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;

  function type(): void {
    clearTimeout(timer);
    timer = setTimeout(() => onquery?.(draft), DEBOUNCE_MS);
  }

  function clear(): void {
    clearTimeout(timer);
    draft = '';
    onquery?.('');
  }

  $effect(() => () => clearTimeout(timer));

  function clearAndFocus(): void {
    clear();
    field?.focus();
  }

  function onFieldKey(event: KeyboardEvent): void {
    if (event.key === 'Escape' && draft !== '') {
      // The first Escape only clears the field; going up a level is for an Escape with nothing to clear.
      event.preventDefault();
      clear();
    } else if (event.key === 'ArrowDown') {
      const first = list?.querySelector<HTMLElement>('.row');

      if (first) {
        event.preventDefault();
        first.focus();
      }
    }
  }

  /** Puts the cursor in the search field, with its text selected. */
  export function focusSearch(): void {
    field?.focus();
    field?.select();
  }

  // Not counted rows are folded; a different folder starts folded again.
  let expanded = $state(false);

  $effect(() => {
    void folder;
    expanded = false;
  });

  // The search starts again with each folder.
  $effect(() => {
    void folder;
    clearTimeout(timer);
    draft = '';
  });

  /** Why a row has no size. */
  function reason(item: NodeDto): string {
    if (item.kind === 'mount') {
      // The backend says why a mount was not entered; older data without the field reads "other disk".
      switch (item.skipped) {
        case 'system':
          return 'system folder';
        case 'duplicate':
          return 'counted elsewhere';
        case 'unknown':
          return 'not read';
        default:
          return 'other disk';
      }
    }

    return item.unreadable ? 'no access' : 'empty';
  }

  let list: HTMLDivElement | undefined = $state();

  // The row of the selected item comes into view, also when it was picked in the map.
  $effect(() => {
    if (selected === null || !list) {
      return;
    }

    list.querySelector<HTMLElement>(`[data-id="${selected}"]`)?.scrollIntoView?.({ block: 'nearest' });
  });

  // The rows of a selected "smaller items" tile come into view, and more are loaded while some are still missing.
  $effect(() => {
    if (highlighted.size === 0 || !list) {
      return;
    }

    const first = [...list.querySelectorAll<HTMLElement>('[data-id]')].find((row) =>
      highlighted.has(Number(row.dataset.id)),
    );

    first?.scrollIntoView?.({ block: 'nearest' });
  });

  $effect(() => {
    if (wantMore && hasMore && !loadingMore && !moreDisabled) {
      onmore();
    }
  });

  // One row is in the tab order; the arrow keys move between rows.
  const tabbable = $derived(shown.find((item) => item.id === selected)?.id ?? shown[0]?.id);

  function onkeydown(event: KeyboardEvent): void {
    moveFocus(event, [...(list?.querySelectorAll<HTMLElement>('.row') ?? [])]);
  }
</script>

<Card class="child-list" grow>
  <div class="head" bind:this={head}>
    <h3 bind:this={title}>In this folder</h3>
    <span class="count">{count}</span>
    <span class="sp"></span>
    <div class="sort" bind:this={sorter}>
      <Dropdown options={SORTS} value={sortValue} label="Sort by" onchange={onsort} />
    </div>
    <!-- Hidden copies of every count text: how wide each is decides which one is shown. -->
    <span class="sizer count" aria-hidden="true" bind:this={sizer}>
      {#each labels as label (label)}<span data-label={label}></span>{/each}
    </span>
  </div>

  <div class="find" class:on={filtering}>
    <svg viewBox="0 0 16 16" aria-hidden="true"
      ><circle cx="7" cy="7" r="4.5" /><path d="M10.5 10.5L14 14" /></svg
    >
    <input
      type="text"
      bind:this={field}
      bind:value={draft}
      data-explorer-search
      placeholder="Search in this folder"
      aria-label="Search in this folder"
      autocomplete="off"
      spellcheck="false"
      oninput={type}
      onkeydown={onFieldKey}
    />
    {#if draft !== ''}
      <button type="button" class="clear" aria-label="Clear the search" onclick={clearAndFocus}>
        <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M3 3l6 6M9 3L3 9" /></svg>
      </button>
    {/if}
  </div>

  <div class="progress" class:on={searching} aria-hidden={!searching}>
    <i style="width:{allTotal > 0 ? Math.min(100, (loaded / allTotal) * 100) : 0}%"></i>
  </div>

  <div class="rows" bind:this={list} aria-busy={loading}>
    <div class="inner">
      {#if error}
        <p class="note">
          The list could not be read: {error}
          <button type="button" class="more" onclick={onretry}>Try again</button>
        </p>
      {:else if items.length === 0 && !loading && !uncountedTotal}
        <p class="note">Nothing in this folder.</p>
      {/if}

      {#if searching}
        <p class="note" role="status">Searching… {formatCount(loaded)} of {formatCount(allTotal)}</p>
      {/if}

      {#each shown as item (item.id)}
        <button
          type="button"
          class="row"
          class:sel={item.id === selected || highlighted.has(item.id)}
          class:hot={item.id === hovered}
          style="--c:{colorOf(item.id)}"
          data-id={item.id}
          aria-pressed={item.id === selected}
          tabindex={item.id === tabbable ? 0 : -1}
          onclick={() => onchoose(item.id)}
          {onkeydown}
          onmouseenter={() => onhover(item.id)}
          onmouseleave={() => onhover(null)}
        >
          <i class="dot"></i>
          <span class="nm" title={item.name}
            >{#each segments(item.name, needle) as part, index (index)}{#if part.hit}<mark>{part.text}</mark
                >{:else}{part.text}{/if}{/each}</span
          >
          <span class="bar" aria-hidden="true"><i style="width:{(item.allocated / largest) * 100}%"></i></span
          >
          <b class="num">{fmt.size(item.allocated)}</b>
        </button>
      {/each}

      {#if nothingFound}
        <p class="note">No folder or file here matches "{needle}"</p>
      {/if}

      {#if uncountedCount > 0}
        <button
          type="button"
          class="foot"
          aria-expanded={expanded}
          aria-controls="uncounted-rows"
          onclick={() => (expanded = !expanded)}
        >
          <svg viewBox="0 0 16 16" aria-hidden="true" class:open={expanded}
            ><path d="M6 3.5L10.5 8 6 12.5" /></svg
          >
          {footer}
        </button>

        {#if expanded}
          <div id="uncounted-rows">
            {#each shownUncounted as item (item.id)}
              <div class="row off" data-uncounted={item.id}>
                <i class="dot"></i>
                <span class="nm" title={item.name}
                  >{#each segments(item.name, needle) as part, index (index)}{#if part.hit}<mark
                        >{part.text}</mark
                      >{:else}{part.text}{/if}{/each}</span
                >
                <span class="why">{reason(item)}</span>
              </div>
            {/each}
          </div>
        {/if}
      {/if}

      {#if hasMore && !filtering}
        <button type="button" class="more" disabled={loadingMore || moreDisabled} onclick={onmore}>
          {loadingMore ? 'Loading…' : `Load more (${formatCount(total - items.length)} left)`}
        </button>
      {/if}
      {#if searchTruncated}
        <p class="note">Searched the first {formatCount(SEARCH_LIMIT)} of {formatCount(allTotal)} items</p>
      {/if}
      {#if moreError}<p class="note">{moreError}</p>{/if}
    </div>
  </div>
</Card>

<style>
  /* The card is the box `cqw` is measured against. */
  :global(.child-list) {
    container-type: inline-size;
  }

  /*
   * One left edge and one right edge for the whole card: the title, the search field, the dots on the left; the sort,
   * the field, the sizes and the separator lines on the right. The list scrolls in a box that reaches out over the
   * card's padding (so the highlight of a row, which is 8 px wider than the row on both sides, is not cut off, and
   * the scroll bar sits in the padding, not on top of the data); the rows are then given the card's content width.
   */
  .head {
    position: relative;
    display: flex;
    align-items: center;
    margin-bottom: 8px;
    min-height: 30px;
  }

  h3 {
    flex: none;
    font-size: 11px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    white-space: nowrap;
    color: var(--muted);
    font-weight: 600;
  }

  .sp {
    flex: 1;
    min-width: 0;
  }

  .count {
    margin-left: 8px;
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    font-size: 10.5px;
    letter-spacing: 0.08em;
    color: var(--dim);
    text-transform: uppercase;
    white-space: nowrap;
    font-weight: 500;
  }

  .sizer {
    position: absolute;
    visibility: hidden;
    pointer-events: none;
    overflow: visible;
    display: flex;
    gap: 40px;
    left: 0;
    top: 0;
  }

  /* The text is in the attribute, so that it is measured but is not a second copy of the count in the page. */
  .sizer span::before {
    content: attr(data-label);
  }

  /* A small version of the shared dropdown. */
  .sort {
    flex: none;
    margin-left: 8px;
  }

  .sort :global(.trigger) {
    padding: 3px 8px 3px 10px;
    font-size: 12px;
    gap: 8px;
  }

  .find {
    position: relative;
    display: flex;
    align-items: center;
    margin-bottom: 4px;
    background: var(--card2);
    border: 1px solid var(--line);
    border-radius: 7px;
    transition: border-color 0.15s;
  }

  .find:hover,
  .find:focus-within,
  .find.on {
    border-color: var(--accent);
  }

  .find > svg {
    flex: none;
    width: 14px;
    height: 14px;
    margin-left: 10px;
    fill: none;
    stroke: var(--muted);
    stroke-width: 1.6;
    stroke-linecap: round;
  }

  .find input {
    flex: 1;
    min-width: 0;
    padding: 6px 10px 6px 8px;
    font-size: 12.5px;
    color: var(--text);
    background: none;
    border: 0;
    outline: none;
  }

  .find input::placeholder {
    color: var(--dim);
  }

  .clear {
    flex: none;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    margin-right: 3px;
    border-radius: 5px;
    color: var(--muted);
  }

  .clear:hover {
    color: var(--text);
  }

  .clear svg {
    width: 11px;
    height: 11px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
  }

  /* A thin line under the field while pages are still arriving; it keeps its place when it is not shown. */
  .progress {
    height: 2px;
    margin-bottom: 4px;
    border-radius: 1px;
    overflow: hidden;
    opacity: 0;
  }

  .progress.on {
    opacity: 1;
    background: var(--track);
  }

  .progress i {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.2s;
  }

  /*
   * The scroller reaches 8 px out on the left (the highlight of a row) and 16 px out on the right (the highlight and
   * the scroll bar, which sits in the card's padding). Whether there is a scroll bar, and how wide it is, changes how
   * wide the scroller's own box is, so the content does not depend on that: `.inner` is exactly as wide as the card's
   * content (`cqw`, the width of the card), whatever the engine does with scroll bars.
   */
  .rows {
    flex: 1;
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
    margin: 0 -16px 0 -8px;
  }

  .inner {
    width: calc(100% - 8px);
    width: 100cqw;
    margin-left: 8px;
  }

  .row {
    position: relative;
    isolation: isolate;
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) 64px 56px;
    gap: 10px;
    align-items: center;
    width: 100%;
    padding: 10px 0;
    font-size: 13px;
    text-align: left;
  }

  .row + .row {
    border-top: 1px solid var(--line);
  }

  /* The highlight reaches 8 px past the content on both sides; the content does not move. */
  .row::before {
    content: '';
    position: absolute;
    inset: 0 -8px;
    z-index: -1;
    border-radius: 6px;
  }

  .row:hover::before,
  .row.hot::before,
  .row.sel::before {
    background: #ffffff0a;
  }

  .row:focus-visible {
    outline: none;
  }

  .row:focus-visible::before {
    background: #ffffff0a;
    box-shadow: 0 0 0 1.5px var(--accent);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--c);
  }

  .nm {
    font-family: var(--mono);
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* What was searched for: bold, and underlined in the accent colour. No background. */
  .nm mark {
    background: none;
    color: #fff;
    font-weight: 700;
    text-decoration: underline;
    text-decoration-color: var(--accent);
    text-decoration-thickness: 1.5px;
    text-underline-offset: 3px;
  }

  .bar {
    height: 4px;
    border-radius: 2px;
    background: var(--track);
    overflow: hidden;
  }

  .bar i {
    display: block;
    height: 100%;
    background: var(--c);
  }

  .row b {
    text-align: right;
    font-weight: 600;
  }

  .row:hover .nm,
  .row.hot .nm,
  .row.sel .nm {
    color: #fff;
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 10px 0;
    font-size: 12.5px;
    color: var(--muted);
    text-align: left;
    border-top: 1px solid var(--line);
  }

  .foot:hover {
    color: var(--text);
  }

  .foot svg {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform 0.15s;
  }

  .foot svg.open {
    transform: rotate(90deg);
  }

  /* Rows with no size: dimmer than the rest, no bar, the reason where the size would be. */
  .row.off {
    grid-template-columns: 10px minmax(0, 1fr) auto;
    cursor: default;
  }

  .row.off > * {
    opacity: 0.55;
  }

  .row.off .dot {
    background: var(--dim);
  }

  .why {
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
  }

  .more {
    display: block;
    width: 100%;
    padding: 10px 0;
    font-size: 12.5px;
    color: var(--accent);
    border-top: 1px solid var(--line);
  }

  .more:hover:not(:disabled) {
    color: var(--accent-hover);
  }

  .more:disabled {
    color: var(--dim);
  }

  .note {
    padding: 10px 0;
    font-size: 12.5px;
    color: var(--muted);
  }

  .note .more {
    display: inline;
    width: auto;
    padding: 0 0 0 6px;
    border: 0;
  }
</style>
