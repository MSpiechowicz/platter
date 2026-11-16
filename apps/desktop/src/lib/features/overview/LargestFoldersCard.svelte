<script lang="ts">
  import type { NodeDto, NodeId } from '../../api/types';
  import Bar from '../../components/ui/Bar.svelte';
  import Card from '../../components/ui/Card.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { fewFoldersNote } from './folders';

  interface Props {
    folders: NodeDto[];
    /** Path of each folder, by node id. */
    paths: Map<NodeId, string>;
    /** Bar colour of each folder: the colour of its slab in the stack. */
    colors: Map<NodeId, string>;
    /** A folder was opened; `color` is its own colour, when it is known. */
    onopen: (nodeId: NodeId, color?: string) => void;
  }

  let { folders, paths, colors, onopen }: Props = $props();

  const largest = $derived(folders[0]?.allocated || 1);
  const note = $derived(fewFoldersNote(folders.length));
</script>

<Card title="Largest folders">
  <div class="fold">
    {#each folders as folder (folder.id)}
      {@const path = paths.get(folder.id) ?? folder.name}
      <button
        type="button"
        class="item"
        title="{path}: open in the Explorer"
        onclick={() => onopen(folder.id, colors.get(folder.id))}
      >
        <!-- The path is cut at its start, so the folder's own name stays visible. -->
        <span class="path mono"><bdi>{path}</bdi></span>
        <span class="size num">{fmt.size(folder.allocated)}</span>
        <span class="bar"
          ><Bar
            value={folder.allocated / largest}
            color={colors.get(folder.id) ?? 'var(--slate)'}
            label="{path} share"
          /></span
        >
      </button>
    {/each}

    {#if note}
      <p class="note">{note}</p>
    {/if}
  </div>
</Card>

<style>
  /*
   * Rows keep one fixed height and are never stretched. This card sets the height of the bottom row, never less
   * than the age card needs; the age card's chart takes whatever height that leaves.
   */
  .fold {
    display: flex;
    flex-direction: column;
  }

  .item {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 4px 10px;
    align-items: center;
    width: 100%;
    text-align: left;
    align-content: center;
    height: 38px;
    font-size: 12.5px;
    border-radius: 4px;
  }

  .note {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--muted);
  }

  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
    color: var(--text);
    transition: color 0.15s;
  }

  .size {
    color: var(--muted);
    text-align: right;
    min-width: 50px;
  }

  .bar {
    grid-column: 1 / -1;
  }

  .item:hover .path {
    color: var(--accent);
  }
</style>
