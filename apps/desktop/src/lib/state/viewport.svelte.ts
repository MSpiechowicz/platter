/** The window is too narrow for the full sidebar: it shows as the icon rail. */
const NARROW_QUERY = '(max-width: 900px)';

class ViewportState {
  narrow = $state(false);

  constructor() {
    if (typeof window === 'undefined' || !window.matchMedia) {
      return;
    }

    const query = window.matchMedia(NARROW_QUERY);

    this.narrow = query.matches;
    query.addEventListener('change', (event) => (this.narrow = event.matches));
  }
}

export const viewport = new ViewportState();
