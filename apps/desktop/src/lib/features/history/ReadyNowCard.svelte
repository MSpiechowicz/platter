<script lang="ts">
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import { fmt } from '../../state/fmt.svelte';

  interface Props {
    /** The suggestions of the scan are known. Without a scan, or until they are, the card says how to get them. */
    known: boolean;
    /** A scan is ready. With `known` false it means its suggestions could not be read, not that there is no scan. */
    scanned: boolean;
    /** What the scan found that is safe to free, in bytes. */
    safeBytes: number;
    /** What the scan found that needs the user's review, in bytes. */
    reviewBytes: number;
    ongo: () => void;
  }

  let { known, scanned, safeBytes, reviewBytes, ongo }: Props = $props();

  const total = $derived(safeBytes + reviewBytes);
  const hasSafe = $derived(safeBytes > 0);
</script>

<!-- The bar and rows are those of the Overview's Reclaimable card, so the two read the same. The button keeps its
     usual size, bottom right, and is plain when only a review is left. -->
<Card title="Ready to clean now">
  {#if !known && scanned}
    <!-- The Cleanup view reads them again and offers its own retry. -->
    <p class="note">What can be cleaned could not be read.</p>
    <div class="foot"><Button onclick={ongo}>Go to Cleanup</Button></div>
  {:else if !known}
    <p class="note">Scan a disk to see what can be cleaned.</p>
  {:else if total === 0}
    <p class="note">Nothing to clean right now.</p>
  {:else}
    <div
      class="rec"
      role="img"
      aria-label="{fmt.size(safeBytes)} safe to clean, {fmt.size(reviewBytes)} to review"
    >
      <div class="fill">
        <i style="width:{(safeBytes / total) * 100}%;background:var(--green)"></i>
        <i style="width:{(reviewBytes / total) * 100}%;background:var(--amber)"></i>
      </div>
    </div>

    <div class="recr">
      <i style="background:var(--green)"></i>
      <span>Safe to clean</span>
      <span class="num">{fmt.size(safeBytes)}</span>
    </div>
    <div class="recr">
      <i style="background:var(--amber)"></i>
      <span>Needs your review</span>
      <span class="num">{fmt.size(reviewBytes)}</span>
    </div>

    <div class="foot">
      <Button variant={hasSafe ? 'primary' : undefined} onclick={ongo}>
        {hasSafe ? 'Go to Cleanup' : 'Review in Cleanup'}
      </Button>
    </div>
  {/if}
</Card>

<style>
  .rec {
    height: 10px;
    border-radius: 2px;
    overflow: hidden;
    background: var(--track);
    margin-bottom: 8px;
  }

  .fill {
    display: flex;
    height: 100%;
  }

  .fill i {
    display: block;
    height: 100%;
  }

  .recr {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    font-size: 12.5px;
    padding: 4px 0;
  }

  .recr i {
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }

  .recr span:last-child {
    color: var(--muted);
  }

  .foot {
    display: flex;
    justify-content: flex-end;
    margin-top: 8px;
  }

  .note {
    margin: 0;
    font-size: 12.5px;
    color: var(--muted);
  }
</style>
