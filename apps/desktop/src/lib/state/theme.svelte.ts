/** What the user picks in Settings: a fixed theme, or whatever the OS is set to. */
export type ThemeChoice = 'dark' | 'light' | 'system';

/** The theme that is on screen. */
export type EffectiveTheme = 'dark' | 'light';

export const THEME_CHOICES: readonly ThemeChoice[] = ['dark', 'light', 'system'];

export function isThemeChoice(value: unknown): value is ThemeChoice {
  return THEME_CHOICES.includes(value as ThemeChoice);
}

const LIGHT_QUERY = '(prefers-color-scheme: light)';

/**
 * The tokens that JS reads back from app.css, as `#rrggbb`, for a page that has no style sheet (the unit tests).
 * They are the values of app.css, and a test keeps them so.
 */
export const TOKEN_FALLBACK: Record<EffectiveTheme, Readonly<Record<string, string>>> = {
  dark: { '--bg': '#0a0e17', '--card': '#111723' },
  light: { '--bg': '#eef1f6', '--card': '#ffffff' },
};

/**
 * The theme of the window. The pick is `dark`, `light` or `system`, and `system` follows the OS while the app runs.
 * The theme on screen is `effective`, mirrored on `<html data-theme>` where app.css swaps its tokens.
 *
 * Plain CSS needs nothing more. Code that draws with colours of its own (the Explorer map, the 3D view) reads tokens
 * with `token()` in a reactive context, or calls `subscribe()` to redraw when the theme changes.
 * The store keeps no setting: the owner of the setting calls `setChoice` (see `applyThemeChoice`).
 */
export class ThemeStore {
  readonly #root: HTMLElement | null;
  // Plain collections: they are bookkeeping, and `token()` fills the cache inside effects, where a reactive map may not be written.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  readonly #listeners = new Set<(theme: EffectiveTheme) => void>();
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  readonly #tokens = new Map<string, string>();
  #choice = $state<ThemeChoice>('dark');
  #systemLight = $state(false);
  /** What the page shows now: it moves only through `#apply`, so a listener hears about every change once. */
  #shown: EffectiveTheme = 'dark';

  constructor(
    root: HTMLElement | null = typeof document === 'undefined' ? null : document.documentElement,
    query: MediaQueryList | null = typeof window === 'undefined' || !window.matchMedia
      ? null
      : window.matchMedia(LIGHT_QUERY),
  ) {
    this.#root = root;

    if (query) {
      this.#systemLight = query.matches;
      query.addEventListener('change', (event) => {
        this.#systemLight = event.matches;
        this.#apply();
      });
    }

    this.#apply();
  }

  get choice(): ThemeChoice {
    return this.#choice;
  }

  get effective(): EffectiveTheme {
    if (this.#choice === 'system') {
      return this.#systemLight ? 'light' : 'dark';
    }

    return this.#choice;
  }

  setChoice(choice: ThemeChoice): void {
    this.#choice = choice;
    this.#apply();
  }

  /**
   * Calls `listener` with the new theme each time the theme on screen changes (not when a pick changes nothing, as
   * `system` does while the OS is already dark). The page has the new tokens by then. Returns what to call to stop.
   */
  subscribe(listener: (theme: EffectiveTheme) => void): () => void {
    this.#listeners.add(listener);

    return () => this.#listeners.delete(listener);
  }

  /**
   * The value of a CSS token (`--bg`) in the theme on screen, trimmed. Reading it inside an effect or a derived value
   * makes that run again when the theme changes. The page's own value wins, the fallback is for a page without styles.
   */
  token(name: string): string {
    const theme = this.effective;
    const known = this.#tokens.get(name);

    if (known !== undefined) {
      return known;
    }

    const page = this.#root ? getComputedStyle(this.#root).getPropertyValue(name).trim() : '';
    const value = page || (TOKEN_FALLBACK[theme][name] ?? '');

    this.#tokens.set(name, value);

    return value;
  }

  #apply(): void {
    const theme = this.effective;

    // Written before any effect runs, so code that reads a token in response sees the new theme's value.
    this.#root?.setAttribute('data-theme', theme);
    this.#tokens.clear();

    if (theme === this.#shown) {
      return;
    }

    this.#shown = theme;
    this.#listeners.forEach((listener) => listener(theme));
  }
}

export const theme = new ThemeStore();

/** Applies the user's pick. The owner of the setting calls it at start and whenever the setting changes. */
export function applyThemeChoice(choice: ThemeChoice): void {
  theme.setChoice(choice);
}
