/** The width at which a page is a single column, and nothing is given a height of its own. */
const SINGLE_COLUMN = '(max-width: 1000px)';

/** The bottom padding of the page, when the page has none to read. */
const PAGE_PADDING = 20;

/** The distance from the top of the page, whatever the page is scrolled to and whatever the element is animated by. */
function documentTop(element: HTMLElement): number {
  let top = 0;

  for (
    let current: Element | null = element;
    current instanceof HTMLElement;
    current = current.offsetParent
  ) {
    top += current.offsetTop;
  }

  return top;
}

/** The elements that come before `element` in its parent. */
function before(element: Element): Element[] {
  const siblings = [...(element.parentElement?.children ?? [])];

  return siblings.slice(0, siblings.indexOf(element));
}

/** `start` and the elements above it, up to but not including `stop`; just `start` when `stop` is not above it. */
function containersUpTo(start: Element | null, stop: Element | null): Element[] {
  const found: Element[] = [];

  for (let node = start; node && node !== stop; node = node.parentElement) {
    found.push(node);

    if (!stop) {
      break;
    }
  }

  return found;
}

/**
 * The height an element may have to reach the bottom of the window, down to the bottom padding of the page: with it
 * the page has no scroll bar of its own, and what is inside the element scrolls. `cap` is that height, never below
 * `minHeight` (a window too short for it scrolls the page instead), and `undefined` in a single column, where nothing
 * is capped.
 *
 * Call it while a component is set up. The element is a getter, so it may appear later (`bind:this`). Anything that
 * scrolls inside the element and holds visually hidden absolute elements needs `position: relative`, or they stretch
 * the page.
 */
export function fillHeight(
  target: () => HTMLElement | undefined,
  minHeight: number,
): { readonly cap: number | undefined } {
  let cap = $state<number>();

  function measure(): void {
    const element = target();

    if (!element) {
      return;
    }

    if (window.matchMedia(SINGLE_COLUMN).matches) {
      cap = undefined;

      return;
    }

    const padding = parseFloat(getComputedStyle(element.closest('main') ?? document.body).paddingBottom);
    const room = window.innerHeight - (Number.isNaN(padding) ? PAGE_PADDING : padding) - documentTop(element);

    cap = Math.max(minHeight, Math.floor(room));
  }

  // The top of the element moves with whatever is above it, and the element gives way: that keeps the page as tall as
  // the window, so only what is above is watched.
  $effect(() => {
    const element = target();

    if (!element) {
      return;
    }

    measure();
    window.addEventListener('resize', measure);

    if (typeof ResizeObserver === 'undefined') {
      return () => window.removeEventListener('resize', measure);
    }

    const grid = element.parentElement;
    const observer = new ResizeObserver(measure);

    // The containers from the parent up to the page area: what comes before any of them (the page header, the banner
    // of an outdated scan) pushes the element down just the same.
    const containers = containersUpTo(grid, element.closest('main'));

    // The elements above, and the parent itself: it is as tall as they are.
    const watch = (): void => {
      observer.disconnect();

      if (grid) {
        observer.observe(grid);
      }

      for (const above of [...before(element), ...containers.flatMap(before)]) {
        observer.observe(above);
      }
    };

    // What is above can be replaced (the Advisor for the result) or appear (the banner): the new elements are watched
    // too.
    const changes = new MutationObserver(() => {
      watch();
      measure();
    });

    watch();

    // Observing the same node twice is harmless: the observer takes it once.
    for (const container of [grid, ...containers.map((node) => node.parentElement)]) {
      if (container) {
        changes.observe(container, { childList: true });
      }
    }

    return () => {
      window.removeEventListener('resize', measure);
      observer.disconnect();
      changes.disconnect();
    };
  });

  return {
    get cap() {
      return cap;
    },
  };
}
