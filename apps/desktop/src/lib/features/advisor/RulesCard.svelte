<script lang="ts">
  import { tick } from 'svelte';
  import type { FolderMatchDto, ScanId } from '../../api/types';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Notice from '../../components/ui/Notice.svelte';
  import Tag from '../../components/ui/Tag.svelte';
  import type { AdvisorStore } from '../../state/advisor.svelte';
  import type { RulesStore } from '../../state/rules.svelte';
  import { visibleText } from '../overview/display';
  import FolderPicker from './FolderPicker.svelte';
  import { noScanText } from './model';

  interface Props {
    /** The finished scan a rule's folder is picked from. Undefined without one: the list shows, nothing can be added. */
    scan: ScanId | undefined;
    /** A scan is running, so there is no finished one yet. */
    scanning?: boolean;
    store: AdvisorStore;
    rules: RulesStore;
  }

  let { scan, scanning = false, store, rules }: Props = $props();

  let picked = $state<FolderMatchDto | null>(null);
  /** Bumped after an add, so the picker starts empty again. */
  let pickerKey = $state(0);
  let busy = $state(false);
  let addError = $state<string | undefined>(undefined);
  let removeError = $state<string | undefined>(undefined);
  let resetError = $state<string | undefined>(undefined);
  let list: HTMLDivElement | undefined = $state();

  // A folder of another scan means nothing in this one.
  $effect(() => {
    void scan;
    picked = null;
    addError = undefined;
  });

  async function add(event?: SubmitEvent): Promise<void> {
    event?.preventDefault();

    if (scan === undefined || !picked || busy) {
      return;
    }

    busy = true;
    addError = await rules.add(scan, picked.node, 'keep');
    busy = false;

    if (addError === undefined) {
      picked = null;
      pickerKey++;

      // The new rule is the last one: it is in view.
      await tick();

      if (list) {
        list.scrollTop = list.scrollHeight;
      }
    }
  }

  async function remove(id: number): Promise<void> {
    if (busy) {
      return;
    }

    busy = true;
    removeError = await rules.remove(id);
    busy = false;
  }

  async function startOver(): Promise<void> {
    if (busy) {
      return;
    }

    busy = true;
    resetError = await rules.reset();
    busy = false;
  }
</script>

<Card title="What to keep" class="rules">
  {#if rules.problem !== undefined}
    <div class="broken">
      <Notice kind="error">{visibleText(rules.problem)}</Notice>
      <p class="muted small">
        Without your rules Platter cannot tell which folders you keep, so it makes no cleanup plan. Fix the
        file, or start over: the file is moved aside and the list starts empty.
      </p>
      <div class="actions">
        <Button disabled={busy} onclick={startOver}>Start over</Button>
      </div>
      {#if resetError}
        <p class="muted small" role="alert">{visibleText(resetError)}</p>
      {/if}
    </div>
  {:else}
    <div class="list" bind:this={list}>
      {#each rules.rules as rule (rule.id)}
        <div class="rule">
          <span class="path" title={visibleText(rule.path)}>{visibleText(rule.path)}</span>
          <Tag kind={rule.verdict}>{rule.verdict}</Tag>
          <button
            type="button"
            class="x"
            title="Remove rule"
            aria-label="Remove the rule for {visibleText(rule.path)}"
            disabled={busy}
            onclick={() => remove(rule.id)}>✕</button
          >
        </div>
      {:else}
        <p class="empty">{rules.loaded ? 'No rules yet.' : 'Reading your rules…'}</p>
        {#if rules.loaded}
          <p class="empty hint">
            A folder you add below is always kept: Platter never cleans it or any file or folder inside it.
          </p>
        {/if}
      {/each}
    </div>

    {#if removeError}
      <p class="muted small note" role="alert">{visibleText(removeError)}</p>
    {/if}

    {#if rules.loadFailure}
      <p class="muted small note" role="alert">{visibleText(rules.loadFailure)}</p>
    {/if}

    {#if scan === undefined}
      <p class="muted small note">{noScanText(scanning)}</p>
    {/if}

    <form class="inp" onsubmit={add}>
      <!-- A search of the previous scan must not deliver its folders here: another scan starts a new picker. -->
      {#key `${scan}:${pickerKey}`}
        <FolderPicker
          label="Folder to always keep"
          placeholder="Type a folder to always keep"
          disabled={scan === undefined}
          find={(query) => (scan === undefined ? Promise.resolve([]) : store.find(scan, query))}
          bind:picked
          onenter={add}
        />
      {/key}
      <Button type="submit" variant="primary" disabled={scan === undefined || !picked || busy}>Add</Button>
    </form>

    {#if addError}
      <p class="muted small note" role="alert">{visibleText(addError)}</p>
    {/if}
  {/if}
</Card>

<style>
  .empty {
    color: var(--muted);
    font-size: 12.5px;
  }

  .empty.hint {
    margin-top: 6px;
    line-height: 1.5;
    color: var(--dim);
  }

  .small {
    font-size: 12px;
    line-height: 1.45;
  }

  .broken {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: flex-start;
    align-self: stretch;
  }

  .actions {
    display: flex;
    gap: 8px;
  }

  /* The list takes what the column leaves and scrolls inside it; the scroll bar sits in the card's padding. */
  .list {
    flex: 1;
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
    margin-right: -16px;
  }

  /* Exactly as wide as the card's content, whatever the engine does with the scroll bar. */
  .list > :global(*) {
    width: calc(100% - 16px);
    width: 100cqw;
  }

  :global(.rules) {
    container-type: inline-size;
  }

  @media (max-width: 1300px) {
    .list {
      flex: none;
      max-height: 420px;
    }
  }

  .rule {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 20px;
    gap: 10px;
    align-items: center;
    padding: 9px 0;
    border-top: 1px solid var(--line);
    font-size: 12.5px;
  }

  .rule:first-child {
    border-top: 0;
    padding-top: 0;
  }

  .rule:last-child {
    padding-bottom: 0;
  }

  .path {
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .x {
    color: var(--dim);
    font-size: 12px;
    line-height: 20px;
    height: 20px;
    text-align: center;
  }

  .x:hover:not(:disabled) {
    color: var(--red);
  }

  .x:disabled {
    opacity: 0.4;
  }

  .note {
    margin-top: 10px;
  }

  .inp {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }
</style>
