<script lang="ts">
  import type { ChoiceDto, NodeId, ScanAnswerDto, ScanRowDto } from '../../api/types';
  import Button from '../../components/ui/Button.svelte';
  import Icon from '../../components/ui/Icon.svelte';
  import Tag from '../../components/ui/Tag.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { visibleText } from '../overview/display';
  import { sourceLabel, verdictWords } from './model';

  interface Props {
    answer: ScanAnswerDto;
    /** A folder was clicked, in a row or in the answer's text: open it in the Explorer. */
    onopen: (node: NodeId) => void;
    /** "Ask" on a row: the verdict for that folder. */
    onask: (row: ScanRowDto) => void;
    /** The advisor is busy, so a row cannot be asked about now. */
    askDisabled?: boolean;
    /** A choice of a question asked back was taken: ask the same question again with it. */
    onpick?: (choice: ChoiceDto) => void;
    /** The advisor is busy (or cannot be asked), so no choice can be taken now. */
    pickDisabled?: boolean;
  }

  let { answer, onopen, onask, askDisabled = false, onpick, pickDisabled = false }: Props = $props();

  /**
   * The template writes the notes into its text, with the paths in them set in “ ” quotes; a note the text already says
   * is not shown twice. Both are compared without quote marks of any kind.
   */
  const unquoted = (value: string): string => value.replace(/['"“”‟〃]/g, '');
  const notes = $derived(
    answer.notes.filter((note) => !unquoted(answer.answer).includes(unquoted(note).replace(/[.!?]?$/, ''))),
  );
</script>

<!-- Every text below comes from the backend, and a folder's name can hold any character: it all goes through visibleText. -->
<!-- A path of the scan in the text is a chip with Platter's own verdict, so a wrong claim in the wording is contradicted on the spot. -->
<!-- A question asked back is the prompt and one button per choice, and nothing else: no rows, no figures, no notes. -->
{#if answer.clarify}
  <p class="text">{visibleText(answer.clarify.prompt)}</p>

  <ul class="choices" aria-label="Which one do you mean?">
    {#each answer.clarify.choices as choice (choice.id)}
      {@const label = visibleText(choice.label)}
      <li>
        <Button
          class="choice"
          disabled={pickDisabled}
          title={choice.path === undefined ? undefined : `${visibleText(choice.path)}: ask about this one`}
          onclick={() => onpick?.(choice)}
        >
          {#if choice.path !== undefined}
            <!-- The same glyph and mono path as a row, cut at the end; the title has the whole path. -->
            <Icon name="explorer" size={12} />
            <span class="cpath">{label}</span>
          {:else}
            <span class="clabel">{label}</span>
          {/if}
        </Button>
      </li>
    {/each}
  </ul>
{:else if answer.segments.length > 0}
  <p class="text">
    {#each answer.segments as segment, index (index)}
      {#if segment.kind === 'text'}
        {visibleText(segment.text)}
      {:else}
        {@const path = visibleText(segment.path)}
        {@const words = verdictWords(segment.verdict)}
        <button
          type="button"
          class="chip"
          aria-label="{path}, {words}"
          title="{path}, {words}: open in the Explorer"
          onclick={() => onopen(segment.node)}
        >
          <Icon name="explorer" size={12} />
          <span class="cp">{path}</span>
          <span class="mark {segment.verdict ?? 'none'}" aria-hidden="true">{segment.verdict ?? ''}</span>
        </button>
      {/if}
    {/each}
  </p>
{:else}
  <p class="text">{visibleText(answer.answer)}</p>
{/if}

{#if !answer.clarify && notes.length > 0}
  <ul class="notes">
    {#each notes as note, index (index)}
      <li>{visibleText(note)}</li>
    {/each}
  </ul>
{/if}

{#if !answer.clarify && answer.rows.length > 0}
  <ul class="rows">
    {#each answer.rows as row, index (index)}
      {@const path = visibleText(row.path)}
      <li class="row">
        <!-- The path is the button that opens it in the Explorer: a folder glyph and the path, cut at the end. -->
        <button
          type="button"
          class="pline"
          aria-label="Open {path} in the Explorer"
          title="{path}: open in the Explorer"
          onclick={() => onopen(row.node)}
        >
          <Icon name="explorer" size={12} />
          <span class="p">{path}</span>
        </button>
        <b class="size num">{fmt.size(row.allocated)}</b>
        <Button size="sm" disabled={askDisabled} aria-label="Ask about {path}" onclick={() => onask(row)}
          >Ask</Button
        >

        {#if row.verdict || row.source || row.note}
          <div class="meta">
            {#if row.verdict}
              <Tag kind={row.verdict}>{row.verdict}</Tag>
            {/if}
            {#if row.source}
              <span class="src">{sourceLabel(row.source)}</span>
            {/if}
            {#if row.note}
              <span class="note" title={visibleText(row.note)}>{visibleText(row.note)}</span>
            {/if}
          </div>
        {/if}
      </li>
    {/each}
  </ul>

  {#if answer.more > 0}
    <p class="more">+{answer.more} more</p>
  {/if}
{/if}

<style>
  .text {
    margin: 0;
  }

  /*
   * A folder named in the text: a chip that fits the text line (17px in a 20px line, so the line does not grow), in the
   * row link's language (mono 11px, 4px radius, glyph and hover colours) with a line around it. It is a button that
   * opens the folder in the Explorer; the path is cut at the end when it is long.
   */
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 100%;
    height: 17px;
    margin: 0 1px;
    padding: 0 6px;
    vertical-align: middle;
    border: 1px solid var(--line);
    border-radius: 4px;
    background: #ffffff08;
    color: var(--text);
    transition:
      color 0.15s,
      background 0.15s,
      border-color 0.15s;
  }

  .chip :global(svg) {
    display: block;
    flex: none;
    color: var(--muted);
    transition: color 0.15s;
  }

  .chip:hover :global(svg),
  .chip:focus-visible :global(svg) {
    color: var(--accent);
  }

  .chip:hover,
  .chip:focus-visible {
    color: var(--accent);
    background: #ffffff0d;
    border-color: var(--dim);
  }

  .cp {
    min-width: 0;
    max-width: 56ch;
    font-family: var(--mono);
    font-size: 11px;
    line-height: 15px;
    color: inherit;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Platter's verdict in the colours of Tag; a folder nobody has a verdict for gets a plain ring. */
  .mark {
    flex: none;
    margin-left: 3px;
    font-size: 9.5px;
    font-weight: 700;
    line-height: 15px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted);
  }

  .mark.safe {
    color: var(--green);
  }

  .mark.review {
    color: var(--amber);
  }

  .mark.keep {
    color: var(--red);
  }

  .mark.none {
    width: 6px;
    height: 6px;
    border: 1px solid var(--muted);
    border-radius: 50%;
  }

  /* The choices of a question asked back: a button each, the paths in the language of a row's path. */
  .choices {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    margin: 10px 0 0;
    padding: 0;
    list-style: none;
  }

  .choices li {
    display: flex;
    max-width: 100%;
  }

  .choices :global(.choice) {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    max-width: 100%;
  }

  .choices :global(.choice svg) {
    display: block;
    flex: none;
    color: var(--muted);
    transition: color 0.15s;
  }

  .choices :global(.choice:hover:not(:disabled) svg),
  .choices :global(.choice:focus-visible svg) {
    color: var(--accent);
  }

  .cpath,
  .clabel {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cpath {
    font-family: var(--mono);
    font-size: 12px;
    font-weight: 500;
  }

  .notes {
    margin: 8px 0 0;
    padding: 0;
    list-style: none;
    font-size: 12px;
    color: var(--muted);
  }

  .rows {
    margin: 10px 0 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }

  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    column-gap: 12px;
    align-items: center;
    padding: 8px 0;
    border-bottom: 1px solid var(--line);
  }

  .row:last-child {
    border-bottom: 0;
    padding-bottom: 0;
  }

  .pline {
    display: flex;
    align-items: center;
    gap: 5px;
    width: fit-content;
    max-width: calc(100% + 6px);
    min-width: 0;
    height: 18px;
    margin-left: -6px;
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

  .pline:hover :global(svg),
  .pline:focus-visible :global(svg) {
    color: var(--accent);
  }

  .pline:hover,
  .pline:focus-visible {
    color: var(--accent);
    background: #ffffff0d;
  }

  .p {
    min-width: 0;
    font-family: var(--mono);
    font-size: 11px;
    line-height: 16px;
    color: inherit;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* The full size, never cut; every row's size ends on the same right edge. */
  .size {
    justify-self: end;
    font-size: 12.5px;
    white-space: nowrap;
  }

  .meta {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    margin-top: 4px;
    font-size: 12px;
    color: var(--muted);
  }

  .src {
    flex: none;
  }

  .note {
    min-width: 0;
    color: var(--dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row :global(.btn) {
    flex: none;
  }

  .more {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--muted);
  }
</style>
