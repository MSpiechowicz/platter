/** The height the banners take from the page, margins included, as a custom property of the main area. */
export const BANNER_SPACE = '--banner-space';

/** The banners on screen in each main area, and how much room each takes. */
const shown = new WeakMap<HTMLElement, Map<HTMLElement, number>>();

/**
 * Pages that fill the window (the Advisor, the Explorer) take the banners' room off their height, so a banner does
 * not make them scroll. Each banner adds its own room, so two banners at once are both counted, and the room follows
 * a banner as it wraps in a narrow window. Returns the function that takes the banner's room away again.
 */
export function publishBannerSpace(banner: HTMLElement): () => void {
  const main = banner.closest('main');

  if (!main) {
    return () => {};
  }

  const rooms = shown.get(main) ?? new Map<HTMLElement, number>();

  shown.set(main, rooms);

  const write = (): void => {
    const total = [...rooms.values()].reduce((sum, room) => sum + room, 0);

    if (rooms.size === 0) {
      main.style.removeProperty(BANNER_SPACE);
    } else {
      main.style.setProperty(BANNER_SPACE, `${total}px`);
    }
  };

  const measure = (): void => {
    const margin = parseFloat(getComputedStyle(banner).marginBottom) || 0;

    rooms.set(banner, banner.offsetHeight + margin);
    write();
  };

  measure();

  const observer = typeof ResizeObserver === 'undefined' ? undefined : new ResizeObserver(measure);

  observer?.observe(banner);

  return () => {
    observer?.disconnect();
    rooms.delete(banner);
    write();
  };
}
