<script lang="ts">
  import type { Totals, VolumeDto } from '../../api/types';
  import { formatCount } from '../../format';
  import Card from '../../components/ui/Card.svelte';
  import SegRing from '../../components/ui/SegRing.svelte';
  import { countUp } from '../../motion/countUp.svelte';
  import { fmt } from '../../state/fmt.svelte';

  interface Props {
    /** Set when the scan root is a whole volume: the card then shows the disk, not just the scan. */
    volume?: VolumeDto;
    totals: Totals;
  }

  let { volume, totals }: Props = $props();

  const used = $derived(volume ? volume.total - volume.available : totals.allocated);
  const share = $derived(volume && volume.total > 0 ? used / volume.total : 0);

  const animatedShare = countUp(() => share * 100);
  const animatedUsed = countUp(() => used);
  const animatedFree = countUp(() => volume?.available ?? 0);
</script>

<Card title={volume ? 'Capacity' : 'Scanned'}>
  <div class="body">
    {#if volume}
      <div class="gauge">
        <SegRing value={share} size={190} segments={56}>
          <b class="num">{Math.round(animatedShare.value)}%</b>
          <span class="label">USED</span>
        </SegRing>
      </div>
      <div class="kv">
        <div><b class="num">{fmt.size(animatedUsed.value)}</b>used</div>
        <div><b class="num">{fmt.size(animatedFree.value)}</b>free</div>
        <div><b class="num">{fmt.size(volume.total)}</b>total</div>
      </div>
    {:else}
      <div class="scanned">
        <b class="num">{fmt.size(animatedUsed.value)}</b>
        <span class="label">SCANNED</span>
      </div>
      <div class="kv">
        <div><b class="num">{formatCount(totals.files)}</b>files</div>
        <div><b class="num">{formatCount(totals.dirs)}</b>folders</div>
        <div><b class="num">{fmt.size(totals.apparent)}</b>file size</div>
      </div>
    {/if}
  </div>
</Card>

<style>
  .body {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
  }

  .gauge {
    display: grid;
    place-items: center;
  }

  .gauge b,
  .scanned b {
    display: block;
    font-size: 30px;
    font-weight: 700;
    line-height: 1;
  }

  .label {
    font-size: 11px;
    letter-spacing: 0.14em;
    color: var(--muted);
  }

  .scanned {
    display: grid;
    justify-items: center;
    gap: 6px;
    padding: 36px 0 26px;
  }

  .kv {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    margin-top: 10px;
    font-size: 12px;
    color: var(--muted);
    text-align: center;
  }

  .kv b {
    display: block;
    color: var(--text);
    font-size: 14px;
  }
</style>
