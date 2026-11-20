<script lang="ts">
  import type { Totals, VolumeDto } from '../../api/types';
  import { formatCount, percentText } from '../../format';
  import Card from '../../components/ui/Card.svelte';
  import SegRing from '../../components/ui/SegRing.svelte';
  import { countUp } from '../../motion/countUp.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { STALE_TEXT } from './cleanupText';

  interface Props {
    /** Set when the scan root is a whole volume: the card then shows the disk, not just the scan. */
    volume?: VolumeDto;
    /** For a folder scan, the disk the folder is on: the card then says how much of it the folder takes. */
    container?: VolumeDto;
    totals: Totals;
    /** A cleanup ran since the scan: the figures describe the disk from before it, and are greyed. */
    stale?: boolean;
  }

  let { volume, container, totals, stale = false }: Props = $props();

  /** The file size is worth its own figure only when it differs from the space on disk (sparse or compressed). */
  const sizeDiffers = $derived(
    Math.abs(totals.apparent - totals.allocated) >= 0.05 * Math.max(totals.apparent, totals.allocated, 1),
  );
  /** The folder's share of its disk's capacity, as text. */
  const diskShare = $derived.by(() => {
    if (!container || container.total <= 0) {
      return undefined;
    }

    return percentText(totals.allocated / container.total, { fine: true });
  });

  const used = $derived(volume ? volume.total - volume.available : totals.allocated);
  const share = $derived(volume && volume.total > 0 ? used / volume.total : 0);
  /**
   * On btrfs, snapshots and copies share disk blocks, and each folder that holds them is charged in full: the
   * folders then add up to more than the disk uses. Nothing new is measured; the two totals are compared.
   */
  const sharesBlocks = $derived(volume?.fsType === 'btrfs' && totals.allocated > used);

  const animatedShare = countUp(() => share);
  const animatedUsed = countUp(() => used);
  const animatedFree = countUp(() => volume?.available ?? 0);

  /** The percent figure, with its sign apart: a full-size "%" widens the figure and pulls it off centre. */
  const shareText = $derived(percentText(animatedShare.value));
  const shareFigure = $derived(shareText.endsWith('%') ? shareText.slice(0, -1) : shareText);
</script>

<Card title={volume ? 'Capacity' : 'Scanned'} tag={stale ? 'OLD SCAN' : undefined}>
  <div class="body" class:stale title={stale ? STALE_TEXT : undefined}>
    {#if volume}
      <div class="gauge">
        <SegRing value={share} size={150} segments={56}>
          <b class="num pct"
            >{shareFigure}{#if shareFigure !== shareText}<span class="sign">%</span>{/if}</b
          >
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

  /* Inside the ring's inner circle (about 84 px across at this size): smaller, with room to the line. */
  .gauge b.pct {
    font-size: 26px;
    margin-bottom: 4px;
  }

  .sign {
    margin-left: 1px;
    font-size: 0.62em;
    font-weight: 600;
  }

  .label {
    font-size: 11px;
    letter-spacing: 0.14em;
    color: var(--muted);
  }

  /* Letter spacing also follows the last letter: taken back, so the word sits in the middle. */
  .gauge .label {
    display: block;
    margin-right: -0.14em;
    font-size: 10px;
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
