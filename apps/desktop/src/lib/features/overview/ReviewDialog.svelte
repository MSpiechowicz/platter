<script lang="ts">
  import type { CleanupPlanDto, PlanGroupDto, SuggestionDto } from '../../api/types';
  import Button from '../../components/ui/Button.svelte';
  import Dialog from '../../components/ui/Dialog.svelte';
  import { MARKED_RULE } from '../../state/cleanup.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { itemCount, reasonOf } from './advisor';
  import { actionText, planNotes } from './cleanupText';
  import { shownPath, visibleText } from './display';

  interface Props {
    /** The plan to review; the dialog is open while there is one. */
    plan: CleanupPlanDto | undefined;
    /** The suggestions of the scan, for the reason of each group. */
    suggestions: SuggestionDto[];
    /** "Clean <total>". The backend asks for its own confirmation after this. */
    onconfirm: () => void;
    /** Cancel, Escape or a click outside. */
    oncancel: () => void;
  }

  let { plan, suggestions, onconfirm, oncancel }: Props = $props();

  const review = $derived(plan?.groups.filter((group) => group.verdict === 'review') ?? []);
  const safe = $derived(plan?.groups.filter((group) => group.verdict === 'safe') ?? []);
  const safeBytes = $derived(safe.reduce((sum, group) => sum + group.allocated, 0));
  const notes = $derived(planNotes(plan, (id) => suggestions.find((item) => item.id === id)?.title ?? id));

  // A group of rule matches is one item of the question; what the user marked is as many items as were marked.
  const reviewItems = $derived(
    review.reduce((total, group) => total + (group.ruleId === MARKED_RULE ? group.items : 1), 0),
  );

  const reasonOfGroup = (group: PlanGroupDto): string | undefined => {
    if (group.ruleId === MARKED_RULE) {
      return 'You marked these yourself, so no rule vouches for them.';
    }

    const suggestion = suggestions.find((item) => item.id === group.ruleId);

    return suggestion ? reasonOf(suggestion) : undefined;
  };

  /** The path of a single match, or the number of matches with one as an example. */
  const where = (group: PlanGroupDto): string => {
    const first = group.samplePaths[0];

    if (first === undefined) {
      return group.command === undefined ? '' : visibleText(group.command);
    }

    return group.items === 1
      ? shownPath(first, 48)
      : `${itemCount(group.items)}, e.g. ${shownPath(first, 48)}`;
  };
</script>

<Dialog
  open={plan !== undefined}
  title="Clean {reviewItems} {reviewItems === 1 ? 'item that needs' : 'items that need'} review?"
  description="These are not certain to be safe to remove. Check each one before you continue."
  onclose={oncancel}
>
  <div class="rows">
    {#each review as group (group.ruleId)}
      <div class="di">
        <span class="what">
          {visibleText(group.title)}
          {#if group.ruleId === MARKED_RULE}
            {#each group.samplePaths as path, index (index)}
              <span class="mono path">{shownPath(path, 48)}</span>
            {/each}
            {#if group.items > group.samplePaths.length}
              <span class="mono path">… and {group.items - group.samplePaths.length} more</span>
            {/if}
          {:else}
            <span class="mono" title={group.samplePaths.map(visibleText).join('\n')}>{where(group)}</span>
          {/if}
        </span>
        <b class="num">{fmt.size(group.allocated)}</b>
        <small>{reasonOfGroup(group) ?? itemCount(group.items)}</small>
        <small>{actionText(group)}</small>
      </div>
    {/each}
  </div>

  <div class="dsum">
    <span>
      {#if safe.length > 0}
        Plus {safe.length} safe {safe.length === 1 ? 'item' : 'items'} · {fmt.size(safeBytes)}
      {:else}
        No safe items selected
      {/if}
    </span>
    <span>Total <b class="num">{fmt.size(plan?.allocated ?? 0)}</b></span>
  </div>

  {#if notes.length > 0}
    <div class="notes">
      {#each notes as line, index (index)}
        <p>{line}</p>
      {/each}
    </div>
  {/if}

  {#snippet actions()}
    <Button data-autofocus onclick={oncancel}>Cancel</Button>
    <Button variant="primary" onclick={onconfirm}>Clean {fmt.size(plan?.allocated ?? 0)}</Button>
  {/snippet}
</Dialog>

<style>
  /* Many review items scroll inside the dialog; the total and the buttons stay in view. */
  .rows {
    max-height: max(160px, calc(100vh - 380px));
    overflow-y: auto;
  }

  .di {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 2px 12px;
    padding: 10px 0;
    border-top: 1px solid var(--line);
    font-size: 13px;
  }

  .what {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .di .mono {
    font-size: 11px;
    color: var(--dim);
  }

  /* The items the user marked are listed one under the other. */
  .di .path {
    display: block;
  }

  .di small {
    grid-column: 1 / -1;
    color: var(--muted);
    font-size: 12px;
  }

  .dsum {
    display: flex;
    justify-content: space-between;
    padding: 12px 0 0;
    border-top: 1px solid var(--line);
    font-size: 13px;
    color: var(--muted);
  }

  .dsum b {
    color: var(--text);
    font-size: 15px;
  }

  .notes {
    margin-top: 12px;
    font-size: 12px;
    color: var(--muted);
    overflow-wrap: anywhere;
  }

  .notes p + p {
    margin-top: 3px;
  }
</style>
