<script lang="ts">
  import { api } from '../../api';
  import { baseName } from '../../format';
  import PageHeader from '../../components/layout/PageHeader.svelte';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Field from '../../components/ui/Field.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import Notice from '../../components/ui/Notice.svelte';
  import Toggle from '../../components/ui/Toggle.svelte';
  import { scan } from '../../state/scan.svelte';
  import { settings } from '../../state/settings.svelte';
  import ScanInfoPanel from '../scan/ScanInfoPanel.svelte';
  import TargetRow from './TargetRow.svelte';
  import { groupVolumes } from './volumes';

  /** The folder the user picked, shown as an extra row. */
  let folder = $state<string | null>(null);
  /** The selected row: a volume by its id (mount points repeat), or the picked folder. */
  let choice = $state<{ kind: 'volume'; id: string } | { kind: 'folder' } | null>(null);

  const groups = $derived(groupVolumes(scan.volumes));

  /** What will be scanned: the mount point of the chosen volume, or the folder. */
  const target = $derived.by(() => {
    if (choice?.kind === 'folder') {
      return folder;
    }

    const chosen = choice;

    return chosen ? (groups.find((group) => group.volume.id === chosen.id)?.volume.mountPoint ?? null) : null;
  });

  // Pre-select the first volume once the list is there.
  $effect(() => {
    const first = groups[0];

    if (choice === null && first) {
      choice = { kind: 'volume', id: first.volume.id };
    }
  });

  async function chooseFolder(): Promise<void> {
    const picked = await api.pickFolder();

    if (picked !== null) {
      folder = picked;
      choice = { kind: 'folder' };
    }
  }

  function startScan(): void {
    if (target !== null) {
      scan.start({
        root: target,
        oneFileSystem: settings.oneFileSystem,
        excluded: [...settings.excludedPaths],
      });
    }
  }

  const previous = $derived(
    scan.state.status === 'cancelled' || scan.state.status === 'failed' ? scan.state : null,
  );
</script>

<PageHeader title="NEW SCAN" sub="pick a disk or a folder to analyse" />

<div class="wrap">
  <Card title="Where to scan" class="pick">
    {#if previous?.status === 'cancelled'}
      <div class="notice"><Notice>The last scan was cancelled. Nothing was kept.</Notice></div>
    {:else if previous?.status === 'failed'}
      <div class="notice"><Notice kind="error">The last scan failed: {previous.message}</Notice></div>
    {/if}

    {#if scan.volumesStatus === 'error'}
      <div class="notice">
        <Notice kind="error">The list of disks could not be read. You can still choose a folder.</Notice>
      </div>
    {/if}

    <div class="rows" role="group" aria-label="Where to scan">
      <Loading when={scan.volumesStatus === 'loading'} label="Reading disks…" />

      <!-- The index is part of the key: two volumes with the same id must not break the page. -->
      {#each groups as { volume, alsoMountedAt }, index (`${volume.id}:${index}`)}
        <TargetRow
          path={volume.mountPoint}
          {alsoMountedAt}
          detail={[volume.label, baseName(volume.device), volume.fsType].filter(Boolean).join(' · ')}
          removable={volume.removable}
          usage={{ used: volume.total - volume.available, total: volume.total }}
          selected={choice?.kind === 'volume' && choice.id === volume.id}
          onselect={() => (choice = { kind: 'volume', id: volume.id })}
        />
      {/each}

      {#if folder !== null}
        <TargetRow
          path={folder}
          detail="Folder"
          selected={choice?.kind === 'folder'}
          onselect={() => (choice = { kind: 'folder' })}
        />
      {/if}
    </div>

    <div class="choose">
      <Button onclick={chooseFolder}>Choose a folder…</Button>
    </div>

    <Field
      label="Stay on this disk"
      description="Skip other disks, network shares and removable drives mounted inside it."
    >
      {#snippet control(labelId, descriptionId)}
        <Toggle
          checked={settings.oneFileSystem}
          aria-labelledby={labelId}
          aria-describedby={descriptionId}
          onchange={(event) => settings.set('oneFileSystem', event.currentTarget.checked)}
        />
      {/snippet}
    </Field>

    <div class="actions">
      <Button variant="primary" disabled={target === null} onclick={startScan}>Scan</Button>
    </div>
  </Card>

  <ScanInfoPanel root={target} />
</div>

<style>
  .wrap {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 420px;
    gap: 16px;
    align-items: stretch;
  }

  .notice {
    margin-bottom: 12px;
  }

  /* Six rows at most (a row is about 74 px, plus the 8 px gap and the padding), then the list scrolls inside the card so the Scan button stays on screen.
     The padding leaves room for the focus ring of the rows, which the scroll area would clip. The scroll bar sits in
     the card's padding (16 px out on the right) and never changes the rows: they are as wide as the card's content
     (`cqw`) whether or not the list scrolls, and whatever the engine does with scroll bars. */
  .rows {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    grid-template-columns: 100cqw;
    gap: 8px;
    min-height: 88px;
    max-height: 516px;
    overflow-x: hidden;
    overflow-y: auto;
    padding: 4px 0 4px 4px;
    margin: -4px -16px 0 -4px;
  }

  /* The card is the box `cqw` is measured against. */
  .wrap :global(.pick) {
    container-type: inline-size;
  }

  .choose {
    margin: 12px 0 16px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    margin-top: auto;
    padding-top: 16px;
    border-top: 1px solid var(--line);
  }

  .actions :global(.btn) {
    min-width: 96px;
  }

  @media (max-width: 1100px) {
    .wrap {
      grid-template-columns: 1fr;
    }
  }
</style>
