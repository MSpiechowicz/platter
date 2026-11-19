import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { isThemeChoice, TOKEN_FALLBACK, ThemeStore } from './theme.svelte';

/** A `prefers-color-scheme: light` query that the test can flip, as the OS does. */
class FakeQuery {
  matches: boolean;
  readonly #listeners = new Set<(event: MediaQueryListEvent) => void>();

  constructor(matches: boolean) {
    this.matches = matches;
  }

  addEventListener(_type: string, listener: (event: MediaQueryListEvent) => void): void {
    this.#listeners.add(listener);
  }

  flip(matches: boolean): void {
    this.matches = matches;
    this.#listeners.forEach((listener) => listener({ matches } as MediaQueryListEvent));
  }
}

function setup(systemLight = false) {
  const root = document.createElement('html');
  const query = new FakeQuery(systemLight);
  const store = new ThemeStore(root, query as unknown as MediaQueryList);

  return { root, query, store };
}

describe('ThemeStore', () => {
  it('starts dark and writes it to the root element', () => {
    const { root, store } = setup();

    expect(store.choice).toBe('dark');
    expect(store.effective).toBe('dark');
    expect(root.dataset.theme).toBe('dark');
  });

  it('shows the picked theme whatever the OS says', () => {
    const { root, store } = setup(true);

    store.setChoice('dark');
    expect(store.effective).toBe('dark');
    expect(root.dataset.theme).toBe('dark');

    store.setChoice('light');
    expect(store.effective).toBe('light');
    expect(root.dataset.theme).toBe('light');
  });

  it('follows the OS while the pick is system', () => {
    const { root, query, store } = setup(true);

    store.setChoice('system');
    expect(store.effective).toBe('light');
    expect(root.dataset.theme).toBe('light');

    query.flip(false);
    expect(store.effective).toBe('dark');
    expect(root.dataset.theme).toBe('dark');

    query.flip(true);
    expect(root.dataset.theme).toBe('light');
  });

  it('ignores the OS while the pick is a fixed theme, and takes it up again on system', () => {
    const { root, query, store } = setup(false);

    store.setChoice('dark');
    query.flip(true);
    expect(root.dataset.theme).toBe('dark');

    store.setChoice('system');
    expect(root.dataset.theme).toBe('light');
  });

  it('tells a subscriber once for each change of the theme on screen, and nothing after it unsubscribes', () => {
    const { query, store } = setup(false);
    const heard: string[] = [];
    const stop = store.subscribe((theme) => heard.push(theme));

    store.setChoice('light');
    store.setChoice('light');
    store.setChoice('dark');
    // The OS is dark already, so system changes nothing.
    store.setChoice('system');
    expect(heard).toEqual(['light', 'dark']);

    query.flip(true);
    expect(heard).toEqual(['light', 'dark', 'light']);

    stop();
    store.setChoice('dark');
    expect(heard).toEqual(['light', 'dark', 'light']);
  });

  it('has the new theme in the page before a subscriber runs', () => {
    const { root, store } = setup();
    let seen = '';

    store.subscribe(() => (seen = root.dataset.theme ?? ''));
    store.setChoice('light');

    expect(seen).toBe('light');
  });

  it('reads a token from the page, and again after the theme changes', () => {
    const { root, store } = setup();

    document.documentElement.append(root);
    root.style.setProperty('--bg', '#010203');
    expect(store.token('--bg')).toBe('#010203');

    root.style.setProperty('--bg', '#040506');
    expect(store.token('--bg')).toBe('#010203');

    store.setChoice('light');
    expect(store.token('--bg')).toBe('#040506');
  });

  it('falls back to the theme own value for a page without styles', () => {
    const { store } = setup();

    expect(store.token('--bg')).toBe('#0a0e17');

    store.setChoice('light');
    expect(store.token('--bg')).toBe('#eef1f6');
    expect(store.token('--nothing')).toBe('');
  });

  it('works without matchMedia', () => {
    const root = document.createElement('html');
    const store = new ThemeStore(root, null);

    store.setChoice('system');

    expect(store.effective).toBe('dark');
    expect(root.dataset.theme).toBe('dark');
  });
});

describe('isThemeChoice', () => {
  it('accepts the three choices only', () => {
    expect(['dark', 'light', 'system'].every(isThemeChoice)).toBe(true);
    expect(isThemeChoice('sepia')).toBe(false);
    expect(isThemeChoice(undefined)).toBe(false);
  });
});

describe('the singleton', () => {
  beforeEach(() => {
    vi.resetModules();
  });

  afterEach(() => {
    document.documentElement.removeAttribute('data-theme');
  });

  it('sets data-theme on the document when applyThemeChoice is called', async () => {
    const { applyThemeChoice } = await import('./theme.svelte');

    applyThemeChoice('light');
    expect(document.documentElement.dataset.theme).toBe('light');

    applyThemeChoice('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });
});

describe('app.css tokens', () => {
  const css = readFileSync(resolve(process.cwd(), 'src/app.css'), 'utf8');
  const dark = css.slice(css.indexOf('\n:root {'), css.indexOf("\n:root[data-theme='light'] {"));
  const light = css.slice(css.indexOf("\n:root[data-theme='light'] {"), css.indexOf('*,\n*::before'));
  const names = (block: string): string[] =>
    [...block.matchAll(/^ {2}(--[\w-]+):/gm)].map((match) => match[1] as string);
  const value = (block: string, name: string): string | undefined =>
    new RegExp(`^ {2}${name}: ([^;]+);`, 'm').exec(block)?.[1]?.trim();

  it('gives every colour token of the dark set a light value, so no component has to know the theme', () => {
    const colourTokens = names(dark).filter((name) => value(dark, name)?.startsWith('#'));
    const missing = colourTokens.filter((name) => value(light, name) === undefined);

    expect(colourTokens.length).toBeGreaterThan(40);
    expect(missing).toEqual([]);
  });

  it('is used only with names that app.css defines or that a component sets for its own children', () => {
    const defined = new Set(names(dark));
    const local = new Set([
      '--c',
      '--tc',
      '--tx',
      '--i',
      '--ey',
      '--dy',
      '--dx',
      '--d',
      '--ay',
      '--advisor-col',
      '--panel-col',
      '--land-ms',
      '--land-ease',
      '--fade-ms',
      '--draw-ms',
    ]);
    const sources = import.meta.glob('../**/*.svelte', {
      query: '?raw',
      import: 'default',
      eager: true,
    }) as Record<string, string>;
    const used = new Set<string>();

    for (const source of Object.values(sources)) {
      for (const match of source.matchAll(/var\((--[\w-]+)/g)) {
        used.add(match[1] as string);
      }
    }

    expect([...used].filter((name) => !defined.has(name) && !local.has(name))).toEqual([]);
  });

  it('keeps the fallbacks the code reads in step with the style sheet', () => {
    for (const [theme, block] of [
      ['dark', dark],
      ['light', light],
    ] as const) {
      for (const [name, fallback] of Object.entries(TOKEN_FALLBACK[theme])) {
        expect(value(block, name)).toBe(fallback);
      }
    }
  });
});
