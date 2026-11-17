<script lang="ts">
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
      <b>{phase === 'ready' ? 'Update ready' : 'Update available'}</b>
      <button type="button" class="info" aria-label="What is new" use:tooltip={notes}>
        <Icon name="info" size={15} stroke={1.5} />
      </button>
    </div>
    <div class="uv">Platter {info.next}{info.size ? ` · ${info.size}` : ''}</div>

    {#if phase === 'available'}
      <div class="ub">
        <Button variant="primary" size="sm" onclick={() => onaction(mainAction)}>
          {info.canInstall ? 'Install' : 'Get it'}
        </Button>
        <Button size="sm" onclick={() => onaction('later')}>Later</Button>
      </div>
    {:else if phase === 'downloading' || phase === 'installing'}
      {@const what = phase === 'downloading' ? 'Downloading' : 'Installing'}
      <div class="progress">
        <Bar
          value={progress}
          indeterminate={indeterminate || phase === 'installing'}
          label="{what} {info.next}"
        />
        <span>{what} {info.next}…</span>
      </div>
    {:else}
      <div class="ub one">
        <Button variant="primary" size="sm" onclick={() => onaction('restart')}>Restart to finish</Button>
      </div>
    {/if}
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

  .progress {
    display: grid;
    gap: 4px;
  }

  .progress :global(.bar) {
    margin: 8px 0 0;
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
