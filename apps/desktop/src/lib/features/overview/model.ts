import type { Api } from '../../api/client';
import { createFolderColorResolver } from '../../folderColor';
import { platform } from '../../state/platform.svelte';
import type { NodeDto, NodeId, ScanId, SummaryDto } from '../../api/types';
import {
  buildSlabs,
  dominantChild,
  isSlabSized,
  levelColors,
  MAX_BREAKDOWN,
  type Slab,
  type SlabLevel,
} from './slabs';

/**
 * What a slab says is inside it: the names of its sub-folders, biggest first. Files are left out, they read as noise
 * (a folder of archives lists archive names); a folder with no sub-folders says nothing.
 */
export function folderNames(children: readonly NodeDto[]): string[] {
  return children.filter((child) => child.kind === 'dir').map((child) => child.name);
}

export interface OverviewData {
  summary: SummaryDto;
  slabs: Slab[];
  /** Path of each of `summary.largestDirs`, for display. */
  paths: Map<NodeId, string>;
  /** Colour of each of `summary.largestDirs`: the colour of the folder itself, the same in every view. */
  folderColors: Map<NodeId, string>;
}

export interface OverviewRequest {
  scanId: ScanId;
  rootId: NodeId;
  /** What 100% of the stack is: the volume size, or the scanned size without a volume. */
  total: number;
  /** Free space of the volume, when the scan root is a whole volume. */
  free?: number;
}

/** How many children of a folder are fetched to find the slabs. More than enough: only a few are shown. */
export const CHILDREN_LIMIT = 64;

/** How many children of a slab are read to name the folders inside it. */
export const INSIDE_LIMIT = 32;

/**
 * The folders to look into, outermost first: the scan root, then the dominant child of the root, then the dominant
 * child of that, up to `MAX_BREAKDOWN` folders deep. A folder is only broken down when something inside it is big
 * enough to be a slab, so the stack never trades a big slab for a pile of "other".
 */
async function loadLevels(
  api: Api,
  { scanId, total }: OverviewRequest,
  root: NodeDto,
  top: NodeDto[],
): Promise<SlabLevel[]> {
  const levels: SlabLevel[] = [{ path: '', children: top }];
  let parentSize = root.allocated;

  for (let depth = 0; depth < MAX_BREAKDOWN; depth++) {
    const last = levels[levels.length - 1] as SlabLevel;
    const dominant = dominantChild(last.children, parentSize);

    if (!dominant) {
      break;
    }

    const page = await api.getChildren(scanId, dominant.id, 0, CHILDREN_LIMIT, 'sizeDesc');

    if (!page.items.some((child) => isSlabSized(child, total))) {
      break;
    }

    levels.push({
      path: `${last.path}${platform.separator}${dominant.name}`,
      parent: dominant.id,
      children: page.items,
    });
    parentSize = dominant.allocated;
  }

  return levels;
}

/**
 * Everything the Overview page shows, fetched from the API in rounds. It keeps no state outside its own call,
 * so a slow answer that arrives after a newer load started cannot mix into it (`resource` drops that result).
 */
export async function loadOverview(api: Api, request: OverviewRequest): Promise<OverviewData> {
  const { scanId, rootId, total, free } = request;

  const [summary, top] = await Promise.all([
    api.getSummary(scanId, rootId),
    api.getChildren(scanId, rootId, 0, CHILDREN_LIMIT, 'sizeDesc'),
  ]);

  const levels = await loadLevels(api, request, summary.node, top.items);
  const known = new Map<NodeId, NodeDto>(
    levels.flatMap((level) => level.children).map((node) => [node.id, node]),
  );

  // Only the folders that become slabs need a look inside, and only the largest dirs need a path.
  const sketch = buildSlabs({ levels, insideOf: () => [], total, free });
  const inside = new Map<NodeId, string[]>();
  const paths = new Map<NodeId, string>();
  const resolver = createFolderColorResolver(api, scanId);

  // The folders of the stack are coloured already; the largest folders are found from them, with the least lookups.
  for (const [id, color] of levelColors(levels)) {
    resolver.seed(id, color);
  }

  await Promise.all([
    ...sketch
      .filter((slab) => slab.nodeId !== undefined && known.get(slab.nodeId)?.hasChildren)
      .map(async (slab) => {
        const id = slab.nodeId as NodeId;
        // The biggest children can all be files, so the listing is long enough to reach the folders among them.
        const page = await api.getChildren(scanId, id, 0, INSIDE_LIMIT, 'sizeDesc');

        inside.set(id, folderNames(page.items));
      }),
    ...summary.largestDirs.map(async (dir: NodeDto) => {
      const path = await api.getPath(scanId, dir.id);

      paths.set(dir.id, path.path);
    }),
  ]);

  const slabs = buildSlabs({ levels, insideOf: (id) => inside.get(id) ?? [], total, free });
  const folderColors = new Map<NodeId, string>(
    await Promise.all(
      summary.largestDirs.map(async (dir): Promise<[NodeId, string]> => [
        dir.id,
        await resolver.colorOf(dir.id),
      ]),
    ),
  );

  return { summary, paths, folderColors, slabs };
}
