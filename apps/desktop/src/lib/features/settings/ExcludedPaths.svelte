<script lang="ts">
  import { api } from '../../api';
  import { describeError } from '../../api/client';
  import { MAX_EXCLUDED } from '../../api/types';
  import Button from '../../components/ui/Button.svelte';
  import Field from '../../components/ui/Field.svelte';
  import Icon from '../../components/ui/Icon.svelte';
  import { shownPath, visibleText } from '../overview/display';
  import { cleanExcluded, settings } from '../../state/settings.svelte';
  import { ui } from '../../state/ui.svelte';

  let picking = $state(false);

  const full = $derived(settings.excludedPaths.length >= MAX_EXCLUDED);

  function saved(): void {
    ui.toast(
      'Saved',
      'A new scan from the Scan page leaves these folders out. Rescan keeps the list of its scan.',
    );
  }

  async function add(): Promise<void> {
    if (picking || full) {
      return;
    }

    picking = true;

    try {
      const path = await api.pickExcludedFolder();

      if (path === null || settings.excludedPaths.includes(path)) {
        return;
      }

      settings.set('excludedPaths', cleanExcluded([...settings.excludedPaths, path]));
      saved();
    } catch (reason) {
      ui.toast('The folder cannot be left out', describeError(reason), 'error');
    } finally {
      picking = false;
    }
  }

  function remove(path: string): void {
    settings.set(
      'excludedPaths',
      settings.excludedPaths.filter((entry) => entry !== path),
    );
    saved();
  }
</script>

<Field label="Excluded paths" description="Never read, counted or cleaned. Applies to new scans.">
  {#snippet control()}
    <Button disabled={picking || full} onclick={add}>
      {full ? `At most ${MAX_EXCLUDED} folders` : 'Add folder…'}
    </Button>
  {/snippet}
</Field>

{#if settings.excludedPaths.length > 0}
  <ul class="chips" aria-label="Folders left out of scans">
    {#each settings.excludedPaths as path (path)}
      <li class="chip">
        <span class="path" title={visibleText(path)}>{shownPath(path)}</span>
        <button
          type="button"
          class="remove"
          aria-label="Stop leaving out {visibleText(path)}"
          onclick={() => remove(path)}
        >
          <Icon name="close" size={12} />
        </button>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    list-style: none;
    margin: 0;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    padding: 4px 6px 4px 9px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--wash-2);
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text);
  }

  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .remove {
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border-radius: 4px;
    color: var(--muted);
  }

  .remove:hover {
    background: var(--hover);
    color: var(--text);
  }
</style>
