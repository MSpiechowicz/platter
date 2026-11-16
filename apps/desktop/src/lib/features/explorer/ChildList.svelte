<script lang="ts">
  import { tick, type Snippet } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import type { NodeDto, NodeId, SortKey } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import Checkbox from '../../components/ui/Checkbox.svelte';
  import Dropdown from '../../components/ui/Dropdown.svelte';
  import { formatCount } from '../../format';
  import { fmt } from '../../state/fmt.svelte';
  import { visibleText } from '../overview/display';
  import { FULL_TEXT, itemOff, KEPT_TEXT } from './marking';
  import { moveFocus } from './roving';
  import { countLabels, normalizeQuery, segments } from './search';

  interface Props {
    /** The folder shown; the not-counted rows fold up again when it changes. */
    folder: NodeId;
    /** The rows that have a size: with a query, the ones the backend found. */
    items: readonly NodeDto[];
    /** How many items of the folder have a size (the most it can be while pages are still on their way). */
    total: number;
    /** With a query: how many of them match, once that is known. */
    matched?: number;
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
    /** The rows of the selected "smaller items" tile are not all loaded, and the list stopped loading by itself. */
    groupTruncated?: boolean;
    /** How many items the list has in all, counted or not (with a query, the matches): "Load more" is out of this. */
    allTotal?: number;
    /** How many of them are loaded, counted or not. */
    loaded?: number;
    /** The items the user marked for cleanup. */
    marked?: ReadonlySet<NodeId>;
    /** A keep suggestion covers the item: it shows a lock and "KEPT" instead of a checkbox. */
    isKept?: (id: NodeId) => boolean;
    /** Why no item can be marked or unmarked now (a cleanup runs, the scan is outdated, ...); `undefined` when they can. */
    markOff?: string;
    /** The most items are marked: the ones that are not marked cannot be ticked. */
    markFull?: boolean;
    /** A checkbox was ticked or cleared. Without it the rows have no checkboxes. */
    onmark?: (node: NodeDto, on: boolean) => void;
    /** What goes under the rows, inside the card (the marked total and "Clean"). */
    bottom?: Snippet;
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
    matched,
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
    groupTruncated = false,
    allTotal = 0,
    loaded = 0,
    marked = new SvelteSet(),
    isKept = () => false,
    markOff,
    markFull = false,
    onmark,
    bottom,
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

  // With a query the rows are the ones the backend found; the query is only what is underlined in their names.
  const needle = $derived(normalizeQuery(query));
  const filtering = $derived(needle !== '');
  // The rows loaded without a size are never hidden: the count is at least their number. It is the whole number when
  // the list is known to the end (or its total is), and otherwise only what has been seen so far.
  const uncountedCount = $derived(Math.max(uncountedTotal ?? 0, uncounted.length));
  const uncountedKnown = $derived(!hasMore || uncountedTotal !== undefined);

  const largest = $derived(items.reduce((most, item) => Math.max(most, item.allocated), 0) || 1);
  const footer = $derived.by(() => {
    const phrase = uncountedCount === 1 ? '1 item' : `${formatCount(uncountedCount)} items`;

    return `${phrase} not counted${uncountedKnown ? '' : ' so far'}`;
  });
  const nothingFound = $derived(
    filtering && items.length === 0 && uncounted.length === 0 && !loading && !error,
  );

  // The count: the longest text that fits next to the title and the sort. Every text is measured in a hidden copy.
  const labels = $derived(countLabels(total, filtering ? matched : undefined));
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

  // The search field: what is typed goes to the store as it is typed; the store waits for a pause before it asks.
  let field: HTMLInputElement | undefined = $state();
  let draft = $state('');

  function type(): void {
    onquery?.(draft);
  }

  function clear(): void {
    draft = '';
    onquery?.('');
  }

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
  const tabbable = $derived(items.find((item) => item.id === selected)?.id ?? items[0]?.id);

  function onkeydown(event: KeyboardEvent): void {
    moveFocus(event, [...(list?.querySelectorAll<HTMLElement>('.row') ?? [])]);
  }

  // Only one row's checkbox is in the tab order too: the one of the row that was focused last.
  let focusedRow = $state<NodeId | null>(null);

  const boxTab = (id: NodeId): number => (id === (focusedRow ?? tabbable) ? 0 : -1);

  /** Why an item's checkbox is off, or `undefined`. A ticked box is always off while marking is, but never refused for being full. */
  function boxOff(item: NodeDto): string | undefined {
    if (markOff !== undefined) {
      return markOff;
    }

    const never = itemOff(item);

    if (never !== undefined) {
      return never;
    }

    return markFull && !marked.has(item.id) ? FULL_TEXT : undefined;
  }

  async function tickBox(event: Event, item: NodeDto): Promise<void> {
    const box = event.currentTarget as HTMLInputElement;

    onmark?.(item, box.checked);

    // The store may have said no: the box shows what is marked, not what was clicked.
    await tick();
    box.checked = marked.has(item.id);
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
      maxlength="255"
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

  <div class="progress" class:on={filtering && loading} aria-hidden={!(filtering && loading)}><i></i></div>

  <div class="rows" bind:this={list} aria-busy={loading}>
    <div class="inner">
      {#if error}
        <p class="note">
          The list could not be read: {error}
          <button type="button" class="more" onclick={onretry}>Try again</button>
        </p>
      {:else if !filtering && items.length === 0 && uncounted.length === 0 && !hasMore && !loading}
        <p class="note">Nothing in this folder.</p>
      {/if}

      {#each items as item (item.id)}
        {@const kept = isKept(item.id)}
        {@const off = boxOff(item)}
        <div
          class="item"
          class:sel={item.id === selected || highlighted.has(item.id)}
          class:hot={item.id === hovered}
          data-item={item.id}
          onfocusin={() => (focusedRow = item.id)}
        >
          {#if kept}
            <span class="lock" role="img" aria-label={KEPT_TEXT} title={KEPT_TEXT}>
              <svg viewBox="0 0 16 16" aria-hidden="true"
                ><rect x="3.5" y="7" width="9" height="6.5" rx="1.5" /><path
                  d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"
                /></svg
              >
            </span>
          {:else if onmark}
            <span class="box" title={off}>
              <Checkbox
                checked={marked.has(item.id)}
                disabled={off !== undefined}
                tabindex={boxTab(item.id)}
                aria-label="Mark {visibleText(item.name)} for cleanup"
                onchange={(event) => void tickBox(event, item)}
              />
            </span>
          {:else}
            <span class="box"></span>
          {/if}

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
            onmouseenter={() => onhover(item.id)}
            onmouseleave={() => onhover(null)}
            {onkeydown}
          >
            <span class="name">
              <span class="nm" title={visibleText(item.name)}
                >{#each segments(visibleText(item.name), needle) as part, index (index)}{#if part.hit}<mark
                      >{part.text}</mark
                    >{:else}{part.text}{/if}{/each}</span
              >
              {#if kept}<span class="kept">Kept</span>{/if}
            </span>
            <span class="bar" aria-hidden="true"
              ><i style="width:{(item.allocated / largest) * 100}%"></i></span
            >
            <b class="num">{fmt.size(item.allocated)}</b>
          </button>
        </div>
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
            {#each uncounted as item (item.id)}
              <div class="row off" data-uncounted={item.id}>
                <i class="dot"></i>
                <span class="nm" title={visibleText(item.name)}
                  >{#each segments(visibleText(item.name), needle) as part, index (index)}{#if part.hit}<mark
                        >{part.text}</mark
                      >{:else}{part.text}{/if}{/each}</span
                >
                <span class="why">{reason(item)}</span>
              </div>
            {/each}
          </div>
        {/if}
      {/if}

      {#if hasMore}
        <button type="button" class="more" disabled={loadingMore || moreDisabled} onclick={onmore}>
          {loadingMore ? 'Loading…' : `Load more (${formatCount(Math.max(0, allTotal - loaded))} left)`}
        </button>
      {/if}
      {#if groupTruncated}
        <p class="note">
          Only the first {formatCount(loaded)} of {formatCount(allTotal)} items are shown, so some of the smaller
          items are not highlighted
        </p>
      {/if}
      {#if moreError}<p class="note">{moreError}</p>{/if}
    </div>
  </div>

  {@render bottom?.()}
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

  /* A thin line under the field while the matches are on their way; it keeps its place when it is not shown. */
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
    width: 100%;
    height: 100%;
    background: var(--accent);
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

  /*
   * One item: a column for the checkbox (or the lock), then the button that selects it. The checkbox is not inside the
   * button (a button holds no other control), so the highlight is drawn by the item and reaches both.
   */
  .item {
    position: relative;
    isolation: isolate;
    display: grid;
    grid-template-columns: 16px minmax(0, 1fr);
    gap: 10px;
    align-items: center;
    font-size: 13px;
  }

  .item + .item {
    border-top: 1px solid var(--line);
  }

  /* The highlight reaches 8 px past the content on both sides; the content does not move. */
  .item::before {
    content: '';
    position: absolute;
    inset: 0 -8px;
    z-index: -1;
    border-radius: 6px;
  }

  .item:hover::before,
  .item.hot::before,
  .item.sel::before {
    background: #ffffff0a;
  }

  .item:has(.row:focus-visible)::before {
    background: #ffffff0a;
    box-shadow: 0 0 0 1.5px var(--accent);
  }

  .box,
  .lock {
    display: grid;
    align-items: center;
    height: 14px;
  }

  /* The box is 14 px in a 16 px column; its left edge is the column's. */
  .box :global(.chk-wrap) {
    margin-left: 0;
  }

  .lock {
    color: var(--dim);
  }

  /* The lock is drawn inside its square: it is moved out so that its stroke, not its square, is on the left edge. */
  .lock svg {
    width: 14px;
    height: 14px;
    margin-left: -2px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
  }

  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 64px 56px;
    gap: 10px;
    align-items: center;
    min-width: 0;
    padding: 10px 0;
    font-size: 13px;
    text-align: left;
  }

  .row.off + .row.off {
    border-top: 1px solid var(--line);
  }

  .row:focus-visible {
    outline: none;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--c);
  }

  /* The name is cut at its end; "KEPT" after it always stays. */
  .name {
    display: flex;
    align-items: baseline;
    min-width: 0;
  }

  .nm {
    min-width: 0;
    font-family: var(--mono);
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kept {
    flex: none;
    margin-left: 8px;
    font-size: 10.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--dim);
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

  .item:hover .nm,
  .item.hot .nm,
  .item.sel .nm {
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
    position: relative;
    display: grid;
    grid-template-columns: 16px minmax(0, 1fr) auto;
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
