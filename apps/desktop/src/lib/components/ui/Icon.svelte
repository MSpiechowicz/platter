<script module lang="ts">
  export type IconName =
    | 'scan'
    | 'overview'
    | 'explorer'
    | 'cleanup'
    | 'advisor'
    | 'history'
    | 'settings'
    | 'collapse'
    | 'download'
    | 'info'
    | 'chevron'
    | 'check'
    | 'close'
    | 'disk'
    | 'refresh'
    | 'alert'
    | 'offline'
    | 'lock';
</script>

<script lang="ts">
  interface Props {
    name: IconName;
    size?: number;
    stroke?: number;
  }

  let { name, size = 16, stroke = 1.6 }: Props = $props();

  // The small glyphs are drawn on a 12 grid, the navigation set on a 16 grid.
  const grid = $derived(name === 'chevron' || name === 'check' || name === 'close' ? 12 : 16);
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 {grid} {grid}"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === 'scan'}
    <path
      d="M2 5.5v-2A1.5 1.5 0 0 1 3.5 2h2M10.5 2h2A1.5 1.5 0 0 1 14 3.5v2M14 10.5v2a1.5 1.5 0 0 1-1.5 1.5h-2M5.5 14h-2A1.5 1.5 0 0 1 2 12.5v-2"
    />
    <circle cx="8" cy="8" r="1.7" />
  {:else if name === 'overview'}
    <rect x="2" y="2" width="5" height="5" rx="1" />
    <rect x="9" y="2" width="5" height="5" rx="1" />
    <rect x="2" y="9" width="5" height="5" rx="1" />
    <rect x="9" y="9" width="5" height="5" rx="1" />
  {:else if name === 'explorer'}
    <path d="M2 4.5h4l1.5 1.5H14v6.5H2z" />
  {:else if name === 'cleanup'}
    <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5" />
  {:else if name === 'advisor'}
    <path d="M8 2a4 4 0 0 0-2.3 7.3V11h4.6V9.3A4 4 0 0 0 8 2zM6.3 13.5h3.4" />
  {:else if name === 'history'}
    <circle cx="8" cy="8" r="6" />
    <path d="M8 4.6V8l2.3 1.5" />
  {:else if name === 'settings'}
    <path d="M2 5h7M12 5h2M2 11h2M7 11h7" />
    <circle cx="10.5" cy="5" r="1.5" />
    <circle cx="5.5" cy="11" r="1.5" />
  {:else if name === 'collapse'}
    <rect x="2" y="3" width="12" height="10" rx="2" />
    <path d="M6 3v10M11 6.5L9.5 8 11 9.5" />
  {:else if name === 'download'}
    <path d="M8 2.5v7.5M4.8 7L8 10.2 11.2 7M3 13h10" />
  {:else if name === 'info'}
    <circle cx="8" cy="8" r="6.2" />
    <path d="M8 7.3v3.6M8 5v.2" />
  {:else if name === 'chevron'}
    <path d="M2.5 4.5L6 8l3.5-3.5" />
  {:else if name === 'check'}
    <path d="M2.5 6.5l2.5 2.5 4.5-5" />
  {:else if name === 'close'}
    <path d="M3 3l6 6M9 3l-6 6" />
  {:else if name === 'disk'}
    <rect x="2" y="8.5" width="12" height="4.5" rx="1.5" />
    <path d="M3.2 8.5L4.8 3.5h6.4l1.6 5M11.5 10.75h.01" />
  {:else if name === 'refresh'}
    <path d="M13 8a5 5 0 1 1-1.5-3.5M13 2.5v3h-3" />
  {:else if name === 'alert'}
    <circle cx="8" cy="8" r="6.2" />
    <path d="M8 4.8v3.8M8 11v.2" />
  {:else if name === 'offline'}
    <path
      d="M2 8.2a8 8 0 0 1 3-1.8M14 8.2a8 8 0 0 0-3.4-1.9M4.6 10.4a4.5 4.5 0 0 1 2-1.1M11.4 10.4a4.5 4.5 0 0 0-1.9-1.1M8 13h.01M2.5 2.5l11 11"
    />
  {:else if name === 'lock'}
    <rect x="2.75" y="7" width="10.5" height="7.25" rx="2" />
    <path d="M5 7V5a3 3 0 0 1 6 0v2" />
    <circle cx="8" cy="10.6" r="1" fill="currentColor" stroke="none" />
  {/if}
</svg>

<style>
  .icon {
    flex: none;
  }
</style>
