import { ApiRequestError, COLLAPSED_NODE_ID, type Api } from './client';
import { REAL_BYTES, REAL_ROOT, SKEW_BYTES, SKEW_ROOT } from './mock-layouts';
import { suggestionsOf } from './mock-suggestions';
import { buildTree, childrenOf, DAY, type MockNode, type MockTree } from './mock-tree';
import type {
  AgeBucket,
  Category,
  ChildrenPage,
  ExternalLink,
  LodNode,
  NodeDto,
  NodeId,
  PathDto,
  ScanEvent,
  ScanId,
  ScanOptions,
  SortKey,
  SummaryDto,
  VolumeDto,
} from './types';

const GB = 1_000_000_000;

const REPOSITORY = 'https://github.com/MSpiechowicz/platter';

export const EXTERNAL_URLS: Record<ExternalLink, string> = {
  source: REPOSITORY,
  issues: `${REPOSITORY}/issues`,
  license: `${REPOSITORY}/blob/main/LICENSE`,
};

export const MOCK_VOLUMES: VolumeDto[] = [
  {
    id: 'nvme0n1p2',
    mountPoint: '/',
    label: '',
    device: '/dev/nvme0n1p2',
    fsType: 'btrfs',
    total: 1900 * GB,
    available: 492 * GB,
    removable: false,
  },
  {
    id: 'sda1',
    mountPoint: '/mnt/backup',
    label: 'Backup',
    device: '/dev/sda1',
    fsType: 'ext4',
    total: 3640 * GB,
    available: 1210 * GB,
    removable: false,
  },
  {
    id: 'sdb1',
    mountPoint: '/run/media/demo/USB',
    label: 'USB stick',
    device: '/dev/sdb1',
    fsType: 'exfat',
    total: 58 * GB,
    available: 31 * GB,
    removable: true,
  },
];

const btrfsRoot = MOCK_VOLUMES[0] as VolumeDto;

/** A disk with several physical filesystems, and a btrfs root whose subvolumes are mounted in many places. */
export const MOCK_VOLUMES_WITH_SUBVOLUMES: VolumeDto[] = [
  ...['/home', '/', '/srv', '/var/log', '/var/cache', '/.snapshots'].map((mountPoint) => ({
    ...btrfsRoot,
    id: mountPoint === '/' ? btrfsRoot.id : `${btrfsRoot.id}:${mountPoint}`,
    mountPoint,
  })),
  ...MOCK_VOLUMES.slice(1),
  ...['c', 'd', 'e', 'f'].map((letter, index) => ({
    id: `sd${letter}1`,
    mountPoint: `/mnt/disk${index + 1}`,
    label: `Disk ${index + 1}`,
    device: `/dev/sd${letter}1`,
    fsType: 'ext4',
    total: 2000 * GB,
    available: (640 + index * 130) * GB,
    removable: false,
  })),
];

export interface MockOptions {
  seed?: number;
  /** How long a scan takes. 0 finishes on the next tick. */
  durationMs?: number;
  /** Time between progress events. */
  tickMs?: number;
  /** Test hook: stall at this fraction (0..1) and never finish, so a half-done scan can be looked at. */
  holdAt?: number;
  /** Scan time in unix seconds. */
  now?: () => number;
  volumes?: VolumeDto[];
  /** Test hook: the demo disk keeps nearly everything in one folder (see `dominantLayout`). */
  dominant?: boolean;
  pickedFolder?: string | null;
}

/** Elapsed time reported for an instant scan, so the header does not read "in 0 s". */
const INSTANT_SCAN_ELAPSED_MS = 2140;
const FOLDER_BYTES = 186 * GB;
const UNREADABLE_DIRS = 3;

interface ScanRecord {
  tree: MockTree;
  onEvent: (event: ScanEvent) => void;
  timer: ReturnType<typeof setInterval> | ReturnType<typeof setTimeout> | null;
  running: boolean;
}

function stopTimer(record: ScanRecord): void {
  if (record.timer !== null) {
    clearInterval(record.timer);
    clearTimeout(record.timer);
    record.timer = null;
  }
}

/** The limits of the real backend: larger requests are clamped, not rejected. */
const MAX_LIMIT = 1000;
const MAX_LOD_NODES = 5000;
const MAX_LOD_DEPTH = 64;

/** Directories listed in `largestDirs`, and the share of a directory one child directory may hold before it is left out. */
const LARGEST_DIRS = 8;
const DOMINANT_SHARE_PERCENT = 80;

const fail = (code: 'notFound' | 'invalid', message: string): never => {
  throw new ApiRequestError(code, message);
};

function sorter(sort: SortKey): (a: MockNode, b: MockNode) => number {
  // Like the backend: ASCII-lowercased byte order (`file10` before `file2`), then id. The mock's names are
  // ASCII, where UTF-16 order and UTF-8 byte order agree.
  const lower = (name: string): string => name.replace(/[A-Z]/g, (letter) => letter.toLowerCase());
  const byName = (a: MockNode, b: MockNode): number => {
    const left = lower(a.name);
    const right = lower(b.name);

    if (left === right) {
      return a.id - b.id;
    }

    return left < right ? -1 : 1;
  };

  switch (sort) {
    case 'sizeDesc':
      return (a, b) => b.allocated - a.allocated || byName(a, b);
    case 'sizeAsc':
      return (a, b) => a.allocated - b.allocated || byName(a, b);
    case 'nameAsc':
      return byName;
    case 'filesDesc':
      return (a, b) => b.files - a.files || byName(a, b);
    case 'mtimeDesc':
      return (a, b) => b.mtime - a.mtime || byName(a, b);
  }
}

function toDto(node: MockNode): NodeDto {
  return {
    id: node.id,
    name: node.name,
    kind: node.kind,
    allocated: node.allocated,
    apparent: node.apparent,
    files: node.files,
    hasChildren: node.childCount > 0,
    category: node.category,
    mtime: node.mtime,
    ...(node.unreadable ? { unreadable: true } : {}),
  };
}

function collapsedDto(hidden: MockNode[]): NodeDto {
  return {
    id: COLLAPSED_NODE_ID,
    name: `${hidden.length} smaller items`,
    kind: 'collapsed',
    allocated: hidden.reduce((sum, node) => sum + node.allocated, 0),
    apparent: hidden.reduce((sum, node) => sum + node.apparent, 0),
    files: hidden.reduce((sum, node) => sum + node.files, 0),
    hasChildren: false,
    category: 'other',
    mtime: hidden.reduce((newest, node) => Math.max(newest, node.mtime), 0),
  };
}

const AGE_LIMITS_DAYS: [AgeBucket, number][] = [
  ['week', 7],
  ['month', 30],
  ['quarter', 90],
  ['year', 365],
  ['twoYears', 730],
];

function ageBucket(ageSeconds: number): AgeBucket {
  const days = ageSeconds / DAY;

  for (const [bucket, limit] of AGE_LIMITS_DAYS) {
    if (days <= limit) {
      return bucket;
    }
  }

  return 'older';
}

function pathOf(tree: MockTree, node: MockNode): PathDto {
  const chain: MockNode[] = [];

  for (let current: MockNode | undefined = node; current; current = tree.nodes[current.parent]) {
    chain.unshift(current);
  }

  const root = chain[0] as MockNode;
  const rest = chain.slice(1).map((entry) => entry.name);
  const prefix = tree.rootPath === '/' ? '' : tree.rootPath;

  return {
    path: rest.length === 0 ? tree.rootPath : `${prefix}/${rest.join('/')}`,
    segments: [
      { id: root.id, name: root.name },
      ...chain.slice(1).map((entry) => ({ id: entry.id, name: entry.name })),
    ],
  };
}

function lodOf(tree: MockTree, rootNode: MockNode, maxNodes: number, maxDepth: number): LodNode {
  interface Draft {
    node: MockNode;
    depth: number;
    kids: (Draft | { collapsed: NodeDto })[];
  }

  const root: Draft = { node: rootNode, depth: 0, kids: [] };
  const frontier: Draft[] = rootNode.childCount > 0 && maxDepth > 0 ? [root] : [];
  let count = 1;

  while (frontier.length > 0 && count < maxNodes) {
    let largest = 0;
    frontier.forEach((draft, index) => {
      if (draft.node.allocated > (frontier[largest] as Draft).node.allocated) {
        largest = index;
      }
    });

    const [draft] = frontier.splice(largest, 1) as [Draft];
    const kids = childrenOf(tree, draft.node).sort(sorter('sizeDesc'));
    const room = maxNodes - count;

    // Show every child if they fit, otherwise the biggest ones plus one "N smaller items" node.
    let shown = kids;
    let hidden: MockNode[] = [];

    if (kids.length > room) {
      if (room < 2) {
        continue;
      }

      shown = kids.slice(0, room - 1);
      hidden = kids.slice(room - 1);
    }

    for (const kid of shown) {
      const child: Draft = { node: kid, depth: draft.depth + 1, kids: [] };
      draft.kids.push(child);
      count++;

      if (kid.childCount > 0 && child.depth < maxDepth) {
        frontier.push(child);
      }
    }

    if (hidden.length > 0) {
      draft.kids.push({ collapsed: collapsedDto(hidden) });
      count++;
    }
  }

  const finish = (draft: Draft): LodNode => ({
    node: toDto(draft.node),
    children: draft.kids.map((kid) =>
      'collapsed' in kid ? { node: kid.collapsed, children: [] } : finish(kid),
    ),
  });

  return finish(root);
}

function summaryOf(tree: MockTree, node: MockNode): SummaryDto {
  const byCategory = new Map<Category, { allocated: number; files: number }>();
  const byAge = new Map<AgeBucket, { allocated: number; files: number }>(
    (['week', 'month', 'quarter', 'year', 'twoYears', 'older'] as const).map((bucket) => [
      bucket,
      { allocated: 0, files: 0 },
    ]),
  );

  const stack: MockNode[] = [node];

  for (let current = stack.pop(); current; current = stack.pop()) {
    if (current.kind === 'mount') {
      continue;
    }

    let own = current.allocated;
    let files = 1;

    // A directory's own blocks count for its category and mtime, and the directory is not a file.
    if (current.kind === 'dir') {
      const children = childrenOf(tree, current);

      own -= children.reduce((sum, child) => sum + child.allocated, 0);
      files = 0;
      stack.push(...children);
    }

    const category = byCategory.get(current.category) ?? { allocated: 0, files: 0 };
    category.allocated += own;
    category.files += files;
    byCategory.set(current.category, category);

    const age = byAge.get(ageBucket(tree.now - current.mtime)) as { allocated: number; files: number };
    age.allocated += own;
    age.files += files;
  }

  return {
    node: toDto(node),
    byCategory: [...byCategory]
      .filter(([, value]) => value.allocated > 0 || value.files > 0)
      .map(([category, value]) => ({ category, ...value }))
      .sort((a, b) => b.allocated - a.allocated),
    byAge: [...byAge].map(([bucket, value]) => ({ bucket, ...value })),
    largestDirs: largestDirs(tree, node).map(toDto),
  };
}

/** Whether one child directory of `dir` holds at least `DOMINANT_SHARE_PERCENT` of its allocated bytes. */
function hasDominantChild(tree: MockTree, dir: MockNode): boolean {
  if (dir.allocated === 0) {
    return false;
  }

  return childrenOf(tree, dir).some(
    (child) => child.kind === 'dir' && child.allocated * 100 >= dir.allocated * DOMINANT_SHARE_PERCENT,
  );
}

/**
 * The 8 largest directories among the children and grandchildren of `node`. A child is left out when one of
 * its own child directories (a grandchild, so itself a candidate) holds most of its bytes: `/home` and
 * `/home/user` are not both listed.
 */
function largestDirs(tree: MockTree, node: MockNode): MockNode[] {
  const candidates: MockNode[] = [];

  for (const child of childrenOf(tree, node).filter((entry) => entry.kind === 'dir')) {
    if (!hasDominantChild(tree, child)) {
      candidates.push(child);
    }

    candidates.push(...childrenOf(tree, child).filter((entry) => entry.kind === 'dir'));
  }

  return candidates.sort((a, b) => b.allocated - a.allocated || a.id - b.id).slice(0, LARGEST_DIRS);
}

/** Every directory path in depth-first order: the "currently reading" line of a running scan. */
function directoryPaths(tree: MockTree): string[] {
  const paths: string[] = [];
  const stack: MockNode[] = [tree.nodes[0] as MockNode];

  for (let current = stack.pop(); current; current = stack.pop()) {
    if (current.kind !== 'dir') {
      continue;
    }

    paths.push(pathOf(tree, current).path);
    stack.push(...childrenOf(tree, current).reverse());
  }

  return paths;
}

/** An in-memory `Api` with a deterministic demo disk. Used in the browser and in tests. */
export function createMockApi(options: MockOptions = {}): Api {
  const seed = options.seed ?? 20240607;
  const durationMs = options.durationMs ?? 2000;
  const tickMs = options.tickMs ?? 100;
  const clock = options.now ?? (() => Math.floor(Date.now() / 1000));
  const volumes = options.volumes ?? MOCK_VOLUMES;
  const scans = new Map<ScanId, ScanRecord>();
  let nextId: ScanId = 1;

  const scanOf = (scan: ScanId): ScanRecord => scans.get(scan) ?? fail('notFound', `No such scan: ${scan}`);

  const nodeOf = (scan: ScanId, id: NodeId): { tree: MockTree; node: MockNode } => {
    const { tree } = scanOf(scan);
    const node = tree.nodes[id];

    if (!node) {
      return fail('notFound', `No such node: ${id}`);
    }

    return { tree, node };
  };

  const bytesFor = (root: string): number => {
    if (root === SKEW_ROOT) {
      return SKEW_BYTES;
    }

    if (root === REAL_ROOT) {
      return REAL_BYTES;
    }

    const volume = volumes.find((candidate) => candidate.mountPoint === root);

    return volume ? volume.total - volume.available : FOLDER_BYTES;
  };

  const run = (record: ScanRecord, id: ScanId): void => {
    if (!record.running) {
      return;
    }

    const { tree, onEvent } = record;
    const total = tree.nodes[0] as MockNode;
    const dirs = directoryPaths(tree);
    const dirCount = tree.nodes.filter((node) => node.kind === 'dir').length;
    const started = Date.now();

    const finish = (): void => {
      record.running = false;
      onEvent({
        type: 'finished',
        scanId: id,
        rootId: 0,
        totals: {
          allocated: total.allocated,
          apparent: total.apparent,
          files: total.files,
          dirs: dirCount,
          errors: UNREADABLE_DIRS,
        },
        elapsedMs: durationMs > 0 ? Date.now() - started : INSTANT_SCAN_ELAPSED_MS,
      });
    };

    if (durationMs <= 0) {
      record.timer = setTimeout(finish, 0);
      return;
    }

    const tick = (): void => {
      const raw = (Date.now() - started) / durationMs;
      const held = options.holdAt !== undefined && raw >= options.holdAt;
      const fraction = Math.min(1, held ? (options.holdAt as number) : raw);
      const eased = 1 - Math.pow(1 - fraction, 2);

      if (fraction >= 1) {
        stopTimer(record);
        finish();
        return;
      }

      onEvent({
        type: 'progress',
        files: Math.round(total.files * eased),
        dirs: Math.round(dirCount * eased),
        bytes: Math.round(total.allocated * eased),
        errors: Math.floor(UNREADABLE_DIRS * eased),
        currentPath: dirs[Math.floor(fraction * (dirs.length - 1))] ?? tree.rootPath,
      });

      if (held) {
        stopTimer(record);
      }
    };

    record.timer = setInterval(tick, tickMs);
    tick();
  };

  return {
    async listVolumes() {
      return volumes.map((volume) => ({ ...volume }));
    },

    async pickFolder() {
      return options.pickedFolder === undefined ? '/home/demo' : options.pickedFolder;
    },

    async startScan(scanOptions: ScanOptions, onEvent) {
      const id = nextId++;
      const tree = buildTree({
        seed,
        root: scanOptions.root,
        oneFileSystem: scanOptions.oneFileSystem,
        bytes: bytesFor(scanOptions.root),
        now: clock(),
        dominant: options.dominant,
      });
      const record: ScanRecord = { tree, onEvent, timer: null, running: true };

      scans.set(id, record);
      // Start on the next tick, so the caller has the id before the first event arrives.
      queueMicrotask(() => run(record, id));

      return id;
    },

    async cancelScan(scan) {
      const record = scans.get(scan);

      if (!record?.running) {
        return;
      }

      record.running = false;
      stopTimer(record);
      scans.delete(scan);
      // Like the real scanner, which notices the flag at its next directory: the event follows, it does not interrupt.
      setTimeout(() => record.onEvent({ type: 'cancelled' }), 0);
    },

    async dropScan(scan) {
      const record = scans.get(scan);

      if (record) {
        record.running = false;
        stopTimer(record);
      }

      scans.delete(scan);
    },

    async getNode(scan, node) {
      return toDto(nodeOf(scan, node).node);
    },

    async getChildren(scan, node, offset, limit, sort): Promise<ChildrenPage> {
      const { tree, node: parent } = nodeOf(scan, node);

      if (offset < 0 || limit < 0) {
        return fail('invalid', 'offset and limit must not be negative');
      }

      const sorted = childrenOf(tree, parent).sort(sorter(sort));
      const window = Math.min(limit, MAX_LIMIT);

      return {
        parentId: parent.id,
        total: sorted.length,
        items: sorted.slice(offset, offset + window).map(toDto),
      };
    },

    async getLod(scan, node, maxNodes, maxDepth) {
      const { tree, node: root } = nodeOf(scan, node);

      if (maxNodes < 1 || maxDepth < 0) {
        return fail('invalid', 'maxNodes must be at least 1 and maxDepth must not be negative');
      }

      return lodOf(tree, root, Math.min(maxNodes, MAX_LOD_NODES), Math.min(maxDepth, MAX_LOD_DEPTH));
    },

    async getPath(scan, node) {
      const { tree, node: target } = nodeOf(scan, node);

      return pathOf(tree, target);
    },

    async getSummary(scan, node) {
      const { tree, node: target } = nodeOf(scan, node);

      return summaryOf(tree, target);
    },

    async getSuggestions(scan) {
      return suggestionsOf(scanOf(scan).tree);
    },

    async openInFileManager(scan, node) {
      nodeOf(scan, node);
    },

    async openExternal(link) {
      window.open(EXTERNAL_URLS[link], '_blank', 'noopener');
    },
  };
}
