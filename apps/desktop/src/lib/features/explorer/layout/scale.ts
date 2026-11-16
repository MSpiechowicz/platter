/**
 * The readable-minimum scale. Pure proportional areas make a 1% folder on a 1 TB disk an unreadable sliver, so the
 * biggest few small folders get the area they need to show a name and a size, and everything else shares what is
 * left, still in proportion. This is the only place that draws anything larger than it is.
 */

/** A tile that shows a name and a size needs about this much room, in pixels. */
export const READABLE_WIDTH = 110;
export const READABLE_HEIGHT = 46;
/** The gap d3 leaves between two tiles; the readable room is on top of it. */
const TILE_GAP = 3;
/** At most this many top-level tiles are made larger than they are; the rest go to "N smaller items". */
export const MAX_BOOSTED = 8;
/** Under this share of the folder nothing is drawn on its own, however much room there is. */
export const MIN_SHARE = 0.0005;
/** The boosted tiles together never take more than this share of the map. */
const BOOST_BUDGET = 0.4;

/** The smallest share of a map of `width` by `height` pixels that still holds a readable tile. */
export function minFraction(width: number, height: number): number {
  if (!(width > 0) || !(height > 0)) {
    return 0;
  }

  return ((READABLE_WIDTH + TILE_GAP) * (READABLE_HEIGHT + TILE_GAP)) / (width * height);
}

export interface Scaled {
  /** Display weights: they add up to the sum of the input shares (1 when the sizes are not empty). */
  weights: number[];
  /** True where the item is drawn larger than its true share. */
  boosted: boolean[];
}

/**
 * Display weights for items of the given sizes, as shares of their total. An item with less than `minShare` is
 * lifted to `minShare`; the others are scaled down by one common factor to pay for it, so a bigger item is never
 * drawn smaller than a smaller one. When nothing is below the minimum the weights are the true shares.
 * Items with no size stay at nothing.
 */
export function readableWeights(sizes: readonly number[], minShare: number): Scaled {
  const total = sizes.reduce((sum, size) => sum + Math.max(0, size), 0);
  const shares = sizes.map((size) => (total > 0 ? Math.max(0, size) / total : 0));
  const counted = shares.filter((share) => share > 0).length;
  const lifted = shares.filter((share) => share > 0 && share < minShare).length;

  // However many need lifting, together they never take more than the budget.
  const floor = lifted === 0 ? 0 : Math.min(minShare, BOOST_BUDGET / lifted, 1 / Math.max(1, counted));

  // Nothing to lift, or everything is small together: the true shares are as readable as they get.
  if (lifted === 0 || lifted === counted || floor <= 0) {
    return { weights: shares, boosted: shares.map(() => false) };
  }

  // Find the factor `scale` for which the sum of max(floor, scale * share) is 1. The sum grows with `scale`.
  const ascending = shares.filter((share) => share > 0).sort((a, b) => a - b);
  let scale = 1;

  for (let low = 0; low <= ascending.length; low++) {
    const rest = ascending.slice(low).reduce((sum, share) => sum + share, 0);
    const candidate = rest > 0 ? (1 - low * floor) / rest : 1;
    const smallestKept = ascending[low];

    if (candidate > 0 && (smallestKept === undefined || smallestKept * candidate >= floor - 1e-12)) {
      scale = candidate;
      break;
    }
  }

  const weights = shares.map((share) => (share > 0 ? Math.max(floor, share * scale) : 0));

  return {
    weights,
    boosted: weights.map((weight, index) => weight > (shares[index] as number) * (1 + 1e-9)),
  };
}

export interface Split {
  /** Indices of the items drawn on their own, in input order. */
  kept: number[];
  /** Indices of the items that go into the "N smaller items" tile. */
  folded: number[];
}

/**
 * Decides which items are drawn on their own: those with a readable share, and at most `MAX_BOOSTED` of the smaller
 * ones (the biggest of them). Anything under `MIN_SHARE` of the total, and the smaller ones beyond the limit, are
 * folded. `protectedIndex` (the backend's own "smaller items" node) is never folded: it is already that tile.
 */
export function splitReadable(
  sizes: readonly number[],
  minShare: number,
  protectedIndex?: number,
  limit: number = MAX_BOOSTED,
): Split {
  const total = sizes.reduce((sum, size) => sum + Math.max(0, size), 0);
  const candidates = sizes
    .map((size, index) => ({ index, share: total > 0 ? Math.max(0, size) / total : 0 }))
    .filter(({ index, share }) => share > 0 && share < minShare && index !== protectedIndex)
    .sort((a, b) => b.share - a.share || a.index - b.index);

  const folded = new Set(
    candidates.filter(({ share }, rank) => rank >= limit || share < MIN_SHARE).map(({ index }) => index),
  );

  return {
    kept: sizes
      .map((_, index) => index)
      .filter((index) => !folded.has(index) && (sizes[index] as number) > 0),
    folded: [...folded].sort((a, b) => a - b),
  };
}

/** `0.013` reads "1.3%", `0.97` "97%", something under a tenth of a per cent "<0.1%". */
export function formatShare(share: number): string {
  const percent = share * 100;

  if (percent < 0.1) {
    return '<0.1%';
  }

  if (percent < 10) {
    return `${percent.toFixed(1)}%`;
  }

  return `${Math.round(percent)}%`;
}
