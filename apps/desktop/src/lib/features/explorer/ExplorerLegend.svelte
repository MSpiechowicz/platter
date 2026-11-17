<script lang="ts">
  import type { ExplorerView } from '../../state/settings.svelte';

  interface Props {
    view: ExplorerView;
    /** 3D: how many small items have no column and are not drawn. */
    omitted?: number;
    /** Some tile or column is drawn larger than its true share, so that it can be read. */
    boosted?: boolean;
    /** The open folder is inside one that is marked for cleanup: everything on the map goes with it. */
    insideMarked?: boolean;
  }

  let { view, omitted = 0, boosted = false, insideMarked = false }: Props = $props();

  const text = $derived.by(() => {
    const base =
      view === '3d' ? 'Column = folder · floor = sub-folder · volume = space' : 'Bigger tile = more space';

    return boosted ? `${base} · small folders are drawn larger so they can be read` : base;
  });
</script>

<div class="legend">
  <span>{text}</span>
  {#if view === '3d' && omitted > 0}
    <span class="omitted">
      {omitted === 1 ? '1 small item is' : `${omitted} small items are`} not drawn — see the list
    </span>
  {/if}
  {#if insideMarked}
    <span class="omitted">This folder is inside one that is marked for cleanup</span>
  {/if}
  <span class="sp"></span>
  <span><i class="k safe"></i>Safe to clean</span>
  <span><i class="k marked"></i>Marked for cleanup</span>
  <span><i class="k"></i>Selected</span>
</div>

<style>
  .legend {
    display: flex;
    gap: 20px;
    flex-wrap: wrap;
    align-items: center;
    font-size: 12px;
    color: var(--muted);
  }

  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }

  .sp {
    flex: 1;
  }

  .k {
    display: block;
    width: 16px;
    height: 11px;
    border-radius: 3px;
    border: 1.5px solid var(--strong);
    background: var(--ring);
  }

  /* Green means safe and nothing else; marked is the dashed outline the map draws. */
  .k.safe {
    border-color: var(--green);
    background: none;
  }

  .k.marked {
    border: 1.5px dashed var(--text);
    background: none;
  }
</style>
