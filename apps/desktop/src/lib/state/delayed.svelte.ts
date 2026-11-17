/** Loading indicators appear only after this long; faster loads show nothing, so they do not flicker. */
export const LOADING_DELAY_MS = 300;

/** A loading indicator that has appeared stays at least this long, so it never flashes for a moment. */
export const LOADING_MIN_VISIBLE_MS = 1000;

/**
 * Becomes true once `waiting()` has been true for `delayMs` without a break. Once true it stays true while `waiting()`
 * is, and for at least `minVisibleMs` in all: a wait that ends just after the indicator appeared keeps it on screen
 * for that long. A card that swaps its indicator for content waits for this value, not for the data alone.
 */
export function delayed(
  waiting: () => boolean,
  delayMs = LOADING_DELAY_MS,
  minVisibleMs = LOADING_MIN_VISIBLE_MS,
): { readonly value: boolean } {
  let value = $state(false);
  let shownAt = 0;

  $effect(() => {
    if (waiting()) {
      if (value) {
        return;
      }

      const timer = setTimeout(() => {
        value = true;
        shownAt = Date.now();
      }, delayMs);

      return () => clearTimeout(timer);
    }

    if (!value) {
      return;
    }

    const left = shownAt + minVisibleMs - Date.now();
    if (left <= 0) {
      value = false;
      return;
    }

    const timer = setTimeout(() => (value = false), left);

    return () => clearTimeout(timer);
  });

  return {
    get value() {
      return value;
    },
  };
}
