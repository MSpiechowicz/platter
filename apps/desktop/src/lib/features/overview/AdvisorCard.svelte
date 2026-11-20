<script lang="ts">
  import { untrack } from 'svelte';
  import Card from '../../components/ui/Card.svelte';
  import { advisor } from '../../state/advisor.svelte';
  import AdvisorList, { type AdvisorListProps } from './AdvisorList.svelte';
  import { putBackText, undoNotes } from './cleanupText';
  import OutdatedHero from './OutdatedHero.svelte';

  interface Props extends AdvisorListProps {
    /** A cleanup ran on the scan: the list gives way to a note, since nothing can be cleaned from it. */
    outdated?: boolean;
  }

  let { outdated = false, ...props }: Props = $props();

  // The list loads the suggestions into the store. While it is not shown the card does, so the other views and the
  // store agree on which scan is on screen. Loading the same data again changes nothing.
  $effect.pre(() => {
    const id = props.scanId;
    const data = props.suggestions;

    if (outdated) {
      untrack(() => props.store.load(id, data));
    }
  });

  /** What the cleanup that made the scan old freed and when, if this session ran one on this scan and did not undo it. */
  const lastCleanup = $derived.by(() => {
    const phase = props.store.phase;

    if (phase.kind !== 'done') {
      return undefined;
    }

    const { result } = phase;

    if (result.kind !== 'finished' || result.scanId !== props.scanId || result.undo !== undefined) {
      return undefined;
    }

    if (result.freed === undefined || result.freed <= 0 || result.endedAt === undefined) {
      return undefined;
    }

    return { freed: result.freed, endedAt: result.endedAt };
  });

  /** The result of the cleanup that made this scan old, while it is the one on screen. */
  const lastResult = $derived.by(() => {
    const phase = props.store.phase;

    return phase.kind === 'done' && phase.result.scanId === props.scanId ? phase.result : undefined;
  });

  // The footer offered this, the same click and the same store call: the backend asks in its own dialog, and a
  // "no" there changes nothing.
  const canUndo = $derived(lastResult !== undefined && props.store.actions.includes('undo'));

  /** How an undo went; the footer that said it is not shown while the scan is outdated. */
  const undone = $derived.by(() => {
    if (!lastResult?.undo) {
      return [];
    }

    return [putBackText(lastResult), ...undoNotes(lastResult)].filter((line) => line !== undefined);
  });
</script>

<!-- One card for both, so it does not enter again when the scan turns old. -->
<Card
  title="Advisor"
  tag={outdated ? 'PAUSED' : undefined}
  hint={advisor.loaded ? 'LOCAL MODEL' : 'RULES'}
  class="advisor"
>
  {#if outdated}
    <OutdatedHero
      freed={lastCleanup?.freed}
      endedAt={lastCleanup?.endedAt}
      onundo={canUndo ? () => void props.store.undo() : undefined}
      {undone}
    />
  {:else}
    <AdvisorList {...props} />
  {/if}
</Card>
