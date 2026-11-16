<script lang="ts">
  import { describeError } from '../../api/client';
  import type { NodeId, ScanId } from '../../api/types';
  import Button from '../../components/ui/Button.svelte';
  import Card from '../../components/ui/Card.svelte';
  import Notice from '../../components/ui/Notice.svelte';
  import Spinner from '../../components/ui/Spinner.svelte';
  import { hooks } from '../../hooks';
  import { delayed } from '../../state/delayed.svelte';
  import { fmt } from '../../state/fmt.svelte';
  import { scan } from '../../state/scan.svelte';
  import { settings, type ExplorerView } from '../../state/settings.svelte';
  import { ui } from '../../state/ui.svelte';
  import { baseName } from '../../format';
  import ChildList from './ChildList.svelte';
  import ColumnsView from './ColumnsView.svelte';
  import DetailsCard from './DetailsCard.svelte';
  import ExplorerHeader from './ExplorerHeader.svelte';
  import ExplorerLegend from './ExplorerLegend.svelte';
  import ExplorerToolbar from './ExplorerToolbar.svelte';
  import FlatMap from './FlatMap.svelte';
  import MapMessage from './MapMessage.svelte';
  import { layoutColumns, MAX_COLUMNS } from './layout/columns';
  import { linksOf } from './layout/chains';
  import { childPath } from './path';
  import { ExplorerStore, type ExplorerStart } from './state/explorer.svelte';

  interface Props {
    scanId: ScanId;
    rootId: NodeId;
    start: ExplorerStart;
  }

  let { scanId, rootId, start }: Props = $props();

  // One Explorer per scan and start; the parent re-creates this page when either changes.
  // svelte-ignore state_referenced_locally
  const store = new ExplorerStore(scanId, rootId, start);

  // Every folder shown is remembered, so that leaving the Explorer and coming back finds the user where they were.
  $effect(() => {
    const focus = store.shown?.focus;

    if (focus !== undefined) {
      ui.rememberExplorer(scanId, focus, store.focusColor ?? undefined);
    }
  });

  let view = $state<ExplorerView>(hooks.ex ?? settings.explorerView);
  let notice = $state<string | null>(null);

  const wholeLabel = $derived(scan.volumeInfo?.exact ? 'Whole disk' : 'Whole scan');
  const shown = $derived(store.shown);
  const parentName = $derived.by(() => {
    const parent = store.parent;

    return parent && store.crumbs.length > 1 ? baseName(parent.name) : undefined;
  });
  // Only the 3D view needs the columns; the legend tells how many items they leave out.
  const model = $derived(store.model);
  const columns = $derived(
    model && view === '3d' ? layoutColumns(model, MAX_COLUMNS, store.dominantTile ?? null) : undefined,
  );
  // Whether the flat map draws some tile larger than it is (the map measures itself, so it says so).
  let flatBoosted = $state(false);
  const boosted = $derived(view === 'flat' ? flatBoosted : (columns?.boosted ?? false));
  const waiting = delayed(() => store.frame.loading);
  const reading = $derived(`Reading ${store.opening ?? store.node?.name ?? 'the folder'}…`);

  // What the details card is about: the selected child (a chain tile: its last folder, the one "Open folder" opens),
  // or else the open folder. The path, the numbers and both actions all refer to this one folder.
  const detail = $derived(store.selectedTarget ?? store.node);
  const detailPath = $derived.by(() => {
    const here = shown?.path.path ?? '';
    const chosen = store.selectedNode;

    // The "smaller items" tile is a set of items, not a place: the details show the open folder's path.
    if (!chosen || store.groupSelected) {
      return here;
    }

    // A chain tile ("home / sam") is the whole path down to the folder it opens.
    const tile = model?.children.find((child) => child.node.id === chosen.id);

    return linksOf(tile ?? { node: chosen }).reduce((path, link) => childPath(path, link.name), here);
  });
  // How much of the open folder the selected child holds, whatever the map draws.
  const detailShare = $derived(
    store.selectedTarget && !store.groupSelected ? store.shareOf(store.selectedTarget) : undefined,
  );
  const detailColor = $derived(
    store.selectedNode && !store.groupSelected ? store.colorOf(store.selectedNode.id) : store.focusColor,
  );

  function setView(next: ExplorerView): void {
    view = next;
    notice = null;
    settings.set('explorerView', next);
  }

  function fallBack(message: string): void {
    // Only for this visit: the saved choice stays, so a working machine is not punished for one failure.
    view = 'flat';
    notice = message;
  }

  async function reveal(): Promise<void> {
    try {
      await store.reveal();
    } catch (error) {
      ui.toast('Could not open the file manager', describeError(error), 'error');
    }
  }

  // The `?pick=n` test hook: select the n-th biggest child once the folder is on screen.
  let picked = false;

  $effect(() => {
    const top = shown?.lod.children;

    if (picked || hooks.pick === null || !top) {
      return;
    }

    picked = true;

    const child = top[hooks.pick]?.node;

    if (child) {
      store.select(child.id);
    }
  });

  let list: ChildList | undefined = $state();

  const FIELDS = 'input, textarea, select, [contenteditable="true"]';

  function onKeydown(event: KeyboardEvent): void {
    const target = event.target as HTMLElement | null;
    const inField = target?.matches(FIELDS) ?? false;
    const inSearch = target?.matches('[data-explorer-search]') ?? false;

    if (event.defaultPrevented || event.altKey || document.querySelector('dialog[open]')) {
      return;
    }

    // "/" and Ctrl+F go to the search field, unless something else is being typed in.
    const find =
      (event.key === '/' && !event.ctrlKey && !event.metaKey) ||
      (event.key.toLowerCase() === 'f' && (event.ctrlKey || event.metaKey));

    if (find && (!inField || inSearch)) {
      if (inSearch && event.key === '/') {
        return;
      }

      event.preventDefault();
      list?.focusSearch();

      return;
    }

    if (event.key !== 'Backspace' && event.key !== 'Escape') {
      return;
    }

    // In the search field Backspace deletes, and Escape clears it first; only an Escape with nothing to clear goes up.
    const emptySearch = inSearch && event.key === 'Escape' && (target as HTMLInputElement).value === '';

    if (event.ctrlKey || event.metaKey || (inField && !emptySearch)) {
      return;
    }

    event.preventDefault();
    store.up();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<ExplorerHeader
  crumbs={store.crumbs}
  size={store.node ? fmt.size(store.node.allocated) : ''}
  color={store.focusColor}
  onopen={(id) => store.open(id)}
/>

<div class="wrap">
  <Card class="stage">
    <ExplorerToolbar
      {parentName}
      atRoot={store.focus === rootId}
      {wholeLabel}
      {view}
      dominant={store.dominant}
      onopendominant={(id) => store.open(id)}
      onback={() => store.up()}
      onroot={() => store.toRoot()}
      onview={setView}
    />

    <div class="mapbox" aria-busy={store.frame.loading}>
      {#if store.frame.error}
        <MapMessage title="This folder could not be read" icon="alert" kind="error">
          {describeError(store.frame.error)}
          <Button size="sm" onclick={() => store.retry()}>Try again</Button>
        </MapMessage>
      {:else if shown && shown.lod.children.length === 0}
        <MapMessage title="Nothing to show here">
          {#if shown.lod.node.kind === 'mount'}
            This is another disk or a system folder, so it was not read.
          {:else}
            The folder is empty, or Platter was not allowed to read it.
          {/if}
        </MapMessage>
      {:else if shown && model && view === 'flat'}
        <FlatMap
          frame={shown}
          {model}
          onboost={(value) => (flatBoosted = value)}
          colors={store.colors}
          selected={store.selected}
          hovered={store.hovered}
          onchoose={(id) => store.choose(id)}
          onhover={(id) => store.hover(id)}
          onclear={() => store.select(null)}
          ongroup={(node, members) => store.chooseGroup(node, members)}
        />
      {:else if shown && columns}
        <ColumnsView
          layout={columns}
          colors={store.colors}
          selected={store.selected}
          hovered={store.hovered}
          onchoose={(id) => store.choose(id)}
          onhover={(id) => store.hover(id)}
          onclear={() => store.select(null)}
          ongroup={(node, members) => store.chooseGroup(node, members)}
          onfail={fallBack}
        />
      {/if}

      {#if waiting.value}
        <div class="veil"><Spinner label={reading} /></div>
      {/if}
    </div>

    {#if notice}<Notice>{notice}</Notice>{/if}
    <ExplorerLegend {view} omitted={columns?.omitted ?? 0} {boosted} />
  </Card>

  <div class="side">
    <div class="inner">
      <DetailsCard
        node={detail}
        selected={store.selectedNode !== undefined}
        path={detailPath}
        share={detailShare}
        color={detailColor}
        canOpen={store.selectedTarget?.hasChildren === true}
        onopen={() => store.openSelected()}
        onreveal={reveal}
      />
      <ChildList
        folder={store.focus}
        items={store.sizedItems}
        total={store.allSizedTotal}
        matched={store.matchedTotal}
        uncounted={store.uncountedItems}
        uncountedTotal={store.uncountedTotal}
        sort={store.sort}
        selected={store.selected}
        highlighted={store.highlighted}
        wantMore={store.groupNeedsRows}
        query={store.query}
        groupTruncated={store.groupTruncated}
        allTotal={store.total}
        loaded={store.items.length}
        onquery={(query) => store.setQuery(query)}
        bind:this={list}
        hovered={store.hovered}
        colorOf={(id) => store.colorOf(id)}
        loading={store.page.loading}
        error={store.page.error ? describeError(store.page.error) : undefined}
        hasMore={store.hasMore}
        loadingMore={store.loadingMore}
        moreDisabled={!store.pageCurrent}
        moreError={store.moreError}
        onsort={(sort) => store.setSort(sort)}
        onchoose={(id) => store.choose(id)}
        onhover={(id) => store.hover(id)}
        onmore={() => store.loadMore()}
        onretry={() => store.retry()}
      />
    </div>
  </div>
</div>

<style>
  .wrap {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 320px;
    gap: 16px;
    /* Fill the window, but never get shorter than the map and the list need. */
    min-height: max(680px, calc(100vh - 110px));
  }

  .wrap :global(.stage) {
    gap: 12px;
  }

  .mapbox {
    flex: 1;
    min-height: 520px;
    position: relative;
    overflow: hidden;
    border-radius: 6px;
  }

  /* While the next folder loads, the old map stays under a veil and the spinner. */
  .veil {
    position: absolute;
    inset: 0;
    background: #111723b3;
  }

  /*
   * The right column is as tall as the map card and never taller: its content is taken out of the flow,
   * so a long list scrolls inside its card instead of stretching the page.
   */
  .side {
    position: relative;
    min-width: 0;
  }

  .inner {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .inner :global(.child-list) {
    min-height: 300px;
  }

  @media (max-width: 1000px) {
    .wrap {
      grid-template-columns: 1fr;
      min-height: 0;
    }

    .wrap :global(.stage) {
      min-height: 520px;
    }

    .mapbox {
      min-height: 400px;
    }

    .inner {
      position: static;
    }

    .inner :global(.child-list) {
      height: 420px;
      flex: none;
    }
  }
</style>
