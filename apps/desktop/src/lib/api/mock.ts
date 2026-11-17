import { ApiRequestError, type Api } from './client';
import { createMockAdvisor, type MockAdvisorOptions, type MockLabelRuns } from './mock-advisor';
import { createMockCleanup } from './mock-cleanup';
import { createMockUpdate, type MockUpdateOptions } from './mock-update';
import { createMockHistory, type MockHistoryOptions } from './mock-history';
import { createMockUserRules, type MockUserRulesOptions } from './mock-user-rules';
import { toDto } from './mock-dto';
import { REAL_BYTES, REAL_ROOT, SKEW_BYTES, SKEW_ROOT } from './mock-layouts';
import { lodOf } from './mock-lod';
import { largestDirs } from './mock-summary';
import { suggestionsOf } from './mock-suggestions';
import { buildTree, CATEGORIES, childrenOf, DAY, type MockNode, type MockTree } from './mock-tree';
import type {
  AgeBucket,
  Category,
  ChildrenPage,
  ExternalLink,
  NodeId,
  PathDto,
  ScanEvent,
  ScanId,
  ScanOptions,
  SortKey,
  SummaryDto,
  VolumeDto,
  ScanWarning,
} from './types';
import { MAX_EXCLUDED, MAX_EXCLUDED_PATH_BYTES } from './types';

const GB = 1_000_000_000;

const REPOSITORY = 'https://github.com/MSpiechowicz/platter';

export const EXTERNAL_URLS: Record<ExternalLink, string> = {
  source: REPOSITORY,
  issues: `${REPOSITORY}/issues`,
  license: `${REPOSITORY}/blob/main/LICENSE`,
  releases: `${REPOSITORY}/releases/latest`,
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
  /** Test hook: what the picker for a folder to leave out returns (`null`: cancelled). */
  pickedExcluded?: string | null;
  /** Test hook: warnings the finished scan reports. */
  warnings?: ScanWarning[];
  /** Test hook: timers and starting state of the advisor model's state machine (see `mock-advisor.ts`). */
  advisor?: MockAdvisorOptions;
  /** Test hook: the rules the user's rule list starts with, or a broken rules file (see `mock-user-rules.ts`). */
  userRules?: MockUserRulesOptions;
  /** Test hook: the history the History view shows (see `mock-history.ts`). */
  history?: MockHistoryOptions;
  /** Test hook: the update the mock finds (default: none; see `mock-update.ts`). */
  update?: MockUpdateOptions;
  /** Test hook: the answer of the Undo confirmation dialog (default: confirmed; see `mock-cleanup.ts`). */
  confirmUndo?: () => boolean;
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
/** The longest `getChildren` name filter, in UTF-8 bytes. */
const MAX_NAME_FILTER_BYTES = 1024;

const encoder = new TextEncoder();
const utf8Length = (text: string): number => encoder.encode(text).length;

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
  const prefix = tree.rootPath.replace(/\/+$/, '');

  return {
    path: rest.length === 0 ? tree.rootPath : `${prefix}/${rest.join('/')}`,
    segments: [
      { id: root.id, name: root.name },
      ...chain.slice(1).map((entry) => ({ id: entry.id, name: entry.name })),
    ],
  };
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
      .sort(
        (a, b) =>
          b.allocated - a.allocated || CATEGORIES.indexOf(a.category) - CATEGORIES.indexOf(b.category),
      ),
    byAge: [...byAge].map(([bucket, value]) => ({ bucket, ...value })),
    largestDirs: largestDirs(tree, node).map(toDto),
  };
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
  const warnings = options.warnings;
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

  /** The labels the advisor made, by scan; they go with the scan, and when a cleanup outdates it. */
  const labelRuns: MockLabelRuns = new Map();
  const outdated = new Set<ScanId>();

  const userRules = createMockUserRules(
    {
      treeOf: (scan) => scanOf(scan).tree,
      isRunning: (scan) => scanOf(scan).running,
      changed: () => dropPlans(),
    },
    options.userRules,
  );
  const { view: userRulesView, ...userRulesApi } = userRules;

  const history = createMockHistory({ now: clock, ...options.history });
  const update = createMockUpdate(options.update);

  const { advisorEnabled, ...advisor } = createMockAdvisor(
    {
      treeOf: (scan) => scanOf(scan).tree,
      pathOf: (tree, node) => pathOf(tree, node).path,
      summary: summaryOf,
      isRunning: (scan) => scanOf(scan).running,
      outdated,
      runs: labelRuns,
      userRules: () => userRulesView().rules,
      rulesProblem: () => userRulesView().problem,
    },
    options.advisor,
  );

  /** What the suggestions and plans see of the labels: nothing while the advisor is switched off. */
  const labelsOf = (scan: ScanId) => (advisorEnabled() ? (labelRuns.get(scan)?.labels ?? []) : []);

  const { dropPlans, ...cleanupApi } = createMockCleanup({
    treeOf: (scan) => scanOf(scan).tree,
    pathOf: (tree, node) => pathOf(tree, node).path,
    durationMs,
    labelsOf,
    advisorEnabled,
    userRules: userRulesView,
    outdated,
    outdate: (scan) => {
      outdated.add(scan);
      labelRuns.delete(scan);
    },
    confirmUndo: options.confirmUndo,
  });

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
        ...(warnings && warnings.length > 0 ? { warnings } : {}),
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

    async pickExcludedFolder() {
      return options.pickedExcluded === undefined ? '/home/demo/Videos' : options.pickedExcluded;
    },

    async startScan(scanOptions: ScanOptions, onEvent) {
      // As `start_scan` checks them, before an id is taken.
      const excluded = scanOptions.excluded ?? [];

      if (excluded.length > MAX_EXCLUDED) {
        return fail('invalid', `leave out at most ${MAX_EXCLUDED} folders`);
      }

      const bad = excluded.find(
        (path) => !path.startsWith('/') || utf8Length(path) > MAX_EXCLUDED_PATH_BYTES,
      );

      if (bad !== undefined) {
        return fail('invalid', `${bad} is not an absolute path of a folder`);
      }

      const id = nextId++;
      const tree = buildTree({
        seed,
        root: scanOptions.root,
        oneFileSystem: scanOptions.oneFileSystem,
        excluded,
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
      labelRuns.delete(scan);
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
      labelRuns.delete(scan);
    },

    async getNode(scan, node) {
      return toDto(nodeOf(scan, node).node);
    },

    async getChildren(scan, node, offset, limit, sort, query): Promise<ChildrenPage> {
      const { tree, node: parent } = nodeOf(scan, node);

      if (offset < 0 || limit < 0) {
        return fail('invalid', 'offset and limit must not be negative');
      }

      if (query !== undefined && utf8Length(query) > MAX_NAME_FILTER_BYTES) {
        return fail('invalid', `the name filter must not be longer than ${MAX_NAME_FILTER_BYTES} bytes`);
      }

      const needle = query === undefined || query.trim() === '' ? undefined : query.toLowerCase();
      const children = childrenOf(tree, parent);
      const sorted = (
        needle === undefined
          ? children
          : children.filter((child) => child.name.toLowerCase().includes(needle))
      ).sort(sorter(sort));
      const window = Math.min(limit, MAX_LIMIT);

      return {
        parentId: parent.id,
        total: sorted.length,
        items: sorted.slice(offset, offset + window).map(toDto),
      };
    },

    async getLod(scan, node, maxNodes, maxDepth) {
      const { tree, node: root } = nodeOf(scan, node);

      // The backend takes unsigned numbers, so a negative one never arrives; zero nodes still shows the root.
      if (maxNodes < 0 || maxDepth < 0) {
        return fail('invalid', 'maxNodes and maxDepth must not be negative');
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
      return suggestionsOf(scanOf(scan).tree, labelsOf(scan), userRulesView().rules);
    },

    ...cleanupApi,

    ...advisor,

    ...userRulesApi,

    ...history,

    ...update,

    async openInFileManager(scan, node) {
      nodeOf(scan, node);
    },

    async openExternal(link) {
      window.open(EXTERNAL_URLS[link], '_blank', 'noopener');
    },
  };
}
