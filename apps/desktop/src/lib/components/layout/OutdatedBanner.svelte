<script lang="ts">
  import { cleanup } from '../../state/cleanup.svelte';
  import { history } from '../../state/history.svelte';
  import { scan } from '../../state/scan.svelte';
  import Button from '../ui/Button.svelte';
  import Banner from './Banner.svelte';

  // A cleanup, an undo or a restore from History is moving files: a scan now would see them half moved.
  const blocked = $derived(cleanup.busy || cleanup.undoing || history.restoring !== undefined);
</script>

<Banner
  icon="refresh"
  title="This scan is out of date"
  said="This scan is out of date. Cleaning is paused until you rescan."
>
  The disk changed after the cleanup. Figures are from before it and cleaning is paused.

  {#snippet actions()}
    <Button variant="primary" disabled={blocked} onclick={() => void scan.rescan()}>Rescan</Button>
  {/snippet}
</Banner>
