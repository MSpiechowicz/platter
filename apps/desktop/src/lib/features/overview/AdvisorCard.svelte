<script lang="ts">
  import { untrack } from 'svelte';
  import type { NodeId, ScanId, SuggestionsDto } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import Checkbox from '../../components/ui/Checkbox.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import Segmented from '../../components/ui/Segmented.svelte';
  import Button from '../../components/ui/Button.svelte';
  import { countUp } from '../../motion/countUp.svelte';
  import type { Action, CleanupStore, SelectionChoice } from '../../state/cleanup.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { groupSuggestions, itemCount, reasonOf, suggestionCount } from './advisor';
  import { footerStatus } from './cleanupText';
  import { visibleText } from './display';
  import RetryNote from './RetryNote.svelte';
  import ReviewDialog from './ReviewDialog.svelte';

  interface Props {
    /** Undefined until the first answer arrives. */
    suggestions: SuggestionsDto | undefined;
    /** The scan the suggestions belong to. */
    scanId: ScanId;
    /** The cleanup flow; the card only shows it and passes the user's clicks on. */
    store: CleanupStore;
    loading: boolean;
    error: unknown;
    onretry: () => void;
    /** A row was clicked: open the Explorer on its largest matched folder. */
    onopen: (nodeId: NodeId) => void;
  }

  let { suggestions, scanId, store, loading, error, onretry, onopen }: Props = $props();

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

<Card title="Advisor" hint="RULES" class="advisor">
  {#if suggestions}
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
              <span class="gsum num">{itemCount(group.suggestions.length)} · {fmt.size(group.allocated)}</span
              >
            </h4>

            {#each group.suggestions as suggestion (suggestion.id)}
              <div class="sug" class:done={store.cleaned.has(suggestion.id)}>
                {#if suggestion.verdict === 'keep'}
                  <span class="slot"></span>
                {:else}
                  <Checkbox
                    checked={store.isSelected(suggestion.id)}
                    disabled={locked || store.cleaned.has(suggestion.id)}
                    aria-label={visibleText(suggestion.title)}
                    onchange={() => store.toggle(suggestion.id)}
                  />
                {/if}

                <button
                  type="button"
                  class="text"
                  disabled={suggestion.nodes.length === 0}
                  title="{visibleText(suggestion.path)}: open in the Explorer"
                  onclick={() => onopen(suggestion.nodes[0] as NodeId)}
                >
                  <span class="t">{visibleText(suggestion.title)}</span>
                  <span class="p">{visibleText(suggestion.path)}</span>
                  <span class="why">{visibleText(reasonOf(suggestion))}</span>
                </button>

                <b class="num">{fmt.size(suggestion.allocated)}</b>
              </div>
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
                  <button type="button" class="act" onclick={() => run(action)}
                    >{ACTION_LABELS[action]}</button
                  >
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
        <Button
          variant="primary"
          disabled={!store.canCleanSelected}
          onclick={() => void store.cleanSelected()}
        >
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
  {:else if error}
    <RetryNote {onretry} />
  {:else}
    <Loading when={loading} label="Looking for things to clean…" />
  {/if}
</Card>

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
  .sug > :global(.chk) {
    margin-top: 3px;
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

  .text {
    display: block;
    min-width: 0;
    padding: 0;
    text-align: left;
    cursor: pointer;
  }

  .text:disabled {
    cursor: default;
  }

  .text > span {
    display: block;
  }

  .t {
    transition: color 0.15s;
  }

  .text:hover:not(:disabled) .t {
    color: #fff;
  }

  .p {
    font-size: 11px;
    color: var(--dim);
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
</style>
