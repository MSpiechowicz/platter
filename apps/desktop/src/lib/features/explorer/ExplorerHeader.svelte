<script lang="ts">
  import type { NodeId } from '../../api/types';
  import type { Crumb } from './state/explorer.svelte';

  interface Props {
    crumbs: readonly Crumb[];
    /** The size of the open folder, already formatted. */
    size: string;
    /** The open folder's colour, shown as a small dot on the last crumb; none at the scan root. */
    color?: string | null;
    onopen: (id: NodeId) => void;
  }

  let { crumbs, size, color = null, onopen }: Props = $props();

  // The root crumb shows the whole path of the scan root ("/", "/home/demo"): its name alone ("demo") would not
  // say which folder was scanned. The others are names, and every crumb has its full path as a tooltip.
</script>

<header class="top">
  <div>
    <h1>EXPLORER</h1>
    <nav class="crumbs" aria-label="Path">
      {#each crumbs as crumb, index (crumb.id)}
        {#if index > 0}<span class="sep" aria-hidden="true">›</span>{/if}
        <button
          type="button"
          aria-current={index === crumbs.length - 1 ? 'page' : undefined}
          title={crumb.path}
          onclick={() => index < crumbs.length - 1 && onopen(crumb.id)}
        >
          {#if color && index === crumbs.length - 1}<i class="dot" style="--c:{color}" aria-hidden="true"
            ></i>{/if}
          {index === 0 ? crumb.path : crumb.name}
        </button>
      {/each}
      {#if size}<span class="size num">{size}</span>{/if}
    </nav>
  </div>
</header>

<style>
  /* The same header as <PageHeader>, with the path under the title. */
  .top {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: 18px;
    flex-wrap: wrap;
  }

  h1 {
    font-family: var(--mono);
    font-size: 17px;
    letter-spacing: 0.1em;
    font-weight: 700;
  }

  .crumbs {
    display: flex;
    gap: 6px;
    align-items: center;
    font-family: var(--mono);
    font-size: 13px;
    flex-wrap: wrap;
  }

  .crumbs button {
    color: var(--cyan);
    padding: 3px 7px;
    border-radius: 6px;
  }

  .crumbs button:hover {
    background: var(--accent-soft);
  }

  .crumbs button[aria-current='page'] {
    cursor: default;
  }

  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 7px;
    border-radius: 50%;
    background: var(--c);
  }

  .crumbs button[aria-current='page']:hover {
    background: none;
  }

  .sep,
  .size {
    color: var(--muted);
  }

  .size {
    margin-left: 8px;
  }
</style>
