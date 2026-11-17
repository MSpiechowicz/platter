<script lang="ts">
  import Button from '../../components/ui/Button.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { platform } from '../../state/platform.svelte';
  import IntoTheTrash from './IntoTheTrash.svelte';
  import { kindsText } from './format';

  interface Props {
    /** The suggestions of the scan are known. Until they are, the card only says what it will show. */
    known: boolean;
    /** What the scan found that is safe to free, in bytes. */
    safeBytes: number;
    /** The titles of the biggest safe suggestions, biggest first. */
    kinds: readonly string[];
    /** How many suggestions need the user's review. */
    reviewCount: number;
    onreview: () => void;
  }

  let { known, safeBytes, kinds, reviewCount, onreview }: Props = $props();

  const hasSafe = $derived(safeBytes > 0);
  const offerReview = $derived(known && (hasSafe || reviewCount > 0));
</script>

<div class="none">
  <IntoTheTrash />
  <div class="words">
    <h4>No cleanups yet</h4>
    <p>
      {#if known && hasSafe}
        This scan found <span class="found">{fmt.size(safeBytes)}</span> Platter can free safely: {kindsText(
          kinds,
        )}.
      {:else if known}
        This scan found nothing that is safe to free right now.
      {/if}
      Every cleanup you run shows here, and what went to {platform.theTrash} can be restored.
    </p>
  </div>
  {#if offerReview}
    <Button variant="primary" onclick={onreview}>Review suggestions</Button>
  {/if}
</div>

<style>
  .none {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 28px;
    padding: 8px 0 4px;
  }

  h4 {
    margin: 0 0 4px;
    font-size: 15px;
  }

  p {
    margin: 0;
    max-width: 520px;
    font-size: 13px;
    line-height: 1.5;
    color: var(--muted);
  }

  .found {
    color: var(--green);
    font-weight: 600;
  }

  @media (max-width: 900px) {
    .none {
      grid-template-columns: 1fr;
      justify-items: start;
    }
  }
</style>
