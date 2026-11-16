<script lang="ts">
  import type { NodeId, SuggestionsDto } from '../../api/types';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import { countUp } from '../../motion/countUp.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { groupSuggestions, itemCount, suggestionCount } from './advisor';
  import RetryNote from './RetryNote.svelte';

  interface Props {
    /** Undefined until the first answer arrives. */
    suggestions: SuggestionsDto | undefined;
    loading: boolean;
    error: unknown;
    onretry: () => void;
    /** A row was clicked: open the Explorer on its largest matched folder. */
    onopen: (nodeId: NodeId) => void;
  }

  let { suggestions, loading, error, onretry, onopen }: Props = $props();

  const groups = $derived(suggestions ? groupSuggestions(suggestions) : []);
  const count = $derived(suggestions?.suggestions.length ?? 0);

  const animatedSafe = countUp(() => suggestions?.totals.safe ?? 0);
</script>

<Card title="Advisor" hint="RULES" class="advisor">
  {#if suggestions}
    <div class="tools">{suggestionCount(count)}</div>

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
              <button
                type="button"
                class="sug"
                disabled={suggestion.nodes.length === 0}
                title="{suggestion.path}: open in the Explorer"
                onclick={() => onopen(suggestion.nodes[0] as NodeId)}
              >
                <span class="text">
                  <span class="t">{suggestion.title}</span>
                  <span class="p">{suggestion.path}</span>
                  <span class="why">{suggestion.reason}</span>
                </span>
                <b class="num">{fmt.size(suggestion.allocated)}</b>
              </button>
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
        <div>
          <div class="label">SAFE TO CLEAN</div>
          <b class="num">{fmt.size(animatedSafe.value)}</b>
        </div>
        <p class="note">
          <span>Cleaning arrives in the next update.</span>
          <span>Nothing is changed for now.</span>
        </p>
      </div>
    </div>
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
    min-height: 32px;
    padding-bottom: 10px;
    font-size: 12.5px;
    color: var(--muted);
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

  /* Title, path and reason share one left edge; the size sits on the right. */
  .sug {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 10px;
    align-items: start;
    width: 100%;
    padding: 10px 0;
    text-align: left;
    cursor: pointer;
  }

  .sug:disabled {
    cursor: default;
  }

  .sug + .sug {
    border-top: 1px solid var(--line);
  }

  .text {
    display: block;
    min-width: 0;
  }

  .text > span {
    display: block;
  }

  .t {
    transition: color 0.15s;
  }

  .sug:hover:not(:disabled) .t {
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

  .trow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    min-height: 44px;
  }

  .label {
    font-size: 11px;
    letter-spacing: 0.12em;
    color: var(--muted);
  }

  .total b {
    font-size: 22px;
  }

  .note {
    font-size: 12.5px;
    line-height: 1.4;
    color: var(--muted);
    text-align: right;
    max-width: 210px;
  }

  .note span {
    display: block;
  }
</style>
