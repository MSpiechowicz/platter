<script lang="ts">
  import type { NodeDto } from '../../api/types';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Tag from '../../components/ui/Tag.svelte';
  import { CATEGORY_META } from '../../categories';
  import { formatCount, formatDate } from '../../format';
  import { fmt } from '../../state/fmt.svelte';
  import { formatShare } from './layout/scale';

  interface Props {
    /** The selected item, or the open folder when nothing is selected. */
    node?: NodeDto;
    selected: boolean;
    path: string;
    /** The share of the open folder that the selected item holds, as a fraction; none when nothing is selected. */
    share?: number;
    /** The colour of the item (or of the open folder), shown as a dot. */
    color?: string | null;
    canOpen: boolean;
    onopen: () => void;
    onreveal: () => void;
  }

  let { node, selected, path, share, color = null, canOpen, onopen, onreveal }: Props = $props();

  const meta = $derived.by(() => {
    if (!node) {
      return '';
    }

    const parts: string[] = [];

    if (node.kind === 'dir' || node.kind === 'mount') {
      parts.push(node.files === 1 ? '1 file' : `${formatCount(node.files)} files`);
    }

    parts.push(`apparent ${fmt.size(node.apparent)}`);

    if (node.mtime > 0) {
      parts.push(`modified ${formatDate(node.mtime)}`);
    }

    return parts.join(' · ');
  });
</script>

<!-- Always the same height: the text lines have room reserved and both buttons are always there, only enabled or not. -->
<Card title={selected ? 'Selected' : 'Current folder'} class="details">
  <h2 class="path mono" title={path}><bdi>{path}</bdi></h2>
  <div class="big num">{node ? fmt.size(node.allocated) : '—'}</div>
  <div class="tag">
    {#if color}<i class="dot" style="--c:{color}" aria-hidden="true"></i>{/if}
    {#if node}<Tag>{CATEGORY_META[node.category].label}</Tag>{/if}
    {#if share !== undefined}<span class="share num">{formatShare(share)} of this folder</span>{/if}
  </div>
  <p class="meta">{meta}</p>
  <div class="actions">
    <Button disabled={!canOpen} onclick={onopen}>Open folder</Button>
    <Button disabled={!node} onclick={onreveal}>Show in file manager</Button>
  </div>
</Card>

<style>
  /* The path is cut at its start, so the name at its end stays visible. */
  .path {
    font-size: 16px;
    font-weight: 400;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }

  .big {
    font-size: 30px;
    font-weight: 700;
    margin: 6px 0;
  }

  .tag {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 22px;
  }

  .share {
    font-size: 12.5px;
    color: var(--muted);
  }

  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    flex: none;
    border-radius: 50%;
    background: var(--c);
  }

  .meta {
    font-size: 12.5px;
    color: var(--muted);
    margin: 10px 0 12px;
    min-height: 2.9em;
    line-height: 1.45;
  }

  .actions {
    display: flex;
    gap: 8px;
  }

  /* "Open folder" and "Show in file manager" share the 286 px of the card. */
  .actions :global(.btn) {
    flex: 1 1 auto;
    min-width: 0;
    padding: 8px 10px;
    font-size: 12.5px;
  }
</style>
