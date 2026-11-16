<script lang="ts">
  /** What the model is given and what it never gets. Fixed text: it describes the design, not a state. */
  const SEES: { ok: boolean; text: string }[] = [
    { ok: true, text: 'Folder and file names' },
    { ok: true, text: 'Sizes, counts and dates' },
    { ok: true, text: 'Which rules matched nearby' },
    { ok: true, text: 'For a question: only your words, not the facts from the scan' },
    { ok: false, text: 'File contents' },
    { ok: false, text: 'The network (except the one-time model download)' },
  ];
</script>

<div class="sees">
  <h4>What the model can see</h4>
  {#each SEES as row (row.text)}
    <div class="sr" class:no={!row.ok}>
      <svg class="mark" viewBox="0 0 16 16" aria-hidden="true">
        {#if row.ok}
          <path d="M3 8.5l3.2 3.2L13 4.8" />
        {:else}
          <path d="M4 4l8 8M12 4l-8 8" />
        {/if}
      </svg>
      <span><span class="vh">{row.ok ? 'Can see: ' : 'Cannot see: '}</span>{row.text}</span>
    </div>
  {/each}
</div>

<style>
  .sees {
    margin-top: auto;
    padding-top: 18px;
  }

  h4 {
    font-size: 11px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--dim);
    font-weight: 600;
    padding-top: 14px;
    border-top: 1px solid var(--line);
    margin-bottom: 8px;
  }

  .sr {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    font-size: 12.5px;
    line-height: 18px;
    padding: 4px 0;
  }

  /* An SVG, not a text glyph: sharp in every engine, and centred on the first line of a wrapped row. */
  .mark {
    flex: none;
    width: 16px;
    height: 16px;
    margin-top: 1px;
    fill: none;
    stroke: var(--accent);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .sr.no {
    color: var(--muted);
  }

  .sr.no .mark {
    stroke: var(--red);
  }

  /* Read by a screen reader only: the mark alone says it for everyone else. */
  .vh {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
