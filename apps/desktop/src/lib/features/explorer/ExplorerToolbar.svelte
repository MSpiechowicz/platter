<script lang="ts">
  import Segmented from '../../components/ui/Segmented.svelte';
  import type { NodeId } from '../../api/types';
  import type { ExplorerView } from '../../state/settings.svelte';
  import type { Dominant } from './state/explorer.svelte';

  interface Props {
    /** Name of the folder "Back" goes to; `undefined` at the scan root, where Back is off. */
    parentName?: string;
    atRoot: boolean;
    /** "Whole disk" when the scan root is a volume, "Whole scan" for a folder. */
    wholeLabel: string;
    view: ExplorerView;
    /** One child holds most of the folder: said in place of the usual hint, as a link that opens it. */
    dominant?: Dominant;
    onopendominant?: (id: NodeId) => void;
    onback: () => void;
    onroot: () => void;
    onview: (view: ExplorerView) => void;
  }

  let { parentName, atRoot, wholeLabel, view, dominant, onopendominant, onback, onroot, onview }: Props =
    $props();

  const VIEWS = [
    { value: 'flat', label: 'Flat' },
    { value: '3d', label: '3D' },
  ] as const;
</script>

<div class="nav">
  <button type="button" class="btn" disabled={atRoot} onclick={onback}>
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M13 8H3M7 4L3 8l4 4" /></svg>
    <span>{parentName === undefined ? 'Back' : `Back to ${parentName}`}</span>
  </button>
  <button type="button" class="btn" disabled={atRoot} onclick={onroot}>
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 7.5L8 3l5.5 4.5M4 6.5V13h8V6.5" /></svg>
    <span>{wholeLabel}</span>
  </button>
  {#if dominant}
    <span class="hint">
      <button type="button" class="link" onclick={() => onopendominant?.(dominant.node.id)}>
        <code title={dominant.name}>{dominant.name}</code> holds {dominant.percent}% of this folder — open it
      </button>
    </span>
  {:else}
    <span class="hint">Click a folder to select it, click again to open it</span>
  {/if}
  <div class="view">
    <Segmented options={VIEWS} value={view} label="View" onchange={onview} />
  </div>
</div>

<style>
  .nav {
    display: flex;
    gap: 6px;
    align-items: center;
    container-type: inline-size;
  }

  /* The icon sits 10px from the left edge and the text ends 12px from the right: they look equal. */
  .btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px 6px 10px;
    font-size: 12.5px;
    line-height: 1.3;
    font-weight: 600;
    white-space: nowrap;
    border: 1px solid var(--line);
    border-radius: 7px;
    background: var(--card2);
    transition: border-color 0.15s;
  }

  .btn:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .btn:disabled {
    opacity: 0.4;
  }

  svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
    transform: translateY(-1px);
  }

  .hint {
    margin-left: auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
    color: var(--dim);
  }

  .link {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--accent);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .link:hover {
    color: var(--accent-hover);
  }

  .link code {
    font-family: var(--mono);
    font-weight: 600;
  }

  .view {
    margin-left: 12px;
  }

  .view :global(button) {
    padding: 6px 14px;
  }

  /* In a narrow card the two buttons and the switch need the room. */
  @container (max-width: 600px) {
    .hint {
      display: none;
    }

    .view {
      margin-left: auto;
    }
  }
</style>
