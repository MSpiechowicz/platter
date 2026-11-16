/** Loading indicators appear only after this long; faster loads show nothing, so they do not flicker. */
export const LOADING_DELAY_MS = 300;

/** Becomes true once `waiting()` has been true for `delayMs` without a break, and false again as soon as it is not. */
export function delayed(waiting: () => boolean, delayMs = LOADING_DELAY_MS): { readonly value: boolean } {
  let value = $state(false);

  $effect(() => {
    if (!waiting()) {
      value = false;
      return;
    }

    const timer = setTimeout(() => (value = true), delayMs);

    return () => clearTimeout(timer);
  });

  return {
    get value() {
      return value;
    },
  };
}
