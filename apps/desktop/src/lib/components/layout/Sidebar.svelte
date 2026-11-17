<script module lang="ts">
  import type { IconName } from '../ui/Icon.svelte';

  export interface NavItem {
    id: string;
    label: string;
    icon: IconName;
    /** Dimmed and not clickable: it has nothing to show yet. `hint` says why. */
    disabled?: boolean;
    hint?: string;
  }
</script>

<script lang="ts">
  import { baseName } from '../../format';
  import type { VolumeDto } from '../../api/types';
  import { afterPaint } from '../../motion/afterPaint';
  import { fmt } from '../../state/fmt.svelte';
  import type { UpdateAction, UpdateInfo, UpdatePhase } from '../../state/update.svelte';
  import Bar from '../ui/Bar.svelte';
  import Icon from '../ui/Icon.svelte';
  import Logo from '../ui/Logo.svelte';
  import UpdateNotice from './UpdateNotice.svelte';

  interface Props {
    items: readonly NavItem[];
    active: string;
    onselect: (id: string) => void;
    /** 60 px icon rail: icons and logo keep their place, labels fade out. */
    collapsed: boolean;
    /** Omit to hide the "Collapse menu" item (the window is too narrow to expand). */
    ontoggle?: () => void;
    /** The volume that was scanned. Nothing is shown at the bottom when there is none. */
    volume?: VolumeDto;
    /** Memory the advisor model holds, while it is loaded; the row is hidden otherwise. */
    advisorBytes?: number;
    update?: UpdateInfo | null;
    updatePhase?: UpdatePhase;
    updateProgress?: number;
    updateIndeterminate?: boolean;
    onupdate?: (action: UpdateAction) => void;
  }

  let {
    items,
    active,
    onselect,
    collapsed,
    ontoggle,
    volume,
    advisorBytes,
    update = null,
    updatePhase = 'available',
    updateProgress = 0,
    updateIndeterminate = false,
    onupdate = () => {},
  }: Props = $props();

  // The highlight slides to the active item. It is placed without animation the first time.
  let buttons: Record<string, HTMLButtonElement | undefined> = $state({});
  let top = $state(0);
  let height = $state(0);
  let placed = $state(false);
  let slideVisible = $state(false);

  $effect(() => {
    const button = buttons[active];

    slideVisible = Boolean(button);

    if (!button) {
      return;
    }

    top = button.offsetTop;
    height = button.offsetHeight;

    if (!placed) {
      return afterPaint(() => (placed = true));
    }
  });

  function titleOf(item: NavItem, rail: boolean): string | undefined {
    if (item.disabled && item.hint) {
      return rail ? `${item.label}: ${item.hint}` : item.hint;
    }

    return rail ? item.label : undefined;
  }

  const used = $derived(volume ? volume.total - volume.available : 0);
  const share = $derived(volume && volume.total > 0 ? used / volume.total : 0);
</script>

<aside class="side" class:mini={collapsed}>
  {#if slideVisible}
    <i class="slide" class:placed style="height:{height}px;transform:translateY({top}px)"></i>
  {/if}

  <div class="brand">
    <Logo />
    <span>PLATTER</span>
  </div>

  <nav aria-label="Main">
    {#each items as item (item.id)}
      <!-- aria-disabled, not disabled: a disabled button swallows the hover that shows the hint. It is
           kept out of the tab order, and the click does nothing. -->
      <button
        type="button"
        class="nav"
        class:on={item.id === active}
        class:off={item.disabled}
        aria-current={item.id === active ? 'page' : undefined}
        aria-disabled={item.disabled ? 'true' : undefined}
        tabindex={item.disabled ? -1 : undefined}
        title={titleOf(item, collapsed)}
        bind:this={buttons[item.id]}
        onclick={() => {
          if (!item.disabled) {
            onselect(item.id);
          }
        }}
      >
        <Icon name={item.icon} />
        <span>{item.label}</span>
      </button>
    {/each}
  </nav>

  {#if ontoggle}
    <button type="button" class="sfold" title={collapsed ? 'Expand menu' : undefined} onclick={ontoggle}>
      <Icon name="collapse" />
      <span>{collapsed ? 'Expand menu' : 'Collapse menu'}</span>
    </button>
  {/if}

  <div class="bottom">
    {#if update}
      <UpdateNotice
        info={update}
        phase={updatePhase}
        progress={updateProgress}
        indeterminate={updateIndeterminate}
        {collapsed}
        onaction={onupdate}
      />
    {/if}

    {#if volume || (advisorBytes !== undefined && !collapsed)}
      <div class="foot">
        {#if volume && collapsed}
          <div
            class="rail"
            title="{baseName(volume.device)}: {fmt.size(used)} of {fmt.size(volume.total)} used"
          >
            <b class="num">{Math.round(share * 100)}%</b>
            <Bar value={share} label="Disk used" />
          </div>
        {:else if volume}
          <div class="kvr"><b>{baseName(volume.device)}</b><span>{volume.fsType}</span></div>
          <div class="usage"><Bar value={share} label="Disk used" /></div>
          <div class="kvr"><span>{fmt.size(used)} of {fmt.size(volume.total)}</span></div>
        {/if}

        {#if advisorBytes !== undefined && !collapsed}
          <div class="kvr model" title="Memory the advisor model holds">
            <span>Advisor model</span>
            <span class="num">{fmt.size(advisorBytes)}</span>
          </div>
        {/if}
      </div>
    {/if}
  </div>
</aside>

<style>
  .side {
    position: sticky;
    top: 0;
    height: 100vh;
    border-right: 1px solid var(--line);
    background: var(--side-bg);
    padding: 18px 12px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow: hidden;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    padding: 4px 5px 18px;
    white-space: nowrap;
  }

  nav {
    display: contents;
  }

  .nav,
  .sfold {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: 7px;
    color: var(--muted);
    text-align: left;
    width: 100%;
    position: relative;
    z-index: 1;
  }

  .nav {
    transition: color 0.15s;
  }

  .nav:not(.off):hover,
  .nav.on,
  .sfold:hover {
    color: var(--text);
  }

  .nav.off {
    color: var(--dim);
    opacity: 0.55;
    cursor: default;
  }

  .nav.on :global(svg) {
    color: var(--accent);
  }

  .slide {
    position: absolute;
    left: 12px;
    right: 12px;
    top: 0;
    border-radius: 7px;
    background: #ffffff10;
    z-index: 0;
  }

  .slide.placed {
    transition: transform 0.28s cubic-bezier(0.3, 0.8, 0.3, 1);
  }

  .sfold :global(svg) {
    transition: transform 0.25s;
  }

  .mini .sfold :global(svg) {
    transform: scaleX(-1);
  }

  .brand span,
  .nav span,
  .sfold span {
    white-space: nowrap;
    transition: opacity 0.15s;
  }

  .mini .brand span,
  .mini .nav span,
  .mini .sfold span {
    opacity: 0;
  }

  .bottom {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .foot {
    font-size: 12px;
    color: var(--muted);
    border-top: 1px solid var(--line);
    padding: 12px 8px 0;
  }

  .mini .foot {
    padding: 12px 0 0;
    text-align: center;
  }

  .kvr {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    white-space: nowrap;
  }

  .foot b {
    color: var(--text);
    font-weight: 600;
  }

  .usage {
    margin: 7px 0 5px;
  }

  .model {
    margin-top: 10px;
  }

  .foot > .model:first-child {
    margin-top: 0;
  }

  .rail :global(.bar) {
    margin: 7px 0 5px;
  }
</style>
