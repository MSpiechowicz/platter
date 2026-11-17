import type { HistoryCleanupDto, HistorySampleDto } from '../../api/types';

/** The drawing is 1000 by 300 units and scales with its card. The plot sits inside; the end labels use the right margin. */
export const CHART = {
  width: 1000,
  height: 300,
  left: 70,
  right: 870,
  top: 24,
  bottom: 250,
  dateY: 276,
} as const;

/** One end label is a figure and a caption under it: this much room, with the lowest and highest place it may take. */
export const LABEL_GAP = 38;
const LABEL_TOP = 16;
const LABEL_BOTTOM = 262;

/** The most date labels along the bottom. */
const MAX_DATES = 7;

/** Room above and below the data, as a share of its range. The disk fills towards the top, so the lower margin is bigger. */
const MARGIN_BELOW = 0.2;
const MARGIN_ABOVE = 0.08;
/** A line that does not move still gets a range to be drawn in: this share of its value. */
const FLAT_RANGE = 0.02;

export interface Tick {
  value: number;
  y: number;
}

export interface DateLabel {
  x: number;
  day: string;
}

export interface Marker {
  cleanup: HistoryCleanupDto;
  x: number;
  y: number;
}

export interface EndLabel {
  /** Where the dot is. */
  x: number;
  y: number;
  /** Where the label is: the same, moved off a label that is too close. */
  labelY: number;
  value: number;
}

export interface ChartModel {
  ticks: Tick[];
  dates: DateLabel[];
  /** The line of the disk, with a vertical step where a cleanup freed space. */
  usedPath: string;
  withoutPath: string;
  /** The area between the two lines. */
  gapPath: string;
  markers: Marker[];
  used: EndLabel;
  without: EndLabel;
  /** What the cleanups saved so far: the gap at the right end. */
  saved: number;
  /** Where the "saved" bracket and its text sit (the middle of the two dots). Undefined when nothing was saved. */
  savedBracket?: { x: number; y1: number; y2: number; textY: number };
}

export interface ChartInput {
  samples: readonly HistorySampleDto[];
  without: readonly number[];
  cleanups: readonly HistoryCleanupDto[];
}

/** The step between the ticks of an axis from `lo` to `hi`: 1, 2 or 5 times a power of ten of `unit`, for about `target` ticks. */
export function niceStep(lo: number, hi: number, unit: number, target = 5): number {
  const rough = Math.max(hi - lo, unit * 1e-6) / unit / target;
  const power = 10 ** Math.floor(Math.log10(rough));

  return ([1, 2, 5, 10].find((factor) => factor * power >= rough) ?? 10) * power * unit;
}

/** The multiples of `step` from `lo` to `hi`. */
export function ticksBetween(lo: number, hi: number, step: number): number[] {
  const ticks: number[] = [];

  for (let value = Math.ceil(lo / step - 1e-9) * step; value <= hi + step * 1e-9; value += step) {
    ticks.push(value);
  }

  return ticks;
}

/**
 * The words for the ticks of the axis. `size` writes a size the way the app does, which rounds a TB to two decimals:
 * ticks a few GB apart would read the same, so then they are written in GB, with a decimal when they are less than
 * 1 GB apart.
 */
export function tickLabels(
  values: readonly number[],
  size: (bytes: number) => string,
  unit: number,
  name: string,
): string[] {
  const labels = values.map(size);

  if (new Set(labels).size === labels.length) {
    return labels;
  }

  const gap = values.length > 1 ? ((values[1] ?? 0) - (values[0] ?? 0)) / unit : 1;
  const decimals = gap < 1 ? 1 : 0;

  return values.map((value) => `${(value / unit).toFixed(decimals)} ${name}`);
}

/**
 * Moves labels apart so that none is closer than `gap` to the next, within `lo` to `hi`. Labels keep their order;
 * the result is in the order of the input.
 */
export function spreadLabels(ys: readonly number[], gap: number, lo: number, hi: number): number[] {
  const order = ys.map((_, index) => index).sort((a, b) => (ys[a] ?? 0) - (ys[b] ?? 0));
  const placed = order.map((index) => Math.min(Math.max(ys[index] ?? 0, lo), hi));

  for (let pass = 0; pass < 20; pass++) {
    let moved = false;

    for (let at = 1; at < placed.length; at++) {
      const short = gap - ((placed[at] ?? 0) - (placed[at - 1] ?? 0));

      if (short > 1e-9) {
        placed[at - 1] = Math.max(lo, (placed[at - 1] ?? lo) - short / 2);
        placed[at] = Math.min(hi, (placed[at] ?? hi) + short / 2);
        moved = true;
      }
    }

    if (!moved) {
      break;
    }
  }

  // The halving above only converges: make the gaps exact, pushing down first and then back up from the bottom.
  for (let at = 1; at < placed.length; at++) {
    placed[at] = Math.max(placed[at] ?? 0, (placed[at - 1] ?? 0) + gap);
  }

  for (let at = placed.length - 1; at >= 0; at--) {
    placed[at] = Math.min(placed[at] ?? hi, at === placed.length - 1 ? hi : (placed[at + 1] ?? hi) - gap);
  }

  const result: number[] = [];
  order.forEach((index, at) => (result[index] = placed[at] ?? 0));

  return result;
}

const fixed = (value: number): string => value.toFixed(1);

const pathOf = (points: readonly { x: number; y: number }[]): string =>
  points.map((point, index) => `${index === 0 ? 'M' : 'L'}${fixed(point.x)},${fixed(point.y)}`).join('');

/**
 * Everything the chart draws, in drawing units. `undefined` without a sample. The disk's line steps down at the
 * sample where the gap to the "without cleanups" line grows: that is the day space was freed.
 */
export function layoutChart(input: ChartInput, unit: number): ChartModel | undefined {
  const { samples, cleanups } = input;
  const rows = samples.slice(0, input.without.length).map((sample, index) => ({
    day: sample.day,
    used: sample.used,
    without: input.without[index] ?? sample.used,
  }));
  const first = rows[0];
  const last = rows.at(-1);

  if (!first || !last) {
    return undefined;
  }

  const low = Math.min(...rows.map((row) => row.used));
  const high = Math.max(...rows.map((row) => Math.max(row.used, row.without)));
  const range = Math.max(high - low, high * FLAT_RANGE, 1);
  // The axis runs between round values, so the top and bottom lines are labelled.
  const step = niceStep(low - range * MARGIN_BELOW, high + range * MARGIN_ABOVE, unit);
  const lo = Math.floor((low - range * MARGIN_BELOW) / step) * step;
  const hi = Math.ceil((high + range * MARGIN_ABOVE) / step) * step;

  // Days have their place on the axis by date: a day with no sample leaves a gap, not a squeeze.
  const firstDay = dayNumber(first.day);
  const span = dayNumber(last.day) - firstDay;
  const xOfDay = (day: number): number =>
    span <= 0 ? CHART.right : CHART.left + ((day - firstDay) / span) * (CHART.right - CHART.left);
  const x = (index: number): number => xOfDay(dayNumber(rows[index]?.day ?? last.day));
  const y = (value: number): number => CHART.bottom - ((value - lo) / (hi - lo)) * (CHART.bottom - CHART.top);

  // The line of the disk, and the same points reversed for the edge of the gap.
  const usedPoints: { x: number; y: number }[] = [];
  let before = first;

  rows.forEach((row, index) => {
    const freed = row.without - row.used - (before.without - before.used);

    if (index > 0 && freed > 0) {
      usedPoints.push({ x: x(index), y: y(row.used + freed) });
    }

    usedPoints.push({ x: x(index), y: y(row.used) });
    before = row;
  });

  const withoutPoints = rows.map((row, index) => ({ x: x(index), y: y(row.without) }));
  const gapPath = `${pathOf([...withoutPoints, ...[...usedPoints].reverse()])}Z`;

  const markers: Marker[] = [];

  for (const cleanup of cleanups) {
    const index = rows.findIndex((row) => row.day >= cleanup.day);
    const row = rows[index];

    if (row) {
      markers.push({ cleanup, x: x(index), y: y(row.used) });
    }
  }

  const usedEnd = { x: x(rows.length - 1), y: y(last.used), value: last.used };
  const withoutEnd = { x: x(rows.length - 1), y: y(last.without), value: last.without };
  const saved = withoutEnd.value - usedEnd.value;
  const bracketed = saved > 0 && usedEnd.y - withoutEnd.y > 20;

  // With nothing saved the two ends are one dot, and one label says it.
  const [usedLabel = usedEnd.y, withoutLabel = withoutEnd.y] =
    saved > 0
      ? spreadLabels([usedEnd.y, withoutEnd.y], LABEL_GAP, LABEL_TOP, LABEL_BOTTOM)
      : [Math.min(Math.max(usedEnd.y, LABEL_TOP), LABEL_BOTTOM)];

  return {
    ticks: ticksBetween(lo, hi, step).map((value) => ({ value, y: y(value) })),
    dates: dateDays(span).map((offset) => ({
      x: xOfDay(firstDay + offset),
      day: dayText(firstDay + offset),
    })),
    usedPath: pathOf(usedPoints),
    withoutPath: pathOf(withoutPoints),
    gapPath,
    markers,
    used: { ...usedEnd, labelY: usedLabel },
    without: { ...withoutEnd, labelY: withoutLabel },
    saved: Math.max(0, saved),
    savedBracket: bracketed
      ? {
          x: usedEnd.x,
          y1: withoutEnd.y + 8,
          y2: usedEnd.y - 8,
          textY: (withoutEnd.y + usedEnd.y) / 2,
        }
      : undefined,
  };
}

const DAY_MS = 86_400_000;

/** Whole days since 1970 of a local day `YYYY-MM-DD`, counted in UTC so that no time zone or clock change shifts it. */
export function dayNumber(day: string): number {
  const [year, month, date] = day.split('-').map(Number);

  return Math.round(Date.UTC(year ?? 1970, (month ?? 1) - 1, date ?? 1) / DAY_MS);
}

/** The day `YYYY-MM-DD` of a day number. */
export function dayText(number: number): string {
  return new Date(number * DAY_MS).toISOString().slice(0, 10);
}

/** Where dates are written under the axis, in days after the first: evenly spaced, and clear of the right end. */
function dateDays(span: number): number[] {
  if (span <= 0) {
    return [0];
  }

  const step = Math.max(1, Math.ceil(span / (MAX_DATES - 1)));
  const offsets: number[] = [];

  for (let offset = 0; offset < span; offset += step) {
    offsets.push(offset);
  }

  // The last label would sit under the end labels, and a close one would touch it.
  return offsets.filter((offset) => span - offset >= step / 2);
}
