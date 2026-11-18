<script lang="ts">
  import { tick } from 'svelte';
  import type { ChoiceDto, EntryMatchDto, ScanRowDto } from '../../api/types';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Icon from '../../components/ui/Icon.svelte';
  import Tag from '../../components/ui/Tag.svelte';
  import { delayed } from '../../state/delayed.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import type { AdvisorStore } from '../../state/advisor.svelte';
  import { ui } from '../../state/ui.svelte';
  import { visibleText } from '../overview/display';
  import { accept, insertable, queryOf, wordAtCaret, type CaretWord } from './autocomplete';
  import { SEARCH_DELAY_MS } from './FolderPicker.svelte';
  import ScanAnswer from './ScanAnswer.svelte';
  import { askReadiness, noScanText, sourceLabel } from './model';
  import { isBlankQuestion } from '../../question';

  /** The longest question the backend takes, in characters. */
  const MAX_QUESTION_CHARS = 500;
  const ANOTHER_SCAN_TEXT = 'A question about another scan is still being answered.';

  interface Props {
    /** The finished scan the questions are about; undefined without one. */
    scan: number | undefined;
    /** A scan is running, so there is no finished one yet. */
    scanning?: boolean;
    store: AdvisorStore;
  }

  let { scan, scanning = false, store }: Props = $props();

  let question = $state('');
  let log: HTMLDivElement | undefined = $state();
  let field: HTMLInputElement | undefined = $state();

  const exchanges = $derived(scan === undefined ? [] : store.exchangesOf(scan));
  /** The model answers one free question at a time, whatever scan it is about. */
  const scanWaiting = $derived(store.scanWaiting);
  const waiting = $derived(store.waiting);
  const waitingHere = $derived(
    exchanges.some((exchange) => exchange.kind === 'scan' && !exchange.answer && !exchange.error),
  );
  /**
   * What the words settle is answered at once, so the wait is only shown when it lasts: a question that comes back
   * within a moment shows no "Reading your question…" that flashes and goes. A question about a folder never waits.
   */
  const slow = delayed(() => waitingHere);
  const lastId = $derived(exchanges.at(-1)?.id);
  /** The wait of the newest free question is shown: once it appeared, for at least a second, even when the answer is in. */
  function spinning(exchange: (typeof exchanges)[number]): boolean {
    return exchange.kind === 'scan' && exchange.id === lastId && slow.value;
  }
  const readiness = $derived(
    askReadiness({ hasScan: scan !== undefined, scanning, status: store.status, enabled: store.enabled }),
  );
  /**
   * A question needs the model; asking about a folder from a row needs only the rules, so it has its own gate: a scan,
   * and nothing else (not the model's state, labelling or a free question on its way).
   */
  const canType = $derived(readiness.ready && !scanWaiting);
  const canAskRow = $derived(scan !== undefined);
  const hint = $derived.by(() => {
    if (scan === undefined) {
      return undefined;
    }

    if (readiness.reason) {
      return readiness.reason;
    }

    return scanWaiting && !waitingHere ? ANOTHER_SCAN_TEXT : undefined;
  });

  // Another scan's question box starts empty.
  $effect(() => {
    void scan;
    question = '';
    reset();
  });

  const id = $props.id();
  const listId = `${id}-list`;

  /** What the word under the caret may be completed to, and the word those were looked up for. */
  let results = $state<EntryMatchDto[]>([]);
  let looked: { start: number; query: string } | undefined;
  /** The word being looked up or listed: the search of a word the caret has left is dropped. */
  let wanted: { start: number; query: string } | undefined;
  let open = $state(false);
  let active = $state(-1);

  let timer: ReturnType<typeof setTimeout> | undefined;
  /** Bumped by every key, every pick and every reset, so an answer that arrives late is ignored. */
  let sequence = 0;

  const showList = $derived(open && results.length > 0);

  // A card that goes away takes its waiting search with it.
  $effect(() => () => {
    clearTimeout(timer);
    sequence++;
  });

  // A field that is shut has no list.
  $effect(() => {
    if (!canType) {
      reset();
    }
  });

  function reset(): void {
    clearTimeout(timer);
    sequence++;
    results = [];
    looked = undefined;
    wanted = undefined;
    open = false;
    active = -1;
  }

  /** Looks for the word at the caret after the pause, and closes the list when there is nothing to look for. */
  function schedule(value: string, caret: number): void {
    clearTimeout(timer);
    sequence++;

    const word = scan === undefined || !canType ? undefined : wordAtCaret(value, caret);

    if (!word) {
      reset();

      return;
    }

    const mine = sequence;
    const target = scan as number;

    wanted = { start: word.start, query: queryOf(word) };

    timer = setTimeout(() => void search(mine, target, word), SEARCH_DELAY_MS);
  }

  async function search(mine: number, target: number, word: CaretWord): Promise<void> {
    const query = queryOf(word);

    try {
      const found = await store.findEntries(target, query);

      if (mine !== sequence) {
        return;
      }

      // The caret may have left the word by a key no handler sees (ArrowUp, PageDown, select all).
      if (!caretAt(word.start, query)) {
        reset();

        return;
      }

      // A name the question could not carry intact is not offered (see `insertable`).
      const offered = found.filter((match) => insertable(match.path));

      results = offered;
      looked = { start: word.start, query };
      active = offered.length > 0 ? 0 : -1;
      open = offered.length > 0;
    } catch {
      if (mine !== sequence) {
        return;
      }

      // The question box is for questions: a search that fails only leaves it without suggestions.
      reset();
    }
  }

  function typed(): void {
    if (!field) {
      return;
    }

    schedule(field.value, field.selectionStart ?? field.value.length);
  }

  /** The caret is still in the word that starts at `start` and is looked up as `query`. */
  function caretAt(start: number, query: string): boolean {
    if (!field) {
      return false;
    }

    const word = wordAtCaret(field.value, field.selectionStart ?? field.value.length);

    return word !== undefined && word.start === start && queryOf(word) === query;
  }

  /** The word at the caret when the list on screen is the one made for it, else undefined. */
  function listed(): CaretWord | undefined {
    if (!field || looked === undefined) {
      return undefined;
    }

    const word = wordAtCaret(field.value, field.selectionStart ?? field.value.length);

    return word && word.start === looked.start && queryOf(word) === looked.query ? word : undefined;
  }

  /**
   * The caret moved by a key or a click: the list, or the search on its way, made for another word is dropped, so a
   * late result for the word the caret left never opens.
   */
  function caretMoved(): void {
    if (wanted === undefined || !field) {
      return;
    }

    const word = wordAtCaret(field.value, field.selectionStart ?? field.value.length);

    if (!word || word.start !== wanted.start || queryOf(word) !== wanted.query) {
      reset();
    }
  }

  function move(step: number): void {
    active = Math.max(0, Math.min(results.length - 1, active + step));
  }

  /** Puts a match in place of the word at the caret. A folder starts the search of its next level. */
  async function choose(match: EntryMatchDto | undefined): Promise<void> {
    const word = listed();

    if (!match || !field || !word) {
      reset();

      return;
    }

    const next = accept(field.value, word, match);

    question = next.value;
    schedule(next.value, next.caret);

    await tick();
    field?.setSelectionRange(next.caret, next.caret);
  }

  function onKey(event: KeyboardEvent): void {
    if (event.isComposing) {
      return;
    }

    // A list that is still the one of the word before the last key is not taken by Tab or Enter: they go on as usual.
    const current = showList && listed() !== undefined;

    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      // With the list closed the arrows move the caret, as in any field.
      if (!showList) {
        return;
      }

      move(event.key === 'ArrowDown' ? 1 : -1);
    } else if (event.key === 'Tab' && current && !event.shiftKey) {
      void choose(results[active]);
    } else if (event.key === 'Enter' && current) {
      void choose(results[active]);
    } else if (event.key === 'Escape' && showList) {
      open = false;
      event.stopPropagation();
    } else {
      return;
    }

    event.preventDefault();
  }

  /** How far from the end the log may be scrolled and still count as reading the newest message. */
  const NEAR_END_PX = 48;

  /**
   * This card scrolled the log to the newest message, and the reader has not scrolled since. A smooth scroll that is
   * still on its way reads as "not at the end", so this is what tells it from a reader who went up.
   */
  let following = true;
  let newest: number | undefined;
  let wasNearEnd = true;

  /** What changes the height of the log at its end: a question, the wait that shows after a moment, an answer. */
  const tail = $derived.by(() => {
    const last = exchanges.at(-1);

    return `${last?.id}:${slow.value}:${last?.answer ? 'answer' : ''}:${last?.error ? 'error' : ''}`;
  });

  function nearEnd(): boolean {
    if (!log) {
      return true;
    }

    return following || log.scrollHeight - log.scrollTop - log.clientHeight <= NEAR_END_PX;
  }

  // The reader takes the log from the card: a wheel, a finger, the keys or the scroll bar.
  $effect(() => {
    const node = log;
    const events = ['wheel', 'touchmove', 'keydown', 'pointerdown'];
    const release = (): void => {
      following = false;
    };

    for (const name of events) {
      node?.addEventListener(name, release, { passive: true });
    }

    return () => {
      for (const name of events) {
        node?.removeEventListener(name, release);
      }
    };
  });

  // Before the new message is drawn, so the log is measured as the reader left it.
  $effect.pre(() => {
    void tail;

    wasNearEnd = nearEnd();
  });

  // A question that was just sent is always shown. What comes after it (the wait, the answer) is shown unless the
  // reader scrolled up to older answers: that is not taken away from them.
  $effect(() => {
    void tail;

    const last = exchanges.at(-1);
    const sent = last?.id !== newest;

    newest = last?.id;

    if (!log || (!sent && !wasNearEnd)) {
      return;
    }

    following = true;

    if (last === undefined) {
      log.scrollTop = 0;
      return;
    }

    // As much of the newest exchange as fits, from its question on: the answer starts in view even when it is long.
    const question = [...log.querySelectorAll<HTMLElement>('.msg.q')].at(-1);
    const end = Math.max(0, log.scrollHeight - log.clientHeight);

    log.scrollTop = Math.min(end, question?.offsetTop ?? end);
  });

  // A disabled field loses the focus: when the user sent the question, it comes back with the answer.
  let refocus = false;

  $effect(() => {
    if (refocus && !scanWaiting) {
      refocus = false;
      tick().then(() => field?.focus({ preventScroll: true }));
    }
  });

  function submit(event?: SubmitEvent): void {
    event?.preventDefault();

    if (scan === undefined || !canType || isBlankQuestion(question)) {
      return;
    }

    reset();
    refocus = true;
    void store.askScan(scan, question);
    question = '';
  }

  /** A choice of a question asked back: the same question again, with the choice. */
  function pick(question: string, choice: ChoiceDto): void {
    if (scan === undefined || !canType) {
      return;
    }

    refocus = true;
    void store.askScan(scan, question, choice);
  }

  /** Takes this scan's questions off the screen (nothing is stored) and leaves the box ready for the next one. */
  function clear(): void {
    if (scan === undefined || waiting) {
      return;
    }

    store.clearExchanges(scan);
    tick().then(() => field?.focus({ preventScroll: true }));
  }

  function askFolder(row: ScanRowDto): void {
    if (scan === undefined || !canAskRow) {
      return;
    }

    void store.ask(scan, row.node, row.path);
  }
</script>

<Card title="Ask about this scan" class="ask">
  <div class="log" bind:this={log} role="log" aria-label="Questions and answers">
    {#each exchanges as exchange (exchange.id)}
      {@const question = exchange.kind === 'scan' ? exchange.question : ''}
      <div class="msg q">
        <div class="who">You</div>
        <div class="bd" class:plain={exchange.kind === 'scan'}>
          {#if exchange.kind === 'folder'}
            Can I delete {visibleText(exchange.path)}?
          {:else if exchange.pick}
            You picked: {visibleText(exchange.pick.label)}
          {:else}
            {visibleText(exchange.question)}
          {/if}
        </div>
      </div>

      <!-- A free question still on its way shows nothing until it has taken a moment: most come back at once. A question about a folder has no wait to show. -->
      {#if exchange.answer || exchange.error || spinning(exchange)}
        <div class="msg a">
          <div class="who"><Icon name="advisor" size={14} /> Advisor</div>
          <div class="bd">
            {#if spinning(exchange)}
              <span class="wait muted" role="status">
                <i class="ring" aria-hidden="true"></i>
                Reading your question…
              </span>
            {:else if exchange.kind === 'scan' && exchange.answer}
              <ScanAnswer
                answer={exchange.answer}
                onopen={(node) => ui.openInExplorer(node)}
                onask={askFolder}
                askDisabled={!canAskRow}
                onpick={(choice) => pick(question, choice)}
                pickDisabled={!canType}
              />
            {:else if exchange.kind === 'folder' && exchange.answer}
              {visibleText(exchange.answer.reason)}
              <div class="v">
                <Tag kind={exchange.answer.verdict}>{exchange.answer.verdict}</Tag>
                <span class="src">{sourceLabel(exchange.answer.source)}</span>
                <span class="detail">{visibleText(exchange.answer.detail)}</span>
              </div>
            {:else if exchange.error}
              <div class="fail" role="alert">
                <span class="muted">{visibleText(exchange.error)}</span>
                <Button
                  size="sm"
                  disabled={exchange.kind === 'scan' && scanWaiting}
                  onclick={() => store.retry(exchange.id)}>Try again</Button
                >
              </div>
            {:else}
              <span class="wait muted" role="status">
                <i class="ring" aria-hidden="true"></i>
                Reading your question…
              </span>
            {/if}
          </div>
        </div>
      {/if}
    {:else}
      <p class="empty">
        {scan === undefined
          ? noScanText(scanning)
          : 'Ask anything about what was scanned. The local model only reads your question. Platter answers from the scan and its rules.'}
      </p>
    {/each}
  </div>

  {#if hint}
    <p class="hint">{hint}</p>
  {/if}

  <form class="inp" onsubmit={submit}>
    <!-- The field stretches to the height of the button beside it. -->
    <div class="field" class:disabled={!canType}>
      <input
        type="text"
        bind:this={field}
        bind:value={question}
        maxlength={MAX_QUESTION_CHARS}
        placeholder="Ask about this scan, in your own words"
        aria-label="Question about this scan"
        autocomplete="off"
        spellcheck="false"
        disabled={!canType}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={showList}
        aria-controls={listId}
        aria-activedescendant={showList && active >= 0 ? `${id}-o${active}` : undefined}
        oninput={typed}
        onkeydown={onKey}
        onkeyup={(event) =>
          ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown'].includes(
            event.key,
          ) && caretMoved()}
        onclick={caretMoved}
        onblur={reset}
      />

      {#if showList}
        <!-- Above the field: it sits at the bottom of its card. A mouse press must not take the focus from the field. -->
        <div
          class="list"
          id={listId}
          role="listbox"
          aria-label="Folders and files"
          tabindex="-1"
          onmousedown={(event) => event.preventDefault()}
        >
          {#each results as match, index (match.node)}
            {@const path = `${visibleText(match.path)}${match.kind === 'dir' ? '/' : ''}`}
            <div
              id="{id}-o{index}"
              class="opt"
              class:on={index === active}
              role="option"
              tabindex="-1"
              aria-selected={index === active}
              aria-label="{match.kind === 'dir' ? 'Folder' : 'File'} {path}, {fmt.size(match.allocated)}"
              onclick={() => void choose(match)}
              onkeydown={(event) => event.key === 'Enter' && void choose(match)}
              onmouseenter={() => (active = index)}
            >
              <span class="p" title={path}>{path}</span>
              <span class="s num">{fmt.size(match.allocated)}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>
    <Button type="submit" variant="primary" disabled={!canType || isBlankQuestion(question)}>Ask</Button>
    <!-- Always there, so the field keeps its width; nothing to clear or a question on its way disables it. -->
    <Button type="button" disabled={waiting || exchanges.length === 0} onclick={clear}>Clear</Button>
  </form>
</Card>

<style>
  /*
   * The log takes the height the card has and scrolls inside it, so the question box below stays in view however
   * many exchanges there are. The scroll bar sits in the card's padding, as in the list of the rules.
   */
  .log {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
    scrollbar-gutter: stable;
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-right: -16px;
    padding-right: 10px;
    scroll-behavior: smooth;
  }

  /* Less motion: the log jumps to the newest message. */
  :global(:root[data-motion='reduce']) .log {
    scroll-behavior: auto;
  }

  @media (prefers-reduced-motion: reduce) {
    .log {
      scroll-behavior: auto;
    }
  }

  /* One column: the card takes the height of its content, so the log stops short of the page and scrolls. */
  @media (max-width: 800px) {
    .log {
      flex: none;
      min-height: 160px;
      max-height: 60vh;
    }
  }

  .empty {
    color: var(--muted);
    font-size: 12.5px;
  }

  .msg {
    max-width: 84%;
    font-size: 13px;
    animation: rise 0.3s both;
  }

  .who {
    font-size: 11px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--dim);
    font-weight: 600;
    margin-bottom: 5px;
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .q {
    align-self: flex-end;
  }

  .q .who {
    justify-content: flex-end;
  }

  .q .bd {
    background: var(--accent-wash);
    border: 1px solid var(--accent-edge);
    /* One radius on every corner (owner: the mockup's speech-tail corner read as a wrong shape). */
    border-radius: 10px;
    padding: 9px 12px;
    font-family: var(--mono);
    font-size: 12.5px;
    overflow-wrap: anywhere;
  }

  .a .who {
    color: var(--accent);
  }

  .a .bd {
    background: var(--card2);
    border-radius: 10px;
    padding: 11px 13px;
    line-height: 1.55;
    overflow-wrap: anywhere;
  }

  .v {
    margin-top: 9px;
    padding-top: 9px;
    border-top: 1px solid var(--line);
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 12px;
    color: var(--muted);
  }

  .src {
    flex: none;
  }

  .detail {
    min-width: 0;
    color: var(--dim);
  }

  /* A free question is words, not a path. */
  .q .bd.plain {
    font-family: inherit;
    font-size: 13px;
  }

  .fail {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .wait {
    display: inline-flex;
    align-items: center;
    gap: 9px;
  }

  .ring {
    flex: none;
    width: 14px;
    height: 14px;
    border: 2px solid var(--ring);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: askspin 0.9s linear infinite;
  }

  @keyframes askspin {
    to {
      transform: rotate(360deg);
    }
  }

  .hint {
    margin-top: 12px;
    font-size: 12px;
    color: var(--muted);
  }

  /* The field is as tall as the button beside it. */
  .inp {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    column-gap: 8px;
    margin-top: 14px;
  }

  .field {
    position: relative;
    display: flex;
    min-width: 0;
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
    font-size: 13px;
    line-height: 18px;
    color: var(--text);
    background: none;
    border: 0;
    outline: none;
  }

  input::placeholder {
    color: var(--dim);
  }

  /* The list of the folder picker, above the field. */
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
    box-shadow: 0 14px 40px var(--shadow-2);
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
</style>
