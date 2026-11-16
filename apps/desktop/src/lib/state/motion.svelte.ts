import { hooks } from '../hooks';
import { settings } from './settings.svelte';

/** True when the user asked for less motion: the OS preference, the setting, or the ?static test hook. */
class MotionState {
  #system = $state(false);

  constructor() {
    if (typeof window === 'undefined' || !window.matchMedia) {
      return;
    }

    const query = window.matchMedia('(prefers-reduced-motion: reduce)');

    this.#system = query.matches;
    query.addEventListener('change', (event) => (this.#system = event.matches));
  }

  get reduced(): boolean {
    return this.#system || settings.reduceMotion || hooks.static;
  }
}

export const motion = new MotionState();

/** Mirrors the state on <html>, where app.css picks it up. Call once from the root component. */
export function syncMotionAttribute(): void {
  $effect(() => {
    if (motion.reduced) {
      document.documentElement.dataset.motion = 'reduce';
    } else {
      delete document.documentElement.dataset.motion;
    }
  });
}
