import type { HistoryCleanupDto, HistorySampleDto } from '../../api/types';
import { dayNumber, dayText } from '../history/chart';

/** The ranges the card offers, in days. */
export const RANGES = [30, 91, 182] as const;

export type Range = (typeof RANGES)[number];

export const DEFAULT_RANGE: Range = 91;

/** A month for the rate, as the card says "per month". */
const MONTH_DAYS = 30;

/**
 * What a day's bar says. `grew`: the used space stayed or rose. `cleanup`: it fell on a day with a cleanup.
 * `shrank`: it fell with no cleanup (something was deleted outside Platter).
 */
export type DayKind = 'grew' | 'cleanup' | 'shrank';

export interface UsageDay {
  /** The local day, `YYYY-MM-DD`. */
  day: string;
  /** Used space at the end of the day. Undefined when no sample was taken. */
  used?: number;
  /** Change since the day before. Undefined (a gap) unless both days have a sample. */
  delta?: number;
  /**
   * The two parts of the bar of the day, both from the zero line: what was written (up) and what was taken away
   * (down). On a cleanup day the down part is what the cleanups freed and the up part what grew besides, so the
   * day's net is the difference. A gap has neither, except that a cleanup is always drawn.
   */
  up: number;
  down: number;
  /** Undefined for a gap with no cleanup: there is nothing to draw. */
  kind?: DayKind;
  /** The cleanups of this day, newest first, and what they freed together. */
  cleanups: HistoryCleanupDto[];
  freed: number;
  what: string[];
}

export interface UsageDaysInput {
  samples: readonly HistorySampleDto[];
  cleanups: readonly HistoryCleanupDto[];
  /** The last day of the range. */
  today: string;
}

export interface UsageDaysModel {
  /** One entry per calendar day of the range, oldest first. */
  days: UsageDay[];
  /** The biggest rise and the biggest fall of a day: what the two halves of the plot are scaled to. 0 when there is none. */
  maxUp: number;
  maxDown: number;
  /** Last used minus first used in the range. */
  net: number;
  /** What the cleanups inside the net change freed: those after the first measured day of the range, through the last. */
  freed: number;
  /** What was written in the range: the net change with the cleanups inside it taken out again, never below 0. */
  added: number;
  perMonth: number;
  /** Days of the range that have a bar: a change, or a cleanup. */
  bars: number;
}

interface Parts {
  up: number;
  down: number;
  kind?: DayKind;
}

/**
 * A cleanup day shows both halves: what it freed, and what grew besides. Even a gap day shows its cleanup. A cleanup
 * that freed nothing changes nothing on the day, so the day is read as any other.
 */
function partsOf(delta: number | undefined, freed: number): Parts {
  if (freed > 0) {
    return { up: delta === undefined ? 0 : Math.max(0, delta + freed), down: freed, kind: 'cleanup' };
  }

  if (delta === undefined) {
    return { up: 0, down: 0 };
  }

  return delta >= 0 ? { up: delta, down: 0, kind: 'grew' } : { up: 0, down: -delta, kind: 'shrank' };
}

/**
 * The days of the last `range` days up to `input.today`, each with its change in used space. A day has a change only
 * when it and the day before it have a sample, so a day the app was closed is a gap and not a long bar.
 */
export function usageDays(input: UsageDaysInput, range: number): UsageDaysModel {
  const last = dayNumber(input.today);
  const first = last - range + 1;

  const usedOn = new Map<number, number>();
  for (const sample of input.samples) {
    usedOn.set(dayNumber(sample.day), sample.used);
  }

  const cleanupsOn = new Map<number, HistoryCleanupDto[]>();
  for (const cleanup of input.cleanups) {
    const at = dayNumber(cleanup.day);
    cleanupsOn.set(at, [...(cleanupsOn.get(at) ?? []), cleanup]);
  }

  const days: UsageDay[] = [];

  for (let at = first; at <= last; at++) {
    const used = usedOn.get(at);
    const before = usedOn.get(at - 1);
    const cleanups = cleanupsOn.get(at) ?? [];
    const delta = used !== undefined && before !== undefined ? used - before : undefined;

    const freed = cleanups.reduce((sum, cleanup) => sum + cleanup.freed, 0);

    days.push({
      day: dayText(at),
      used,
      delta,
      ...partsOf(delta, freed),
      cleanups,
      freed,
      what: cleanups.flatMap((cleanup) => cleanup.what),
    });
  }

  const measured = days.filter((day) => day.used !== undefined);
  const firstMeasured = measured[0];
  const lastMeasured = measured.at(-1);
  let net = 0;
  let freed = 0;
  let added = 0;

  if (firstMeasured && lastMeasured && measured.length > 1) {
    net = (lastMeasured.used ?? 0) - (firstMeasured.used ?? 0);

    // Only a cleanup whose effect is inside the net change counts: one before the first measured day (or after the
    // last) is not in it. So the card can show the three as a sum: added − freed = net.
    freed = days
      .filter((day) => day.day > firstMeasured.day && day.day <= lastMeasured.day)
      .reduce((sum, day) => sum + day.freed, 0);

    added = Math.max(0, net + freed);
  }

  return {
    days,
    maxUp: Math.max(0, ...days.map((day) => day.up)),
    maxDown: Math.max(0, ...days.map((day) => day.down)),
    net,
    freed,
    added,
    perMonth: (added / range) * MONTH_DAYS,
    bars: days.filter((day) => day.kind !== undefined).length,
  };
}
