<script lang="ts">
  import type { NodeDto } from '../../api/types';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Tag from '../../components/ui/Tag.svelte';
  import { CATEGORY_META } from '../../categories';
  import { formatCount, formatDate } from '../../format';
  import { fmt } from '../../state/fmt.svelte';
  import { platform } from '../../state/platform.svelte';
  import { visibleText } from '../overview/display';
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
    /** The mark button says "Unmark" when what the card shows is marked. */
    marked?: boolean;
    /** Why the mark button is off; `undefined` when it can be clicked. The button is always there. */
    markOff?: string | undefined;
    onmark?: () => void;
  }

  let {
    node,
    selected,
    path,
    share,
    color = null,
    canOpen,
    onopen,
    onreveal,
    marked = false,
    markOff,
    onmark,
  }: Props = $props();

  // Without a way to mark, the button is simply off.
  const off = $derived(onmark === undefined ? 'Nothing to mark.' : markOff);

  // A name can hold characters that hide or reorder what the user reads: they are made visible here.
  const shownPath = $derived(visibleText(path));

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
  <h2 class="path mono" title={shownPath}><bdi>{shownPath}</bdi></h2>
  <div class="big num">{node ? fmt.size(node.allocated) : '—'}</div>
  <div class="tag">
    {#if color}<i class="dot" style="--c:{color}" aria-hidden="true"></i>{/if}
    {#if node}<Tag>{CATEGORY_META[node.category].label}</Tag>{/if}
    {#if share !== undefined}<span class="share num">{formatShare(share)} of this folder</span>{/if}
  </div>
  <p class="meta">{meta}</p>
  <div class="actions">
    <Button disabled={!canOpen} onclick={onopen}>Go into folder</Button>
    <Button
      variant={marked ? 'default' : 'primary'}
      disabled={off !== undefined}
      title={off}
      onclick={onmark}
    >
      {marked ? 'Unmark' : 'Mark for cleanup'}
    </Button>
    <Button
      class="reveal"
      disabled={!node}
      title={platform.showInFileManager}
      aria-label={platform.showInFileManager}
      onclick={onreveal}
    >
      <svg viewBox="0 0 16 16" aria-hidden="true"
        ><path d="M2 4.5h4l1.5 1.5H14v6.5H2z" /><path d="M6.5 11l3-3M7.5 8h2v2" /></svg
      >
    </Button>
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

  /* "Go into folder" and "Mark for cleanup" share the card's width; the file manager has a small button of its own. */
  .actions :global(.btn) {
    flex: 1 1 auto;
    min-width: 0;
    padding: 8px 10px;
    font-size: 12.5px;
  }

  .actions :global(.btn.reveal) {
    flex: none;
    display: grid;
    place-items: center;
    width: 36px;
    padding: 8px 0;
  }

  .actions svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
