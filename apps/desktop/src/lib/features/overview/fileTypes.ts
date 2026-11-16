import { CATEGORY_META } from '../../categories';
import type { SummaryDto } from '../../api/types';
import { SLATE } from '../../palette';

/** One row of the "By file type" card. */
export interface TypeRow {
  key: string;
  label: string;
  color: string;
  allocated: number;
  /** Share of everything the categories add up to, 0 to 1. */
  share: number;
  /** For the "Smaller types" row: the categories folded into it, biggest first. */
  folded?: { label: string; allocated: number }[];
}

/** Rows shown before the smaller categories are folded into one. */
export const MAX_TYPE_ROWS = 8;

/**
 * Categories biggest first. When there are more than `MAX_TYPE_ROWS`, the biggest stay and the rest become one
 * final "Smaller types" row, so the card stays compact. Folding a single category would not save a row, so it
 * only happens for two or more.
 */
export function typeRows(byCategory: SummaryDto['byCategory'], max: number = MAX_TYPE_ROWS): TypeRow[] {
  const sorted = byCategory.filter((entry) => entry.allocated > 0).sort((a, b) => b.allocated - a.allocated);
  const whole = sorted.reduce((sum, entry) => sum + entry.allocated, 0);
  const share = (allocated: number): number => (whole > 0 ? allocated / whole : 0);

  const row = (entry: SummaryDto['byCategory'][number]): TypeRow => ({
    key: entry.category,
    label: CATEGORY_META[entry.category].label,
    color: CATEGORY_META[entry.category].color,
    allocated: entry.allocated,
    share: share(entry.allocated),
  });

  if (sorted.length <= max + 1) {
    return sorted.map(row);
  }

  const rest = sorted.slice(max).map(row);
  const allocated = rest.reduce((sum, entry) => sum + entry.allocated, 0);

  return [
    ...sorted.slice(0, max).map(row),
    {
      key: 'smaller',
      label: 'Smaller types',
      color: SLATE,
      allocated,
      share: share(allocated),
      folded: rest.map(({ label, allocated: size }) => ({ label, allocated: size })),
    },
  ];
}
