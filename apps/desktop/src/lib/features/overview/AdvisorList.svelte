<script module lang="ts">
  import type { NodeId, ScanId, SuggestionDto, SuggestionsDto } from '../../api/types';
  import type { CleanupStore } from '../../state/cleanup.svelte';

  /*
   * The Advisor's list and footer: the rows with their checkboxes, "Select Safe | All | None", the total, the status
   * text and the two Clean buttons, with the review dialog. It renders inside a `Card` with class `advisor`, a flex
   * column whose width is the box `cqw` is measured against. The Overview and the Cleanup view both show it, from
   * the same store. A row is the label of its checkbox (a click on its text ticks it); opening the Explorer is a
   * small button of its own under the size. After a cleanup the sizes are greyed out until a rescan.
   */
  export interface AdvisorListProps {
    /** Undefined until the first answer arrives. */
    suggestions: SuggestionsDto | undefined;
    /** The scan the suggestions belong to. */
    scanId: ScanId;
    /** The cleanup flow; the list only shows it and passes the user's clicks on. */
    store: CleanupStore;
    loading: boolean;
    error: unknown;
    onretry: () => void;
    /** A row's "Open in the Explorer" button was clicked: open the Explorer on its largest matched folder. */
    onopen: (nodeId: NodeId) => void;
  }
</script>

<script lang="ts">
  import { delayed } from '../../state/delayed.svelte';
  import { untrack } from 'svelte';
  import Checkbox from '../../components/ui/Checkbox.svelte';
  import Icon from '../../components/ui/Icon.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import Segmented from '../../components/ui/Segmented.svelte';
  import Button from '../../components/ui/Button.svelte';
  import { countUp } from '../../motion/countUp.svelte';
  import type { Action, SelectionChoice } from '../../state/cleanup.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { groupSuggestions, itemCount, reasonOf, suggestionCount } from './advisor';
  import { footerStatus, STALE_TEXT } from './cleanupText';
  import { visibleText } from './display';
  import RetryNote from './RetryNote.svelte';
  import ReviewDialog from './ReviewDialog.svelte';

  let { suggestions, scanId, store, loading, error, onretry, onopen }: AdvisorListProps = $props();

  // A spinner that appeared stays at least a second, so the content does not replace it after a flash.
  const shown = delayed(() => !suggestions && !error && loading);

  const CHOICES: { value: SelectionChoice; label: string }[] = [
    { value: 'safe', label: 'Safe' },
    { value: 'all', label: 'All' },
    { value: 'none', label: 'None' },
  ];

  const ACTION_LABELS: Record<Action, string> = { cancel: 'Cancel', undo: 'Undo', rescan: 'Rescan' };

  // The suggestions become the store's rows before the first paint, so the safe ones are ticked from the start.
  $effect.pre(() => {
    const id = scanId;
    const data = suggestions;

    untrack(() => store.load(id, data));
  });

  const groups = $derived(suggestions ? groupSuggestions(suggestions) : []);
  const count = $derived(suggestions?.suggestions.length ?? 0);
  const status = $derived(footerStatus(store));
  const locked = $derived(store.busy || store.outdated);

  const animatedSelected = countUp(() => store.selectedBytes, 300);

  // A disabled button gets no click from a browser; this is the same rule for any other way a click can arrive.
  function open(suggestion: SuggestionDto): void {
    if (!locked && suggestion.nodes.length > 0) {
      onopen(suggestion.nodes[0] as NodeId);
    }
  }

  function run(action: Action): void {
    if (action === 'cancel') {
      void store.cancel();
    } else if (action === 'undo') {
      void store.undo();
    } else {
      void store.rescan();
    }
  }
</script>

{#if suggestions && !shown.value}
  <div class="tools">
    <span>
      {store.cleanable.length > 0
        ? `${store.selectedItems.length} of ${store.cleanable.length} selected`
        : suggestionCount(count)}
    </span>
    <span class="sp"></span>
    <span>Select</span>
    <Segmented
      size="sm"
      label="Select"
      options={CHOICES}
      value={store.choice}
      disabled={locked || store.cleanable.length === 0}
      onchange={(choice) => store.select(choice as 'safe' | 'all' | 'none')}
    />
  </div>

  {#if count > 0}
    <div class="list">
      {#each groups as group (group.verdict)}
        <section class="group" aria-label={group.title}>
          <h4 class="gh {group.verdict}">
            <span>{group.title}</span>
            <span
              class="gsum num"
              class:stale={store.outdated}
              title={store.outdated ? STALE_TEXT : undefined}
            >
              {itemCount(group.suggestions.length)} · {fmt.size(group.allocated)}
            </span>
          </h4>

          {#each group.suggestions as suggestion (suggestion.id)}
            {@const kept = suggestion.verdict === 'keep'}
            {@const done = store.cleaned.has(suggestion.id)}
            {@const title = visibleText(suggestion.title)}

            <!-- A kept row has no checkbox: it is no label, so a click on its text does nothing. -->
            <svelte:element
              this={kept ? 'div' : 'label'}
              class="sug"
              class:done
              class:pick={!kept && !locked && !done}
            >
              {#if kept}
                <span class="slot"></span>
              {:else}
                <Checkbox
                  checked={store.isSelected(suggestion.id)}
                  disabled={locked || done}
                  aria-label={title}
                  onchange={() => store.toggle(suggestion.id)}
                />
              {/if}

              <span class="text">
                <span class="t">{title}</span>
                <!-- The path is the button that opens it in the Explorer: a folder glyph and the path, on one line. -->
                <button
                  type="button"
                  class="pline"
                  disabled={locked || suggestion.nodes.length === 0}
                  aria-label="Open {title} in the Explorer"
                  title="{visibleText(suggestion.path)}: open in the Explorer"
                  onclick={() => open(suggestion)}
                >
                  <Icon name="explorer" size={12} />
                  <span class="p">{visibleText(suggestion.path)}</span>
                </button>
                <span class="why">{visibleText(reasonOf(suggestion))}</span>
              </span>

              <span class="end">
                <b
                  class="num"
                  class:stale={store.outdated && !done}
                  title={store.outdated && !done ? STALE_TEXT : undefined}>{fmt.size(suggestion.allocated)}</b
                >
              </span>
            </svelte:element>
          {/each}
        </section>
      {/each}
    </div>
  {:else}
    <p class="empty">
      Nothing to suggest here: none of the known caches, build folders or leftovers were found.
    </p>
  {/if}

  <div class="total">
    <div class="trow">
      <div class="sel">
        <div class="label">SELECTED</div>
        <b class="num">{fmt.size(animatedSelected.value)}</b>
      </div>

      <div class="status" class:attention={status.attention} role="status">
        <div class="sbody">
          <p class="msg">{status.text}</p>

          {#if status.actions.length > 0}
            <p class="acts">
              {#each status.actions as action (action)}
                <button type="button" class="act" onclick={() => run(action)}>{ACTION_LABELS[action]}</button>
              {/each}
            </p>
          {/if}

          {#each status.details as line, index (index)}
            <p class="detail">{line}</p>
          {/each}
        </div>
      </div>
    </div>

    <div class="tbtns">
      <Button disabled={!store.canCleanSafe} onclick={() => void store.cleanSafe()}>
        {store.safeItems.length > 0 ? `Clean safe · ${fmt.size(store.safeBytes)}` : 'Clean safe'}
      </Button>
      <Button variant="primary" disabled={!store.canCleanSelected} onclick={() => void store.cleanSelected()}>
        Clean selected ({store.selectedItems.length})
      </Button>
    </div>
  </div>

  <ReviewDialog
    plan={store.reviewing}
    suggestions={store.suggestions}
    onconfirm={() => void store.confirmReview()}
    oncancel={() => store.dismissReview()}
  />
{:else if error && !shown.value}
  <RetryNote {onretry} />
{:else}
  <div class="wait"><Loading when={loading || shown.value} label="Looking for things to clean…" /></div>
{/if}

<style>
  /* The card is the box `cqw` is measured against. */
  :global(.advisor) {
    container-type: inline-size;
  }

  .tools {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
    padding-bottom: 10px;
    font-size: 12.5px;
    color: var(--muted);
  }

  .sp {
    flex: 1;
  }

  .list {
    flex: 1;
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
    /* The scroll bar sits in the card's padding; its width, or its absence, never changes the rows (see below). */
    margin-right: -16px;
  }

  /* Exactly as wide as the card's content, whatever the engine does with scroll bars. */
  .list > .group {
    width: calc(100% - 16px);
    width: 100cqw;
  }

  /* A group header scrolls with its rows; it does not stick to the top of the list. */
  .gh {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 9px 0;
    font-size: 11px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    font-weight: 600;
  }

  /* In a single column the Advisor is a card among the others: its list gets a height of its own. */
  @media (max-width: 1000px) {
    .list {
      flex: none;
      max-height: 520px;
    }
  }

  .group + .group .gh {
    border-top: 1px solid var(--line);
  }

  .gsum {
    letter-spacing: 0;
    text-transform: none;
    font-weight: 400;
    color: var(--muted);
    font-size: 12px;
  }

  .safe {
    color: var(--green);
  }

  .review {
    color: var(--amber);
  }

  .keep {
    color: var(--red);
  }

  /* The checkbox has its own column; title, path and reason share one left edge, and the size sits on the right. */
  .sug {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto;
    gap: 10px;
    align-items: start;
    padding: 10px 0;
  }

  /* The box lines up with the first line of the title. */
  /* Centred on the title's first line, whatever line height the engine gives it (fallback: the measured 3px). */
  .sug > :global(.chk-wrap) {
    margin-top: 3px;
    margin-top: calc((1lh - 14px) / 2);
  }

  /* A kept row has no checkbox; its column stays so that every row starts at the same edge. */
  .slot {
    display: block;
  }

  .sug + .sug {
    border-top: 1px solid var(--line);
  }

  .sug.done {
    opacity: 0.35;
  }

  .sug.done .t {
    text-decoration: line-through;
  }

  /* Only a row that can be ticked answers to a click on its text. */
  .sug.pick {
    cursor: pointer;
  }

  .sug.pick:hover .t {
    color: var(--strong);
  }

  .text {
    display: block;
    min-width: 0;
  }

  .text > span {
    display: block;
  }

  /* Fixed line heights, so the checkbox, the size and the Explorer button line up in every engine. */
  .t {
    line-height: 20px;
    transition: color 0.15s;
  }

  /*
   * The size, at the row's right edge, on the title's line.
   */
  .end {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
  }

  /* The size on the title's line, the button centred on the path's line. */
  .end > :global(.num) {
    line-height: 20px;
  }

  /*
   * The path line opens the path in the Explorer. Glyph and text share one line box and are centred in it; the glyph
   * starts on the row's left edge (the padding is taken back by the margin), and the highlight reaches past both.
   */
  .text > .pline {
    display: flex;
    align-items: center;
    gap: 5px;
    width: fit-content;
    max-width: calc(100% + 6px);
    min-width: 0;
    height: 18px;
    margin: 0 0 1px -6px;
    padding: 0 6px;
    border-radius: 4px;
    color: var(--dim);
    transition:
      color 0.15s,
      background 0.15s;
  }

  .pline :global(svg) {
    display: block;
    flex: none;
    color: var(--muted);
    transition: color 0.15s;
  }

  .pline:hover:not(:disabled) :global(svg),
  .pline:focus-visible :global(svg) {
    color: var(--accent);
  }

  .pline:hover:not(:disabled),
  .pline:focus-visible {
    color: var(--accent);
    background: var(--wash-4);
  }

  .pline:disabled {
    cursor: default;
  }

  .p {
    font-size: 11px;
    line-height: 16px;
    color: inherit;
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .why {
    font-size: 12px;
    color: var(--muted);
    margin-top: 3px;
  }

  .empty {
    flex: 1;
    display: grid;
    place-content: center;
    padding: 0 12px;
    text-align: center;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--muted);
  }

  .total {
    margin-top: 12px;
    padding-top: 14px;
    border-top: 1px solid var(--line);
  }

  /* A fixed height: the status text can change as it likes, nothing around it moves. */
  .trow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    height: 60px;
  }

  .sel {
    flex: none;
  }

  .label {
    font-size: 11px;
    letter-spacing: 0.12em;
    color: var(--muted);
  }

  .total b {
    font-size: 22px;
  }

  /* The text is centred against the total; a long list of details scrolls inside the box instead of growing it. */
  .status {
    flex: 1;
    align-self: stretch;
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow-y: auto;
    font-size: 12.5px;
    line-height: 1.4;
    color: var(--muted);
    text-align: right;
  }

  .status.attention {
    color: var(--amber);
  }

  /* Auto margins centre the text while it fits, and give way to scrolling when it does not. */
  .sbody {
    margin-block: auto;
  }

  .detail {
    margin-top: 3px;
    font-size: 11.5px;
    color: var(--dim);
    overflow-wrap: anywhere;
  }

  .acts {
    display: flex;
    justify-content: flex-end;
    gap: 14px;
  }

  .act {
    padding: 0;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--accent);
  }

  .act:hover {
    color: var(--accent-hover);
    text-decoration: underline;
  }

  .tbtns {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    margin-top: 14px;
  }

  .tbtns :global(.btn) {
    padding: 9px 8px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* The spinner fills its positioned parent: this box keeps it inside the card while the card holds nothing else. */
  .wait {
    position: relative;
    min-height: 200px;
  }
</style>
