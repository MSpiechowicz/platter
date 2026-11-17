<script lang="ts">
  import Icon from '../../components/ui/Icon.svelte';
  import { STEPS, stepMarks, type Step } from './model';

  interface Props {
    step: Step;
    /** The cleanup is over: every step is ticked. */
    complete?: boolean;
  }

  let { step, complete = false }: Props = $props();

  const marks = $derived(stepMarks(step, complete));
</script>

<!-- As wide as the box it is put in: the page gives it the width of the panel below. -->
<ol class="steps" aria-label="Steps">
  {#each STEPS as entry, index (entry.id)}
    {@const mark = marks[index]}

    <li class={mark} aria-current={mark === 'current' ? 'step' : undefined}>
      <i class="dot">
        {#if mark === 'done'}
          <Icon name="check" size={10} stroke={2.2} />
        {:else}
          <span class="n">{index + 1}</span>
        {/if}
      </i>
      <span>{entry.label}</span>
    </li>
  {/each}
</ol>

<style>
  .steps {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    width: 100%;
    margin: 0;
    padding: 0;
    list-style: none;
    border: 1px solid var(--line);
    border-radius: 7px;
    overflow: hidden;
    font-size: 12.5px;
    font-weight: 600;
  }

  li {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-width: 0;
    padding: 8px 6px;
    color: var(--dim);
    transition:
      background 0.2s,
      color 0.2s;
  }

  li + li {
    border-left: 1px solid var(--line);
  }

  /* The number sits in the middle of its circle, both ways, whatever the font's own spacing. */
  .dot {
    box-sizing: border-box;
    flex: none;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 1px solid currentColor;
    font-style: normal;
    font-size: 11px;
    font-weight: 600;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }

  /*
   * A font's line box is not centred on its digits (here the digit sat about 1 px low). Where the engine can trim the
   * box to the height of a capital, the grid centres the digit itself; elsewhere the line box is all there is.
   */
  .n {
    display: block;
    text-box: trim-both cap alphabetic;
  }

  /* The tick is drawn a little below the middle of its 12-unit grid; lift it by that much. */
  .dot :global(svg) {
    transform: translateY(-0.42px);
  }

  .current {
    background: var(--track);
    color: var(--text);
  }

  .current .dot {
    border-color: var(--accent);
    color: var(--accent);
  }

  .done {
    color: var(--muted);
  }

  /* A finished step is a filled circle with a tick. Green is kept for "safe", so it is the accent here. */
  .done .dot {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
</style>
