import { ApiRequestError, type Api } from './client';
import { createMockAdvisor, type MockAdvisorOptions, type MockLabelRuns } from './mock-advisor';
import { createMockCleanup } from './mock-cleanup';
import { createMockUpdate, type MockUpdateOptions } from './mock-update';
import { createMockHistory, type MockHistoryOptions } from './mock-history';
import { createMockUserRules, type MockUserRulesOptions } from './mock-user-rules';
import { toDto } from './mock-dto';
import { isAbsolutePath, isSamePath, joinPath } from '../paths';
import { REAL_BYTES, REAL_ROOT, SKEW_BYTES, SKEW_ROOT } from './mock-layouts';
import { nameFilter, sorter } from './mock-children';
import { lodOf } from './mock-lod';
import { largestDirs } from './mock-summary';
import { suggestionsOf } from './mock-suggestions';
import { buildTree, CATEGORIES, childrenOf, DAY, type MockNode, type MockTree } from './mock-tree';
import type {
  AgeBucket,
  Category,
  ChildrenPage,
  DiskAccess,
  ExternalLink,
  LastScanDto,
  NodeId,
  OsName,
  PathDto,
  ScanEvent,
  ScanId,
  ScanOptions,
  SummaryDto,
  Totals,
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

/** The volumes of a Windows machine (`?os=windows`): drives with a letter and a label, no device path. */
export const MOCK_WINDOWS_VOLUMES: VolumeDto[] = [
  {
    id: 'C:',
    mountPoint: 'C:\\',
    label: 'Windows',
    device: 'C:',
    fsType: 'NTFS',
    total: 953 * GB,
    available: 241 * GB,
    removable: false,
  },
  {
    id: 'D:',
    mountPoint: 'D:\\',
    label: 'Data',
    device: 'D:',
    fsType: 'NTFS',
    total: 1863 * GB,
    available: 702 * GB,
    removable: false,
  },
];

/** The volumes of a Mac (`?os=macos`): the system disk mounted at `/`, and a stick under `/Volumes`. */
export const MOCK_MAC_VOLUMES: VolumeDto[] = [
  {
    id: 'disk3s1s1',
    mountPoint: '/',
    label: 'Macintosh HD',
    device: '/dev/disk3s1s1',
    fsType: 'apfs',
    total: 494 * GB,
    available: 163 * GB,
    removable: false,
  },
  {
    id: 'disk4s1',
    mountPoint: '/Volumes/USB',
    label: 'USB',
    device: '/dev/disk4s1',
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
  /** Test hook: the system the mock stands for: the volumes, the folders the pickers offer and the demo disk follow it (default `linux`). */
  os?: OsName;
  /** Test hook: warnings the finished scan reports. */
  warnings?: ScanWarning[];
  /** Test hook: timers and starting state of the advisor model's state machine (see `mock-advisor.ts`). */
  advisor?: MockAdvisorOptions;
  /** Test hook: the rules the user's rule list starts with, or a broken rules file (see `mock-user-rules.ts`). */
  userRules?: MockUserRulesOptions;
  /** Test hook: the history the History view shows (see `mock-history.ts`). */
  history?: MockHistoryOptions;
  /**
   * Test hook: what the Full Disk Access check answers (default: `granted`). Opening the settings switches it to
   * `granted`, as if the user turned the access on there.
   */
  access?: DiskAccess;
  /** Test hook: the update the mock finds (default: none; see `mock-update.ts`). */
  update?: MockUpdateOptions;
  /** Test hook: a scan saved by an earlier run, which `openLastScan` opens (default: none). */
  lastScan?: MockLastScan;
  /** Test hook: the answer of the Undo confirmation dialog (default: confirmed; see `mock-cleanup.ts`). */
  confirmUndo?: () => boolean;
  /** Test hook: the answer of the "Forget the saved scan?" dialog (default: confirmed). */
  confirmForget?: () => boolean;
}

/** The saved scan of an earlier run: fields left out take the demo values (the whole disk, as it was an hour ago). */
export interface MockLastScan {
  options?: ScanOptions;
  /** A cleanup ran since, so the scan no longer describes the disk. */
  outdated?: boolean;
  /** When it was scanned, in unix seconds. */
  scannedAt?: number;
}

/** Elapsed time reported for an instant scan, so the header does not read "in 0 s". */
const INSTANT_SCAN_ELAPSED_MS = 2140;
const FOLDER_BYTES = 186 * GB;

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

  return {
    path: rest.reduce(joinPath, tree.rootPath),
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

/**
 * The totals of a scan, as the backend counts them: every directory and mount except the root, and an error for each
 * folder that could not be read and for each warning (a warning counts as an error too).
 */
function totalsOf(tree: MockTree, warnings: ScanWarning[] | undefined): Totals {
  const total = tree.nodes[0] as MockNode;
  const dirs = tree.nodes.filter((node) => node.kind === 'dir' || node.kind === 'mount').length - 1;
  const errors = tree.nodes.filter((node) => node.unreadable === true).length + (warnings?.length ?? 0);

  return { allocated: total.allocated, apparent: total.apparent, files: total.files, dirs, errors };
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
  const demoVolumes = { windows: MOCK_WINDOWS_VOLUMES, macos: MOCK_MAC_VOLUMES, linux: MOCK_VOLUMES };
  const demoHomes = { windows: 'C:\\Users\\demo', macos: '/Users/demo', linux: '/home/demo' };
  const os = options.os ?? 'linux';
  let access: DiskAccess = options.access ?? 'granted';
  const volumes = options.volumes ?? demoVolumes[os];
  const demoHome = demoHomes[os];
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

    const volume = volumes.find((candidate) => isSamePath(candidate.mountPoint, root));

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
    const totals = totalsOf(tree, warnings);
    const started = Date.now();

    const finish = (): void => {
      record.running = false;
      onEvent({
        type: 'finished',
        scanId: id,
        rootId: 0,
        totals,
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
        dirs: Math.round(totals.dirs * eased),
        bytes: Math.round(total.allocated * eased),
        errors: Math.floor(totals.errors * eased),
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
      return options.pickedFolder === undefined ? demoHome : options.pickedFolder;
    },

    async pickExcludedFolder() {
      return options.pickedExcluded === undefined ? joinPath(demoHome, 'Videos') : options.pickedExcluded;
    },

    async startScan(scanOptions: ScanOptions, onEvent) {
      // As `start_scan` checks them, before an id is taken.
      const excluded = scanOptions.excluded ?? [];

      if (excluded.length > MAX_EXCLUDED) {
        return fail('invalid', `leave out at most ${MAX_EXCLUDED} folders`);
      }

      const bad = excluded.find(
        (path) => !isAbsolutePath(path) || utf8Length(path) > MAX_EXCLUDED_PATH_BYTES,
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
        os,
      });
      const record: ScanRecord = { tree, onEvent, timer: null, running: true };

      scans.set(id, record);
      // Start on the next tick, so the caller has the id before the first event arrives.
      queueMicrotask(() => run(record, id));

      return id;
    },

    async forgetLastScan() {
      if (!(options.confirmForget ?? (() => true))()) {
        throw new ApiRequestError('cancelled', 'forgetting the saved scan was not confirmed');
      }

      options.lastScan = undefined;
    },

    async openLastScan() {
      const saved = options.lastScan;

      if (!saved) {
        return null;
      }

      const scanOptions = saved.options ?? { root: volumes[0]?.mountPoint ?? '/', oneFileSystem: true };
      const id = nextId++;
      const tree = buildTree({
        seed,
        root: scanOptions.root,
        oneFileSystem: scanOptions.oneFileSystem,
        excluded: scanOptions.excluded ?? [],
        bytes: bytesFor(scanOptions.root),
        now: clock(),
        dominant: options.dominant,
        os,
      });
      const totals = totalsOf(tree, warnings);

      scans.set(id, { tree, onEvent: () => {}, timer: null, running: false });

      if (saved.outdated) {
        outdated.add(id);
      }

      const dto: LastScanDto = {
        scanId: id,
        rootId: 0,
        totals,
        elapsedMs: INSTANT_SCAN_ELAPSED_MS,
        ...(warnings && warnings.length > 0 ? { warnings } : {}),
        options: scanOptions,
        outdated: saved.outdated ?? false,
        scannedAt: saved.scannedAt ?? clock() - 3600,
      };

      return dto;
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

      const matches = nameFilter(query);
      const children = childrenOf(tree, parent);
      const sorted = (matches === undefined ? children : children.filter(matches)).sort(sorter(sort));
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

    async getSuggestions(scan, units) {
      return suggestionsOf(scanOf(scan).tree, labelsOf(scan), userRulesView().rules, units);
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

    async diskAccess() {
      return access;
    },

    async openDiskAccessSettings() {
      if (os !== 'macos') {
        throw new ApiRequestError('invalid', 'Full Disk Access is a setting of macOS only');
      }

      access = 'granted';
    },
  };
}
