<script module lang="ts">
  import type { FolderMatchDto } from '../../api/types';

  /** The longest text the picker takes (the backend's limit is in bytes and is checked there). */
  export const MAX_QUERY_CHARS = 255;
  /** The pause after the last key before the backend is asked. */
  export const SEARCH_DELAY_MS = 150;

  export interface FolderPickerProps {
    /** Looks folders up in the finished scan; rejects as the API does. */
    find: (query: string) => Promise<FolderMatchDto[]>;
    /** The folder picked, or null. Typing again clears it. */
    picked?: FolderMatchDto | null;
    label: string;
    placeholder: string;
    disabled?: boolean;
    /** Enter pressed with a folder picked and no list open. */
    onenter?: () => void;
  }
</script>

<script lang="ts">
  import { describeError } from '../../api/client';
  import { fmt } from '../../state/fmt.svelte';
  import { visibleText } from '../overview/display';

  let {
    find,
    picked = $bindable(null),
    label,
    placeholder,
    disabled = false,
    onenter,
  }: FolderPickerProps = $props();

  const id = $props.id();
  const listId = `${id}-list`;

  let text = $state('');
  let results = $state<FolderMatchDto[]>([]);
  /** The search of the text came back (an empty list then means "no folder matches"). */
  let searched = $state(false);
  let error = $state<string | undefined>(undefined);
  let open = $state(false);
  let active = $state(-1);
  let root: HTMLDivElement | undefined = $state();

  let timer: ReturnType<typeof setTimeout> | undefined;
  /** Bumped by every key and every pick, so an answer that arrives late is ignored. */
  let sequence = 0;

  // A picker that goes away takes its waiting search with it.
  $effect(() => () => {
    clearTimeout(timer);
    sequence++;
  });

  function reset(): void {
    clearTimeout(timer);
    sequence++;
    results = [];
    searched = false;
    error = undefined;
    open = false;
    active = -1;
  }

  function typed(): void {
    picked = null;
    clearTimeout(timer);
    sequence++;

    if (text.trim() === '') {
      reset();

      return;
    }

    const mine = sequence;

    timer = setTimeout(() => void search(mine), SEARCH_DELAY_MS);
  }

  async function search(mine: number): Promise<void> {
    try {
      const found = await find(text);

      if (mine !== sequence) {
        return;
      }

      results = found;
      error = undefined;
      active = found.length > 0 ? 0 : -1;
    } catch (failure) {
      if (mine !== sequence) {
        return;
      }

      results = [];
      error = describeError(failure);
      active = -1;
    }

    searched = true;
    open = true;
  }

  function pick(match: FolderMatchDto): void {
    reset();
    picked = match;
    text = visibleText(match.path);
  }

  function move(step: number): void {
    if (results.length === 0) {
      return;
    }

    open = true;
    active = Math.max(0, Math.min(results.length - 1, active + step));
  }

  function onKey(event: KeyboardEvent): void {
    if (event.key === 'ArrowDown') {
      move(1);
    } else if (event.key === 'ArrowUp') {
      move(-1);
    } else if (event.key === 'Enter') {
      const match = open ? results[active] : undefined;

      if (match) {
        pick(match);
      } else if (picked) {
        onenter?.();
      }
    } else if (event.key === 'Escape' && open) {
      open = false;
      event.stopPropagation();
    } else {
      return;
    }

    event.preventDefault();
  }

  function onFocusOut(event: FocusEvent): void {
    if (!root?.contains(event.relatedTarget as Node | null)) {
      open = false;
    }
  }

  const showList = $derived(open && (results.length > 0 || searched));
</script>

<div class="picker" bind:this={root} onfocusout={onFocusOut}>
  <div class="field" class:disabled>
    <input
      type="text"
      role="combobox"
      aria-label={label}
      aria-autocomplete="list"
      aria-expanded={showList}
      aria-controls={listId}
      aria-activedescendant={showList && active >= 0 ? `${id}-o${active}` : undefined}
      {placeholder}
      {disabled}
      maxlength={MAX_QUERY_CHARS}
      autocomplete="off"
      spellcheck="false"
      bind:value={text}
      oninput={typed}
      onkeydown={onKey}
      onfocus={() => {
        if (results.length > 0 || error) {
          open = true;
        }
      }}
    />
  </div>

  {#if showList}
    <!-- Above the field: it sits at the bottom of its card. A mouse press must not take the focus from the field. -->
    <div
      class="list"
      id={listId}
      role="listbox"
      aria-label="Folders"
      tabindex="-1"
      onmousedown={(event) => event.preventDefault()}
    >
      {#each results as match, index (match.node)}
        <div
          id="{id}-o{index}"
          class="opt"
          class:on={index === active}
          role="option"
          tabindex="-1"
          aria-selected={index === active}
          onclick={() => pick(match)}
          onkeydown={(event) => event.key === 'Enter' && pick(match)}
          onmouseenter={() => (active = index)}
        >
          <span class="p" title={visibleText(match.path)}>{visibleText(match.path)}</span>
          <span class="s num">{fmt.size(match.allocated)}</span>
        </div>
      {/each}

      {#if error}
        <p class="note" role="alert">{error}</p>
      {:else if results.length === 0}
        <p class="note" role="status">No folder matches.</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* A column, so the field fills the height the form row gives the picker: as tall as the button beside it. */
  .picker {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .field {
    display: flex;
    flex: 1;
    background: var(--card2);
    border: 1px solid var(--line);
    border-radius: 7px;
    transition: border-color 0.15s;
  }

  .field:hover:not(.disabled),
  .field:focus-within {
    border-color: var(--accent);
  }

  .field.disabled {
    opacity: 0.4;
  }

  input {
    flex: 1;
    min-width: 0;
    padding: 7px 10px;
    font: inherit;
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 18px;
    color: var(--text);
    background: none;
    border: 0;
    outline: none;
  }

  input::placeholder {
    color: var(--dim);
  }

  .list {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 6px);
    z-index: 30;
    max-height: 260px;
    overflow-y: auto;
    padding: 5px;
    background: var(--card2);
    border: 1px solid var(--line);
    border-radius: 9px;
    box-shadow: 0 14px 40px #000a;
  }

  .opt {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 7px 8px;
    border-radius: 6px;
    font-size: 12.5px;
    color: var(--muted);
    cursor: pointer;
  }

  .opt.on {
    background: var(--hover);
    color: var(--text);
  }

  .p {
    flex: 1;
    min-width: 0;
    font-family: var(--mono);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .s {
    flex: none;
    color: var(--muted);
  }

  .note {
    padding: 7px 8px;
    font-size: 12.5px;
    color: var(--muted);
  }
</style>
