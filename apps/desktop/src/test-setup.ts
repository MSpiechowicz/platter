import { cleanup } from '@testing-library/svelte';
import { afterEach } from 'vitest';

// Without vitest globals the library cannot register its own cleanup.
afterEach(cleanup);

// jsdom has no layout engine and no matchMedia; give components what they read.
if (!window.matchMedia) {
  window.matchMedia = (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}

// <dialog> is not implemented in jsdom.
if (typeof HTMLDialogElement !== 'undefined' && !HTMLDialogElement.prototype.showModal) {
  HTMLDialogElement.prototype.showModal = function showModal(this: HTMLDialogElement) {
    this.setAttribute('open', '');
  };
  HTMLDialogElement.prototype.close = function close(this: HTMLDialogElement) {
    this.removeAttribute('open');
    this.dispatchEvent(new Event('close'));
  };
}

// Svelte's transitions use the Web Animations API, which jsdom lacks. A stub that finishes at once is enough.
if (!Element.prototype.animate) {
  Element.prototype.animate = function animate() {
    return {
      cancel: () => {},
      finish: () => {},
      pause: () => {},
      play: () => {},
      reverse: () => {},
      currentTime: 0,
      set onfinish(callback: (() => void) | null) {
        if (callback) {
          queueMicrotask(callback);
        }
      },
    } as unknown as Animation;
  };
}
