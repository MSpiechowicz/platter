<script lang="ts">
  import Bar from '../../components/ui/Bar.svelte';
  import Icon from '../../components/ui/Icon.svelte';
  import Tag from '../../components/ui/Tag.svelte';
  import { fmt } from '../../state/fmt.svelte';

  interface Props {
    /** Mount point or folder path. */
    path: string;
    /** "Backup · sda1 · ext4" */
    detail: string;
    selected: boolean;
    removable?: boolean;
    /** Other mount points of the same filesystem, shown as a dim line. */
    alsoMountedAt?: string[];
    /** Disk usage; a folder has none. */
    usage?: { used: number; total: number };
    onselect: () => void;
  }

  let { path, detail, selected, removable = false, alsoMountedAt = [], usage, onselect }: Props = $props();

  const also = $derived(alsoMountedAt.join(', '));
</script>

<button type="button" aria-pressed={selected} class="row" class:selected onclick={onselect}>
  <Icon name={usage ? 'disk' : 'explorer'} size={18} />
  <span class="main">
    <span class="top">
      <span class="path mono" title={path}>{path}</span>
      {#if removable}<Tag>removable</Tag>{/if}
      <span class="detail">{detail}</span>
    </span>
    {#if usage}
      <span class="usage">
        <Bar value={usage.total > 0 ? usage.used / usage.total : 0} label="Disk used" />
        <span class="num">{fmt.size(usage.used)} of {fmt.size(usage.total)}</span>
      </span>
    {/if}
    {#if also}
      <span class="also" title="Also mounted at {also}">also mounted at {also}</span>
    {/if}
  </span>
</button>

<style>
  .row {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr);
    gap: 12px;
    align-items: start;
    width: 100%;
    text-align: left;
    padding: 12px 14px;
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--muted);
    transition:
      border-color 0.15s,
      background 0.15s;
  }

  .row:hover {
    border-color: var(--dim);
  }

  .row.selected {
    border-color: var(--accent);
    background: var(--wash-2);
    color: var(--text);
  }

  .row :global(svg) {
    margin-top: 1px;
  }

  .row.selected :global(svg) {
    color: var(--accent);
  }

  .main {
    display: grid;
    gap: 8px;
    min-width: 0;
  }

  .top {
    min-height: 22px;
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .path {
    color: var(--text);
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail {
    margin-left: auto;
    font-size: 12px;
    white-space: nowrap;
  }

  .also {
    color: var(--dim);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .usage .num {
    text-align: right;
  }

  .usage {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 112px;
    align-items: center;
    gap: 14px;
    font-size: 12px;
  }
</style>
