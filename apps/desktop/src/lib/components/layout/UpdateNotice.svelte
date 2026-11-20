<script lang="ts">
  import { untrack } from 'svelte';
  import { tooltip } from '../ui/tooltip.svelte';
  import type { UpdateAction, UpdateInfo, UpdatePhase } from '../../state/update.svelte';
  import Bar from '../ui/Bar.svelte';
  import Button from '../ui/Button.svelte';
  import Icon from '../ui/Icon.svelte';

  interface Props {
    info: UpdateInfo;
    phase: UpdatePhase;
    /** Download progress, 0 to 1. */
    progress: number;
    /** The download has no known size, or the install is running: the bar slides instead of filling. */
    indeterminate?: boolean;
    /** The sidebar is the 60 px rail: only a download button with a dot is shown. */
    collapsed: boolean;
    onaction: (action: UpdateAction) => void;
  }

  let { info, phase, progress, indeterminate = false, collapsed, onaction }: Props = $props();

  /** How long the full bar stays after the download or install is done, before "Restart to finish" replaces it. */
  const FULL_BAR_HOLD_MS = 700;

  // The phase on screen. It follows `phase`, except that the progress stays a moment after it ends, so the bar
  // is seen to reach the end instead of the button cutting it off.
  let view = $state<UpdatePhase>(untrack(() => phase));

  $effect.pre(() => {
    const next = phase;
    const shown = untrack(() => view);
    const busy = shown === 'downloading' || shown === 'installing';

    if (next !== 'ready' || !busy) {
      view = next;

      return;
    }

    const timer = setTimeout(() => (view = next), FULL_BAR_HOLD_MS);

    return () => clearTimeout(timer);
  });

  /** What the main button does: install the update here, or open the release page. */
  const mainAction = $derived<UpdateAction>(info.canInstall ? 'install' : 'open');
  const railLabel = $derived(info.canInstall ? `Update to Platter ${info.next}` : `Get Platter ${info.next}`);

  const notes = $derived({
    title: `New in ${info.next}`,
    items: info.notes,
    note: `You have ${info.current} · checked ${info.checked}`,
  });
</script>

<div class="upd" class:mini={collapsed}>
  {#if collapsed}
    <button
      type="button"
      class="ubtn"
      title={railLabel}
      aria-label={railLabel}
      onclick={() => onaction(mainAction)}
    >
      <Icon name="download" />
      <i></i>
    </button>
  {:else}
    <div class="ut">
      <i></i>
      <b>{view === 'ready' ? 'Update ready' : 'Update available'}</b>
      <button type="button" class="info" aria-label="What is new" use:tooltip={notes}>
        <Icon name="info" size={15} stroke={1.5} />
      </button>
    </div>
    <div class="uv">Platter {info.next}{info.size ? ` · ${info.size}` : ''}</div>

    <!-- Every state takes the height of the button row, so the card does not change size between them. -->
    <div class="act">
      {#if view === 'available'}
        <div class="ub">
          <Button variant="primary" size="sm" onclick={() => onaction(mainAction)}>
            {info.canInstall ? 'Install' : 'Get it'}
          </Button>
          <Button size="sm" onclick={() => onaction('later')}>Later</Button>
        </div>
      {:else if view === 'downloading' || view === 'installing'}
        {@const what = view === 'downloading' ? 'Downloading' : 'Installing'}
        <!-- Installing follows a finished download: the bar stays full instead of sliding back to a segment. -->
        {@const done = view === 'installing' || phase === 'ready'}
        <div class="progress">
          <span>{what} {info.next}…</span>
          <Bar
            value={done ? 1 : progress}
            indeterminate={indeterminate && !done}
            label="{what} {info.next}"
          />
        </div>
      {:else}
        <div class="ub one">
          <Button variant="primary" size="sm" onclick={() => onaction('restart')}>Restart to finish</Button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .upd {
    border: 1px solid var(--line);
    border-radius: 9px;
    background: var(--card);
    padding: 11px 12px;
    font-size: 12px;
    color: var(--muted);
  }

  .upd.mini {
    border: 0;
    background: none;
    padding: 0;
  }

  .ut {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
    font-weight: 600;
    font-size: 12.5px;
    white-space: nowrap;
  }

  .ut i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    flex: none;
  }

  .uv {
    margin: 3px 0 10px;
    white-space: nowrap;
  }

  .ub {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }

  .ub.one {
    grid-template-columns: 1fr;
  }

  /* The height of a small button: its 12px text line, 5px padding and 1px border on each side. */
  .act {
    height: calc(12px * 1.45 + 12px);
  }

  .progress {
    height: 100%;
    display: grid;
    align-content: center;
    gap: 6px;
  }

  /* Progress comes in often: a short, even motion keeps up with it instead of easing far behind. */
  .progress :global(.bar i) {
    transition: width 0.3s linear;
  }

  .info {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    margin-left: -5px;
    border-radius: 50%;
    color: var(--muted);
  }

  .info:hover {
    color: var(--text);
  }

  .ubtn {
    position: relative;
    width: 100%;
    height: 34px;
    border-radius: 7px;
    color: var(--text);
    display: grid;
    place-items: center;
  }

  .ubtn:hover {
    background: var(--wash-3);
  }

  .ubtn i {
    position: absolute;
    right: 4px;
    top: 5px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 0 2px var(--side-bg);
  }
</style>
