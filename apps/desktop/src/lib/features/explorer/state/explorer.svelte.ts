import { untrack } from 'svelte';
import { SvelteSet } from 'svelte/reactivity';
import { api as defaultApi } from '../../../api';
import { COLLAPSED_NODE_ID, describeError, isApiError, type Api } from '../../../api/client';
import type { ChildrenPage, LodNode, NodeDto, NodeId, PathDto, ScanId, SortKey } from '../../../api/types';
import { resource, type Resource } from '../../../state/resource.svelte';
import { createFolderColorResolver, type FolderColorResolver } from '../../../folderColor';
import { assignMapColors, colorOf, dominantTile, type ColorMap } from '../colors';
import {
  chainsToExpand,
  collapseChains,
  lastOf,
  linksOf,
  MAX_CHAIN_EXPANSIONS,
  spliceLod,
  type ChainedLod,
} from '../layout/chains';
import { itemCount } from '../layout/legibility';
import { MAP_DEPTH } from '../layout/treemap';
import { crumbPaths } from '../path';
import { normalizeQuery, SEARCH_LIMIT } from '../search';

/** How many nodes the map asks for. More would be too small to see; the rest arrive as "N smaller items". */
export const LOD_MAX_NODES = 600;
/** Rows per request of the "In this folder" list. */
export const PAGE_SIZE = 200;
/** How long the search waits after the last key before it asks the backend. */
export const QUERY_DELAY_MS = 150;
/** How deep a chain's last folder is read: far enough for the levels inside it and for the chains among them. */
export const EXPANSION_DEPTH = MAP_DEPTH + 2;
/** One child holding more than this share of the folder is worth pointing out. */
export const DOMINANT_SHARE = 0.85;

/** Everything the map, the breadcrumb and the details of one folder need, loaded together so they never disagree. */
export interface Frame {
  focus: NodeId;
  lod: LodNode;
  path: PathDto;
}

interface LoadedPage {
  page: ChildrenPage;
  focus: NodeId;
  sort: SortKey;
}

/** What the backend answered to a search: the page of matches, or why it could not be read. */
type MatchAnswer = { focus: NodeId; sort: SortKey; query: string } & (
  { page: ChildrenPage } | { error: unknown }
);

/** The rows loaded after the first page of a list. */
interface MoreRows {
  base: ChildrenPage;
  items: NodeDto[];
}

/** How many of `items` (a list of `total`, sorted by `sort`) have no size, or `undefined` while that cannot be known. */
function uncountedIn(items: readonly NodeDto[], total: number, sort: SortKey): number | undefined {
  const loadedZero = items.filter((item) => item.allocated === 0).length;

  if (items.length >= total) {
    return loadedZero;
  }

  // Sorted by size they are the tail, so the first one seen tells that all the rest are too.
  return sort === 'sizeDesc' && loadedZero > 0 ? total - (items.length - loadedZero) : undefined;
}

/** How many items have a size: exact once `uncountedIn` is known, otherwise the most it can be. */
function sizedIn(items: readonly NodeDto[], total: number, sort: SortKey): number {
  const uncounted = uncountedIn(items, total, sort) ?? items.filter((item) => item.allocated === 0).length;

  return Math.max(0, total - uncounted);
}

/** A child that holds most of the open folder. */
export interface Dominant {
  node: NodeDto;
  /** The folder's name, or the names of its chain joined with "/": `home/sam`. */
  name: string;
  /** Whole per cent of the folder, rounded down, so "97%" is never more than it is. */
  percent: number;
}

export interface ExplorerStart {
  focus: NodeId;
  /** Select this child once the folder is shown. */
  select?: NodeId;
  /** The colour of the folder, when the caller knows it (the Overview does): the scan root has `null`. */
  color?: string | null;
}

export async function loadFrame(api: Api, scan: ScanId, focus: NodeId): Promise<Frame> {
  const [lod, path] = await Promise.all([
    api.getLod(scan, focus, LOD_MAX_NODES, MAP_DEPTH),
    api.getPath(scan, focus),
  ]);

  return { focus, lod, path };
}

/**
 * Where the Explorer opens. A folder opens on itself; a file opens on its folder with the file selected;
 * a node that does not exist (an old link after a rescan) falls back to the scan root.
 */
export async function resolveStart(
  api: Api,
  scan: ScanId,
  rootId: NodeId,
  wanted: NodeId | undefined,
): Promise<ExplorerStart> {
  if (wanted === undefined || wanted === rootId) {
    return { focus: rootId };
  }

  try {
    const node = await api.getNode(scan, wanted);

    if (node.hasChildren || node.kind === 'dir' || node.kind === 'mount') {
      return { focus: wanted };
    }

    const parent = (await api.getPath(scan, wanted)).segments.at(-2);

    return parent ? { focus: parent.id, select: wanted } : { focus: rootId };
  } catch (error) {
    if (isApiError(error) && error.code === 'notFound') {
      return { focus: rootId };
    }

    throw error;
  }
}

export interface Crumb {
  id: NodeId;
  /** The name of the folder, like `getPath` gives it: the root crumb is named after the root folder. */
  name: string;
  /** The full path of the folder, to show on the root crumb and as a tooltip on every crumb. */
  path: string;
}

/**
 * What the Explorer page shows and does: which folder is open, what is selected or hovered, the list's sort and pages.
 * Create it while a component is initialising: its loads are `resource()`s, which drop answers that arrive late.
 */
export class ExplorerStore {
  /** The folder the user asked for. The map follows once its data has arrived. */
  focus = $state<NodeId>(0);
  /**
   * The colour of the open folder: its largest child inherits it, the rest take the palette. `null` at the scan
   * root, which has none, and while the colour of a folder reached by a link is still being looked up.
   */
  focusColor = $state<string | null>(null);
  /** A child of the open folder. */
  selected = $state<NodeId | null>(null);
  /** The child under the pointer, in the map or in the list. */
  hovered = $state<NodeId | null>(null);
  sort = $state<SortKey>('sizeDesc');
  /**
   * What the list is filtered by (a part of a name); `''` when it is not. This is the query the backend is asked
   * for: `setQuery` applies it after a short pause. Not saved, and cleared by `open`.
   */
  query = $state('');
  /** The name of the folder being opened, for "Reading …" while its data is on the way. */
  opening = $state<string | null>(null);
  /** A request for more rows is on its way for the open folder and sort. */
  loadingMore = $state(false);
  moreError = $state<string | null>(null);

  readonly scanId: ScanId;
  readonly rootId: NodeId;
  readonly frame: Resource<Frame>;
  readonly page: Resource<ChildrenPage>;

  readonly #api: Api;
  readonly #resolver: FolderColorResolver;
  /** The colours of the folders the user has been through: going back restores them without asking. */
  // Bookkeeping that is never rendered, so a plain Map is right.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  readonly #known = new Map<NodeId, string | null>();
  #reload = $state(0);
  /** The rows loaded after the first page of the whole folder, and after the first page of the matches. */
  #more = $state.raw<MoreRows | null>(null);
  #moreMatching = $state.raw<MoreRows | null>(null);
  /** Counts the folders, sorts and queries the list has had, so a request knows when it was left behind. */
  #moreRound = 0;
  /** The answer to the query, for the folder and sort it was asked for. */
  #answer = $state.raw<MatchAnswer | null>(null);
  #queryTimer: ReturnType<typeof setTimeout> | undefined;
  /** The child that was last chosen: the details keep describing it while a search hides its row. */
  #chosen: NodeDto | undefined;
  /** The first page together with the folder and sort it was loaded for; `page` shows just the page. */
  readonly #first: Resource<LoadedPage>;
  /** The way to `focus` while its frame is still loading; `crumbs` is the way to the shown folder. */
  #trail: Crumb[] | null = null;
  /** Deeper answers for the last folders of the chains on the map, for the frame they were asked for. */
  #expansions = $state.raw<{ frame: Frame; subs: readonly LodNode[] } | null>(null);
  /** Counts the frames and navigations, so an answer for a chain the user has left is dropped. */
  #expandRound = 0;
  /** The "N smaller items" tile or column that is selected, and the items it stands for. */
  #group = $state.raw<{ node: NodeDto; members: readonly NodeId[] } | null>(null);

  constructor(scanId: ScanId, rootId: NodeId, start: ExplorerStart, api: Api = defaultApi) {
    this.#api = api;
    this.scanId = scanId;
    this.rootId = rootId;
    this.focus = start.focus;
    this.selected = start.select ?? null;
    this.#resolver = createFolderColorResolver(api, scanId);
    this.#known.set(rootId, null);

    if (start.color !== undefined) {
      this.#known.set(start.focus, start.color);
    }

    this.frame = resource(() => {
      void this.#reload;

      return loadFrame(this.#api, this.scanId, this.focus);
    });
    const first = resource(() => {
      void this.#reload;

      const { focus, sort } = this;

      return this.#api
        .getChildren(this.scanId, focus, 0, PAGE_SIZE, sort)
        .then((page): LoadedPage => ({ page, focus, sort }));
    });

    // With a query the backend filters the folder: its first page of matches replaces the list. An answer for a
    // query, folder or sort the user has left is dropped, so the last thing typed is what is shown.
    $effect(() => {
      void this.#reload;

      const { focus, sort, query } = this;

      this.#answer = null;

      if (query === '') {
        return;
      }

      let current = true;

      this.#api.getChildren(this.scanId, focus, 0, PAGE_SIZE, sort, query).then(
        (page) => {
          if (current) {
            this.#answer = { focus, sort, query, page };
          }
        },
        (error: unknown) => {
          if (current) {
            this.#answer = { focus, sort, query, error };
          }
        },
      );

      return () => {
        current = false;
      };
    });

    $effect(() => () => clearTimeout(this.#queryTimer));

    // Every shown folder gets its chains read deeper, a few at a time.
    $effect(() => {
      const frame = this.shown;

      untrack(() => {
        if (frame) {
          this.#adoptColor(frame.focus);
        }

        void this.#expandChains(frame);
      });
    });

    this.#first = first;

    const matching = (): MatchAnswer | undefined => this.#matching;
    const filtering = (): boolean => this.#filtering;

    this.page = {
      get data() {
        const answer = matching();

        if (filtering()) {
          return answer && 'page' in answer ? answer.page : undefined;
        }

        return first.data?.page;
      },
      get error() {
        const answer = matching();

        if (filtering()) {
          return answer && 'error' in answer ? answer.error : undefined;
        }

        return first.error;
      },
      get loading() {
        return filtering() ? matching() === undefined : first.loading;
      },
    };
  }

  /** A query is applied: the list shows the matches, which the backend counts and pages. */
  get #filtering(): boolean {
    return this.query !== '';
  }

  /** The answer to the query that is applied now, once it has arrived. */
  get #matching(): MatchAnswer | undefined {
    const answer = this.#answer;

    if (answer && answer.focus === this.focus && answer.sort === this.sort && answer.query === this.query) {
      return answer;
    }

    return undefined;
  }

  /**
   * Whether the first page held in `page` was loaded for the folder, sort and query that are asked for now. Right
   * after `open` or `setSort` it still holds the previous one until the new page arrives; with a query there is no
   * page at all until the matches have arrived.
   */
  get pageCurrent(): boolean {
    if (this.#filtering) {
      return this.page.data !== undefined;
    }

    const loaded = this.#first.data;

    return loaded !== undefined && loaded.focus === this.focus && loaded.sort === this.sort;
  }

  /** The folder on screen. While the next one loads this is still the previous one. */
  get shown(): Frame | undefined {
    return this.frame.data;
  }

  /** The shown folder itself. */
  get node(): NodeDto | undefined {
    return this.shown?.lod.node;
  }

  get crumbs(): Crumb[] {
    const path = this.shown?.path;

    if (!path) {
      return [];
    }

    const paths = crumbPaths(path.path, path.segments.length);

    return path.segments.map((segment, index) => ({ ...segment, path: paths[index] as string }));
  }

  get parent(): Crumb | undefined {
    return this.crumbs.at(-2);
  }

  /** The children the map draws. */
  get topLevel(): NodeDto[] {
    return this.shown?.lod.children.map((child) => child.node) ?? [];
  }

  /** The shown folder's data with the deeper answers for its chains spliced in. */
  readonly #lod: LodNode | undefined = $derived.by(() => {
    const frame = this.shown;
    const expansions = this.#expansions;

    if (!frame) {
      return undefined;
    }

    return (expansions?.frame === frame ? expansions.subs : []).reduce<LodNode>(
      (lod, sub) => spliceLod(lod, sub),
      frame.lod,
    );
  });

  /** What the map draws: the shown folder with every chain of single-child folders as one node. */
  readonly model: ChainedLod | undefined = $derived(this.#lod ? collapseChains(this.#lod) : undefined);

  /** The colour of each top-level child, and of each folder inside a top-level tile that holds most of the map. */
  readonly colors: ColorMap = $derived(assignMapColors(this.focusColor, this.model?.children ?? []));

  /** The top-level tile that holds most of the map and shows its own folders in colours of their own. */
  get dominantTile(): NodeId | undefined {
    const tile = dominantTile(this.model?.children ?? []);

    return tile?.node.id;
  }

  /**
   * Reads the last folder of each chain at the top of the map deeper (the biggest chain first, at most
   * `MAX_CHAIN_EXPANSIONS` requests), one after another, and splices each answer into the model. An answer that
   * comes after the user has moved on, or after a newer frame replaced this one, is dropped.
   */
  async #expandChains(frame: Frame | undefined): Promise<void> {
    const round = ++this.#expandRound;
    const subs: LodNode[] = [];
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const asked = new Set<NodeId>();

    this.#expansions = null;

    if (!frame) {
      return;
    }

    while (subs.length < MAX_CHAIN_EXPANSIONS) {
      const lod = subs.reduce<LodNode>((whole, sub) => spliceLod(whole, sub), frame.lod);
      const [next] = chainsToExpand(collapseChains(lod), lod, asked);

      if (next === undefined) {
        return;
      }

      asked.add(next);

      try {
        const sub = await this.#api.getLod(this.scanId, next, LOD_MAX_NODES, EXPANSION_DEPTH);

        if (round !== this.#expandRound) {
          return;
        }

        subs.push(sub);
        this.#expansions = { frame, subs: [...subs] };
      } catch {
        // The map is drawn without the extra depth.
        return;
      }
    }
  }

  /** The colour of the focused folder as it is looked up: a known one at once, a deep link's after a request. */
  #adoptColor(focus: NodeId): void {
    if (this.#known.has(focus)) {
      this.focusColor = this.#known.get(focus) ?? null;

      return;
    }

    this.focusColor = null;
    void this.#resolver.colorOf(focus).then(
      (color) => {
        this.#known.set(focus, color);

        if (this.shown?.focus === focus) {
          this.focusColor = color;
        }
      },
      () => undefined,
    );
  }

  /** The folder a tile opens: the last of its chain, or the folder itself. */
  openTarget(id: NodeId): NodeDto | undefined {
    const tile = this.model?.children.find((child) => child.node.id === id);

    return tile ? lastOf(tile) : this.find(id);
  }

  /** The share of the open folder that a child holds, as a fraction. */
  shareOf(node: NodeDto): number {
    const total = this.node?.allocated ?? 0;

    return total > 0 ? node.allocated / total : 0;
  }

  /**
   * The colour of a child. The map colours the biggest children by rank and draws every other child inside the
   * "N smaller items" tile, which is slate; a row of the list that the map does not show separately is slate too.
   */
  colorOf(id: NodeId): string {
    return colorOf(this.colors, id);
  }

  /** The loaded rows, joined once per change and not on every read. */
  readonly #items: NodeDto[] = $derived.by(() => {
    const first = this.page.data;

    if (!first) {
      return [];
    }

    const more = this.#filtering ? this.#moreMatching : this.#more;

    return more?.base === first ? [...first.items, ...more.items] : first.items;
  });

  /** The loaded rows of the whole folder, which stay while a search shows the matches. */
  readonly #allRows: { items: NodeDto[]; total: number } | undefined = $derived.by(() => {
    const first = this.#first.data?.page;

    if (!first) {
      return undefined;
    }

    return {
      items: this.#more?.base === first ? [...first.items, ...this.#more.items] : first.items,
      total: first.total,
    };
  });

  /** The loaded rows, for the list. */
  get items(): NodeDto[] {
    return this.#items;
  }

  /** How many items the list has: the folder's, or with a query the number of matches. */
  get total(): number {
    return this.page.data?.total ?? 0;
  }

  get hasMore(): boolean {
    return this.page.data !== undefined && this.items.length < this.total;
  }

  /** Finds a child of the shown folder by id: in the map's data first, then in the loaded rows. */
  find(id: NodeId): NodeDto | undefined {
    return (
      this.topLevel.find((node) => node.id === id) ??
      this.items.find((node) => node.id === id) ??
      this.#allRows?.items.find((node) => node.id === id)
    );
  }

  /** The selected child, or `undefined` when nothing is selected or it is not in this folder. */
  get selectedNode(): NodeDto | undefined {
    if (this.selected === null) {
      return undefined;
    }

    if (this.selected === COLLAPSED_NODE_ID) {
      return this.#group?.node;
    }

    // A search can leave the chosen child's row out of the loaded ones; it is still the selection.
    return this.find(this.selected) ?? (this.#chosen?.id === this.selected ? this.#chosen : undefined);
  }

  /**
   * The folder the selection stands for, which is the one "Go into folder" opens and "Show in file manager" shows: for
   * a chain tile ("home / sam") the last folder of the chain, otherwise the selected child itself. The details of
   * the selection come from this one node, so that its path, size and actions never describe two folders.
   */
  get selectedTarget(): NodeDto | undefined {
    const chosen = this.selectedNode;

    if (!chosen || this.groupSelected) {
      return chosen;
    }

    return this.openTarget(chosen.id);
  }

  /** The "N smaller items" tile is selected: it is not a folder, so it has no path and cannot be opened. */
  get groupSelected(): boolean {
    return this.selected === COLLAPSED_NODE_ID && this.#group !== null;
  }

  /**
   * The rows of the list that belong to the selected "smaller items" tile: the items that were merged into it, and
   * those the backend left out of the map before (every child the map has no node for).
   */
  get highlighted(): ReadonlySet<NodeId> {
    return this.#highlighted;
  }

  /** Worked out once per change of the rows or the selection, not on every read. */
  readonly #highlighted: ReadonlySet<NodeId> = $derived.by(() => {
    const group = this.#group;

    if (!group || this.selected !== COLLAPSED_NODE_ID) {
      return new SvelteSet();
    }

    const merged = new SvelteSet(group.members);
    const onMap = new SvelteSet(this.topLevel.map((node) => node.id));

    return new SvelteSet(
      this.items.filter((item) => merged.has(item.id) || !onMap.has(item.id)).map((item) => item.id),
    );
  });

  /** The selected "smaller items" tile has rows that are not loaded yet, however many rows are loaded already. */
  get #groupIncomplete(): boolean {
    const group = this.#group;

    return (
      group !== null && !this.#filtering && this.hasMore && this.#highlighted.size < itemCount(group.node)
    );
  }

  /**
   * The selected "smaller items" tile has rows that are not loaded yet, and the list may load them in the
   * background: up to `SEARCH_LIMIT` rows in all, and not after a request failed.
   */
  get groupNeedsRows(): boolean {
    return this.#groupIncomplete && this.items.length < SEARCH_LIMIT && this.moreError === null;
  }

  /** The selected "smaller items" tile has more rows than the list loads by itself, so only the first ones are shown. */
  get groupTruncated(): boolean {
    return this.#groupIncomplete && this.items.length >= SEARCH_LIMIT;
  }

  /** The biggest child, when it holds more than `DOMINANT_SHARE` of the folder and can be opened. */
  get dominant(): Dominant | undefined {
    const children = this.model?.children ?? [];
    const total = children.reduce((sum, child) => sum + child.node.allocated, 0);
    const biggest = children
      .filter((child) => child.node.id !== COLLAPSED_NODE_ID)
      .reduce<ChainedLod | undefined>(
        (best, child) => (!best || child.node.allocated > best.node.allocated ? child : best),
        undefined,
      );

    if (
      !biggest ||
      !biggest.node.hasChildren ||
      total <= 0 ||
      biggest.node.allocated / total <= DOMINANT_SHARE
    ) {
      return undefined;
    }

    return {
      node: biggest.node,
      name: linksOf(biggest)
        .map((link) => link.name)
        .join('/'),
      percent: Math.floor((biggest.node.allocated / total) * 100),
    };
  }

  /**
   * Sets what the list is filtered by. A query is applied after `QUERY_DELAY_MS` without another key, so typing asks
   * the backend once; clearing it applies at once and shows the whole folder again, without asking for it.
   */
  setQuery(text: string): void {
    const query = normalizeQuery(text);

    clearTimeout(this.#queryTimer);

    if (query === '') {
      this.#applyQuery('');

      return;
    }

    this.#queryTimer = setTimeout(() => this.#applyQuery(query), QUERY_DELAY_MS);
  }

  #applyQuery(query: string): void {
    if (query === this.query) {
      return;
    }

    this.query = query;
    this.#moreMatching = null;
    // A request for more rows of the previous list is left behind.
    this.#moreRound++;
    this.loadingMore = false;
    this.moreError = null;
  }

  /** The rows with a size: these are the list. */
  get sizedItems(): NodeDto[] {
    return this.items.filter((item) => item.allocated > 0);
  }

  /** The loaded rows without a size (not read, empty, another disk), by name. */
  get uncountedItems(): NodeDto[] {
    return this.items
      .filter((item) => item.allocated === 0)
      .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()) || a.id - b.id);
  }

  /**
   * How many items of the list have no size, or `undefined` while that cannot be known. All of them are known once
   * every page is loaded. Sorted by size they are the tail, so the first one seen tells that all the rest are too.
   */
  get uncountedTotal(): number | undefined {
    if (this.page.data === undefined) {
      return undefined;
    }

    return uncountedIn(this.items, this.total, this.sort);
  }

  /** How many items of the list have a size: exact once `uncountedTotal` is known, otherwise the most it can be. */
  get sizedTotal(): number {
    return sizedIn(this.items, this.total, this.sort);
  }

  /** How many items of the whole folder have a size, which a search does not change. */
  get allSizedTotal(): number {
    const all = this.#allRows;

    if (!this.#filtering || !all) {
      return this.sizedTotal;
    }

    return sizedIn(all.items, all.total, this.sort);
  }

  /** How many of the matches have a size while a query is applied, once they have arrived. */
  get matchedTotal(): number | undefined {
    return this.#filtering && this.page.data !== undefined ? this.sizedTotal : undefined;
  }

  /**
   * Opens a folder, or a tile of the map: a chain tile ("home / sam") opens its last folder, and the way to it
   * goes through every folder of the chain.
   */
  open(id: NodeId): void {
    if (id === COLLAPSED_NODE_ID) {
      return;
    }

    const shown = this.crumbs;
    const tile = this.model?.children.find((child) => child.node.id === id);
    const links = tile ? [...linksOf(tile)] : [];
    const destination = links.at(-1)?.id ?? id;
    const known = shown.findIndex((crumb) => crumb.id === destination);
    const via = links.map((link): Crumb => ({ id: link.id, name: link.name, path: '' }));
    const target: Crumb =
      known >= 0
        ? (shown[known] as Crumb)
        : { id: destination, name: this.find(destination)?.name ?? '', path: '' };

    // What is on screen is the folder the user clicked in, so the way to the new one starts from it.
    this.#trail = known >= 0 ? shown.slice(0, known + 1) : [...shown, ...(via.length > 0 ? via : [target])];
    this.opening = target.name || null;
    this.#expandRound++;
    clearTimeout(this.#queryTimer);
    this.query = '';

    // The way back finds every folder it came through with its colour. A chain has one colour: its first folder's.
    if (tile) {
      const color = this.colors.get(tile.node.id) ?? null;

      links.forEach((link) => this.#known.set(link.id, color));
    } else if (!this.#known.has(id) && this.colors.has(id)) {
      this.#known.set(id, this.colors.get(id) ?? null);
    }

    id = destination;
    this.focus = id;
    this.#group = null;
    this.selected = null;
    this.hovered = null;
    this.#startOver();
  }

  /** One level up. Does nothing at the scan root. Two quick calls go up two levels. */
  up(): void {
    const trail = this.#trail?.at(-1)?.id === this.focus ? this.#trail : this.crumbs;
    const parent = trail.at(-2);

    if (parent && this.focus !== this.rootId) {
      this.open(parent.id);
    }
  }

  toRoot(): void {
    if (this.focus !== this.rootId) {
      this.open(this.rootId);
    }
  }

  /** "Go into folder" for the selected child: a chain tile opens its last folder. */
  openSelected(): void {
    if (this.selected !== null) {
      this.open(this.selected);
    }
  }

  select(id: NodeId | null): void {
    this.#group = null;
    this.selected = id === COLLAPSED_NODE_ID ? null : id;
  }

  /** A click on the "smaller items" tile: it is selected, and its items are shown in the list. It is never opened. */
  chooseGroup(node: NodeDto, members: readonly NodeId[]): void {
    this.#group = { node, members };
    this.selected = COLLAPSED_NODE_ID;
  }

  /** A click on a child: the first selects it, the next opens it. Files only select. */
  choose(id: NodeId): void {
    if (id === COLLAPSED_NODE_ID) {
      return;
    }

    this.#group = null;

    if (this.selected === id && this.find(id)?.hasChildren) {
      this.open(id);
    } else {
      this.#chosen = this.find(id);
      this.selected = id;
    }
  }

  hover(id: NodeId | null): void {
    this.hovered = id === COLLAPSED_NODE_ID ? null : id;
  }

  setSort(sort: SortKey): void {
    if (sort !== this.sort) {
      this.sort = sort;
      this.#startOver();
    }
  }

  /** The list starts again from its first page: forget the extra rows, and let a request in flight go. */
  #startOver(): void {
    this.#more = null;
    this.#moreMatching = null;
    this.#moreRound++;
    this.loadingMore = false;
    this.moreError = null;
  }

  /**
   * Loads the next rows of the list. It does nothing until the first page for the current folder and sort has
   * arrived, and an answer for a folder or sort the user has left is dropped.
   */
  async loadMore(): Promise<void> {
    const base = this.page.data;

    if (!base || !this.pageCurrent || this.loadingMore || !this.hasMore) {
      return;
    }

    const { focus, sort, query } = this;
    const round = this.#moreRound;
    const have = this.items.length;

    this.loadingMore = true;
    this.moreError = null;

    try {
      const next = await this.#api.getChildren(
        this.scanId,
        focus,
        have,
        PAGE_SIZE,
        sort,
        query === '' ? undefined : query,
      );

      if (
        round === this.#moreRound &&
        this.page.data === base &&
        this.focus === focus &&
        this.sort === sort &&
        this.query === query
      ) {
        this.#keepMore(base, next.items);
      }
    } catch (error) {
      if (round === this.#moreRound) {
        this.moreError = describeError(error);
      }
    } finally {
      if (round === this.#moreRound) {
        this.loadingMore = false;
      }
    }
  }

  /** Adds rows that arrived after the first page `base` to the list they belong to. */
  #keepMore(base: ChildrenPage, items: NodeDto[]): void {
    const kept = this.#filtering ? this.#moreMatching : this.#more;
    const more: MoreRows = { base, items: [...(kept?.base === base ? kept.items : []), ...items] };

    if (this.#filtering) {
      this.#moreMatching = more;
    } else {
      this.#more = more;
    }
  }

  /** Shows the selected child (a chain tile: its last folder, which "Go into folder" opens), or else the open folder. */
  async reveal(): Promise<void> {
    const id = (this.groupSelected ? undefined : this.selectedTarget?.id) ?? this.shown?.focus;

    if (id !== undefined) {
      await this.#api.openInFileManager(this.scanId, id);
    }
  }

  /** Tries the current folder again after an error. */
  retry(): void {
    this.#reload++;
  }
}
