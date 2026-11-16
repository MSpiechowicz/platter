<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '../../api';
  import { describeError } from '../../api/client';
  import PageHeader from '../../components/layout/PageHeader.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Loading from '../../components/ui/Loading.svelte';
  import Notice from '../../components/ui/Notice.svelte';
  import { hooks } from '../../hooks';
  import { resource } from '../../state/resource.svelte';
  import { scan } from '../../state/scan.svelte';
  import { ui } from '../../state/ui.svelte';
  import ExplorerPage from './ExplorerPage.svelte';
  import { resolveStart } from './state/explorer.svelte';

  interface Props {
    /**
     * The node to open on, instead of where the user was. Without it the Explorer opens on the folder last asked for
     * by the Overview or last open here (`ui`), if that is from this scan. A file opens its folder with the file selected.
     */
    focusId?: number;
  }

  let { focusId }: Props = $props();

  // Where to open: a folder opens as it is, a file opens on its folder. A stale answer never replaces a newer one.
  const opening = resource(async () => {
    const state = scan.state;

    if (state.status !== 'ready') {
      return null;
    }

    // Read once, without following it: the page keeps this place up to date while it is open, which must not reopen it.
    const place = untrack(() => ({
      focus: ui.explorerFocus,
      color: ui.explorerFocusColor,
      scan: ui.explorerScan,
    }));
    // Ids are only good for the scan they came from.
    const remembered = place.scan === undefined || place.scan === state.scanId ? place.focus : undefined;
    const wanted = focusId ?? remembered ?? hooks.focus ?? undefined;

    const start = await resolveStart(api, state.scanId, state.rootId, wanted);
    // The colour is only known for the folder that was remembered with it.
    const color = wanted !== undefined && wanted === remembered ? place.color : undefined;

    return {
      scanId: state.scanId,
      rootId: state.rootId,
      start: color !== undefined && start.focus === wanted ? { ...start, color } : start,
    };
  });
</script>

<div class="explorer">
  {#if opening.error}
    <PageHeader title="EXPLORER" />
    <Notice kind="error">The Explorer could not open: {describeError(opening.error)}</Notice>
  {:else if opening.data}
    {#key opening.data}
      <ExplorerPage scanId={opening.data.scanId} rootId={opening.data.rootId} start={opening.data.start} />
    {/key}
  {:else}
    <PageHeader title="EXPLORER" />
    <Card class="waiting" aria-busy="true">
      <Loading when={opening.loading} label="Opening the Explorer…" />
    </Card>
  {/if}
</div>

<style>
  .explorer :global(.waiting) {
    min-height: 420px;
  }
</style>
