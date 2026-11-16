import { untrack } from 'svelte';
import { motion } from '../state/motion.svelte';

/**
 * A number that eases towards `target` whenever it changes. With reduced motion it jumps at once.
 * Call it while a component initialises.
 */
export function countUp(target: () => number, durationMs = 1100): { readonly value: number } {
  let value = $state(motion.reduced ? untrack(target) : 0);

  $effect(() => {
    const to = target();

    if (motion.reduced) {
      value = to;
      return;
    }

    const from = untrack(() => value);
    const start = performance.now();
    let frame = 0;

    const step = (now: number): void => {
      // A frame timestamp can be slightly older than `start`.
      const progress = Math.max(0, Math.min(1, (now - start) / durationMs));

      value = from + (to - from) * (1 - Math.pow(1 - progress, 3));

      if (progress < 1) {
        frame = requestAnimationFrame(step);
      }
    };

    frame = requestAnimationFrame(step);

    return () => cancelAnimationFrame(frame);
  });

  return {
    get value() {
      return value;
    },
  };
}
