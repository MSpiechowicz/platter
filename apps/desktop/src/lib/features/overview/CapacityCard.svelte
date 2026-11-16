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
    /** For a folder scan, the disk the folder is on: the card then says how much of it the folder takes. */
    container?: VolumeDto;
    totals: Totals;
  }

  let { volume, container, totals }: Props = $props();

  /** The file size is worth its own figure only when it differs from the space on disk (sparse or compressed). */
  const sizeDiffers = $derived(
    Math.abs(totals.apparent - totals.allocated) >= 0.05 * Math.max(totals.apparent, totals.allocated, 1),
  );
  /** The folder's share of its disk's capacity, as text. */
  const diskShare = $derived.by(() => {
    if (!container || container.total <= 0) {
      return undefined;
    }

    const percent = (totals.allocated / container.total) * 100;

    return percent > 0 && percent < 0.1
      ? '<0.1%'
      : `${percent < 10 ? percent.toFixed(1) : Math.round(percent)}%`;
  });

  const used = $derived(volume ? volume.total - volume.available : totals.allocated);
  const share = $derived(volume && volume.total > 0 ? used / volume.total : 0);
  /**
   * On btrfs, snapshots and copies share disk blocks, and each folder that holds them is charged in full: the
   * folders then add up to more than the disk uses. Nothing new is measured; the two totals are compared.
   */
  const sharesBlocks = $derived(volume?.fsType === 'btrfs' && totals.allocated > used);

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
      {#if sharesBlocks}
        <p class="note">
          Some files share disk blocks (snapshots or copies), so folder sizes add up to more than the disk
          uses.
        </p>
      {/if}
    {:else}
      <div class="scanned">
        <b class="num">{fmt.size(animatedUsed.value)}</b>
        <span class="label">ON DISK</span>
      </div>
      <div class="kv" class:two={!sizeDiffers && diskShare === undefined}>
        <div><b class="num">{formatCount(totals.files)}</b>files</div>
        <div><b class="num">{formatCount(totals.dirs)}</b>folders</div>
        {#if sizeDiffers}
          <div><b class="num">{fmt.size(totals.apparent)}</b>file size</div>
        {:else if diskShare !== undefined}
          <div title="Of the {fmt.size(container?.total ?? 0)} disk at {container?.mountPoint}">
            <b class="num">{diskShare}</b>of its disk
          </div>
        {/if}
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

  .kv.two {
    grid-template-columns: repeat(2, 1fr);
  }

  .note {
    margin: 12px 0 0;
    font-size: 12px;
    line-height: 1.4;
    color: var(--muted);
    text-align: center;
  }

  .kv b {
    display: block;
    color: var(--text);
    font-size: 14px;
  }
</style>
