import { ApiRequestError, type Api } from './client';
import type {
  HistoryCleanupDto,
  HistoryMethodDto,
  HistoryOverviewDto,
  HistorySampleDto,
  HistoryStatus,
} from './types';

/*
 * The mock's stand-in for the history commands of `src-tauri/src/commands/history.rs`: a disk that was cleaned five
 * times in the last 90 days. The series is made once from a fixed seed, so every run draws the same picture.
 *
 * Still different from the backend:
 * - Nothing is recorded: the cleanups the mock runs never appear here.
 * - Restoring only flips the status of the run. Every item comes back.
 */

const GB = 1_000_000_000;

/** The days of the demo series, today included. */
const SERIES_DAYS = 91;
const DISK_TOTAL = 1900 * GB;
const DISK_USED = 1408 * GB;
const VOLUME = { id: 'nvme0n1p2', label: 'nvme0n1p2' };

/** How much the demo disk grows in each week of the series, in GB. */
const WEEKLY_GROWTH = [19, 24, 21, 27, 18, 23, 25, 20, 29, 22, 17, 26, 20];

/** Fewer samples than this, and the backend gives no growth rate. */
const THIN_BELOW = 7;
/** A disk that grows by less than this a day is not growing. */
const NO_GROWTH = 1_000_000;

export type MockHistoryVariant = 'thin' | 'empty' | 'broken' | 'flat';

export interface MockHistoryOptions {
  variant?: MockHistoryVariant;
  /** Time in unix seconds, to fix the dates in tests. */
  now?: () => number;
  /** Test hook: the answer of the Restore confirmation dialog (default: confirmed). */
  confirmRestore?: () => boolean;
}

export type MockHistoryApi = Pick<Api, 'historyOverview' | 'restoreHistoryCleanup'>;

interface Run {
  id: number;
  daysAgo: number;
  what: string[];
  /** Where the items were, for the restore answer. */
  paths: string[];
  freed: number;
  methods: HistoryMethodDto[];
  status: HistoryStatus;
}

const TRASH: HistoryMethodDto = { kind: 'trash' };
const DELETED: HistoryMethodDto = { kind: 'deleted' };

const demoRuns = (): Run[] => [
  {
    id: 5,
    daysAgo: 5,
    what: ['Old recordings', 'Old downloads'],
    paths: ['~/Videos/Old recordings', '~/Downloads/Old downloads'],
    freed: 31 * GB,
    methods: [TRASH],
    status: 'inTrash',
  },
  {
    id: 4,
    daysAgo: 20,
    what: ['Rust build artifacts', 'node_modules'],
    paths: ['~/Projekty/platter/target', '~/Projekty/site/node_modules'],
    freed: 74 * GB,
    methods: [DELETED],
    status: 'permanent',
  },
  {
    id: 3,
    daysAgo: 45,
    what: ['Pacman package cache'],
    paths: ['/var/cache/pacman/pkg'],
    freed: 22 * GB,
    methods: [{ kind: 'tool', command: 'paccache -rk2' }],
    status: 'permanent',
  },
  {
    id: 2,
    daysAgo: 64,
    what: ['Docker dangling images'],
    paths: ['/var/lib/docker'],
    freed: 48 * GB,
    methods: [{ kind: 'tool', command: 'docker prune' }],
    status: 'permanent',
  },
  {
    id: 1,
    daysAgo: 79,
    what: ['Thumbnails', 'pip cache'],
    paths: ['~/.cache/thumbnails', '~/.cache/pip'],
    freed: 12 * GB,
    methods: [DELETED],
    status: 'permanent',
  },
];

/** A run that only moved to the trash has freed nothing on the disk yet. */
const isTrashOnly = (run: Run): boolean => run.methods.every((method) => method.kind === 'trash');

const measuredFreed = (run: Run): number => (isTrashOnly(run) ? 0 : run.freed);

const pad = (value: number): string => String(value).padStart(2, '0');

/** The local date `daysAgo` days before `now`, as `YYYY-MM-DD`, and noon of that day in unix seconds. */
function dayAgo(now: number, daysAgo: number): { day: string; at: number } {
  const today = new Date(now * 1000);
  const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() - daysAgo, 12);

  return {
    day: `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`,
    at: Math.floor(date.getTime() / 1000),
  };
}

interface Series {
  samples: HistorySampleDto[];
  without: number[];
  runs: Run[];
}

/** The seeded pseudo random numbers of the demo series (the same ones as in the mockup). */
function randoms(seed: number): () => number {
  let state = seed;

  return () => {
    state = (state * 16807) % 2147483647;

    return state / 2147483647;
  };
}

function demoSeries(now: number, runs: Run[]): Series {
  const rnd = randoms(11);
  const growth = Array.from(
    { length: SERIES_DAYS - 1 },
    (_, day) => ((WEEKLY_GROWTH[Math.floor(day / 7)] ?? 20) / 7) * (0.45 + 1.1 * rnd()) * GB,
  );
  const totalGrowth = growth.reduce((sum, value) => sum + value, 0);
  const totalFreed = runs.reduce((sum, run) => sum + measuredFreed(run), 0);

  // What the disk would hold with no cleanup: it ends at today's use plus everything the cleanups freed.
  const without = [DISK_USED + totalFreed - totalGrowth];
  for (const step of growth) {
    without.push((without.at(-1) ?? 0) + step);
  }

  const samples = without.map((value, index): HistorySampleDto => {
    const freedSoFar = runs
      .filter((run) => SERIES_DAYS - 1 - run.daysAgo <= index)
      .reduce((sum, run) => sum + measuredFreed(run), 0);

    return {
      day: dayAgo(now, SERIES_DAYS - 1 - index).day,
      used: Math.round(value - freedSoFar),
      total: DISK_TOTAL,
    };
  });

  return { samples, without: without.map(Math.round), runs };
}

function thinSeries(now: number): Series {
  const used = [1405, 1406.5, 1408].map((value) => value * GB);

  return {
    samples: used.map((value, index) => ({
      day: dayAgo(now, 2 - index).day,
      used: value,
      total: DISK_TOTAL,
    })),
    without: used,
    runs: [],
  };
}

function flatSeries(now: number): Series {
  const samples = Array.from({ length: SERIES_DAYS }, (_, index) => ({
    day: dayAgo(now, SERIES_DAYS - 1 - index).day,
    used: DISK_USED,
    total: DISK_TOTAL,
  }));

  return { samples, without: samples.map((sample) => sample.used), runs: [] };
}

const fail = (code: 'notFound' | 'invalid' | 'cancelled', message: string): never => {
  throw new ApiRequestError(code, message);
};

export function createMockHistory(options: MockHistoryOptions = {}): MockHistoryApi {
  const now = options.now ?? (() => Math.floor(Date.now() / 1000));
  const variant = options.variant;
  const confirmRestore = options.confirmRestore ?? (() => true);
  const start = now();
  const runs = variant === undefined ? demoRuns() : [];
  const series =
    variant === 'thin'
      ? thinSeries(start)
      : variant === 'flat'
        ? flatSeries(start)
        : variant === undefined
          ? demoSeries(start, runs)
          : { samples: [], without: [], runs: [] };

  const cleanupOf = (run: Run): HistoryCleanupDto => ({
    id: run.id,
    ...dayAgo(start, run.daysAgo),
    what: run.what,
    freed: run.freed,
    methods: run.methods,
    status: run.status,
    restorable: run.status === 'inTrash' ? run.freed : 0,
  });

  return {
    async historyOverview(days): Promise<HistoryOverviewDto> {
      if (!Number.isInteger(days) || days < 1 || days > 366) {
        return fail('invalid', 'days must be between 1 and 366');
      }

      const today = dayAgo(start, 0).day;

      if (variant === 'broken') {
        return {
          days,
          today,
          samples: [],
          without: [],
          cleanups: [],
          freed: 0,
          cleanupCount: 0,
          restorable: 0,
          thin: true,
          problem: 'The history file could not be read. Nothing is recorded until it is fixed.',
        };
      }

      const first = Math.max(0, series.samples.length - days);
      const samples = series.samples.slice(first);
      const without = series.without.slice(first);
      const cleanups = runs
        .map(cleanupOf)
        .filter((cleanup) => samples.length > 0 && cleanup.day >= (samples[0] as HistorySampleDto).day);
      const thin = samples.length < THIN_BELOW;
      const rate = thin ? 0 : ((without.at(-1) ?? 0) - (without[0] ?? 0)) / (samples.length - 1);
      const last = samples.at(-1);

      return {
        ...(variant === 'empty' ? {} : { volume: VOLUME }),
        days,
        today,
        samples,
        without,
        cleanups,
        freed: runs
          .filter((run) => cleanups.some((cleanup) => cleanup.id === run.id))
          .reduce((sum, run) => sum + measuredFreed(run), 0),
        cleanupCount: cleanups.length,
        restorable: cleanups.reduce((sum, cleanup) => sum + cleanup.restorable, 0),
        ...(rate >= NO_GROWTH && last
          ? { growthPerDay: rate, daysUntilFull: (last.total - last.used) / rate }
          : {}),
        thin,
      };
    },

    async restoreHistoryCleanup(cleanup) {
      const run = runs.find((candidate) => candidate.id === cleanup);

      if (!run) {
        return fail('notFound', `cleanup ${cleanup} is not in the history`);
      }

      if (run.status !== 'inTrash' && run.status !== 'notChecked') {
        return fail('notFound', `cleanup ${cleanup} has nothing left to put back`);
      }

      // The backend's native dialog: declined, nothing is put back and the run stays as it was.
      if (!confirmRestore()) {
        return fail('cancelled', 'The restore was not confirmed.');
      }

      run.status = 'restored';

      return { restored: run.paths.length, items: run.paths.map((path) => ({ path, restored: true })) };
    },
  };
}
