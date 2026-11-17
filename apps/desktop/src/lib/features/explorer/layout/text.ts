/**
 * Fitting text into a tile. Widths are measured, not guessed: a name is cut only when it really does not fit, and it is
 * cut at its end, never in the middle (`Sid Meier's Ci…`, never `Sid…ilization`). A size is written whole or not at all.
 */

/** The fonts the map writes in: bold mono names, UI-font sizes, and the (smaller) names on 3D labels. */
export type TextFont = 'name' | 'size' | 'label';

/** The width in pixels of `text` written in `font`. */
export type Measurer = (text: string, font: TextFont) => number;

export const ELLIPSIS = '…';
/** What a chain is written as when all of it fits: `Game / Content / Paks`. */
export const CHAIN_SEPARATOR = ' / ';
/** A name that has to be cut keeps at least this many characters (and the ellipsis), or it is not written at all. */
export const MIN_NAME_CHARS = 4;

/** Rough widths per character, used where nothing can be measured: the same for every character. */
const APPROX_PX: Readonly<Record<TextFont, number>> = { name: 7.9, size: 6.6, label: 7.3 };

/** A measurer that counts characters. Deterministic, so it is what tests and layout without a DOM use. */
export const approximateMeasure: Measurer = (text, font) => text.length * APPROX_PX[font];

const CSS: Readonly<Record<TextFont, { weight: number; px: number; variable: '--mono' | '--font' }>> = {
  name: { weight: 600, px: 13, variable: '--mono' },
  size: { weight: 400, px: 12, variable: '--font' },
  label: { weight: 600, px: 12, variable: '--mono' },
};
const FALLBACK_FAMILY: Readonly<Record<'--mono' | '--font', string>> = {
  '--mono': "'JetBrains Mono', 'Fira Code', ui-monospace, monospace",
  '--font': "'Inter Variable', 'Inter', 'Segoe UI', system-ui, sans-serif",
};
const CACHE_LIMIT = 20_000;

/** The canvas that measures, or `null` where there is none (a DOM without a layout engine, such as jsdom). */
function measuringContext(): CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null {
  if (typeof document === 'undefined' || navigator.userAgent.includes('jsdom')) {
    return null;
  }

  try {
    const canvas =
      typeof OffscreenCanvas === 'undefined' ? document.createElement('canvas') : new OffscreenCanvas(1, 1);

    return canvas.getContext('2d') as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
  } catch {
    return null;
  }
}

/** Measures with the browser's own text engine, once per string and font (the answers are kept). */
export function createCanvasMeasure(): Measurer {
  const context = measuringContext();
  const cache = new Map<string, number>();
  const fonts = new Map<TextFont, string>();

  if (!context) {
    return approximateMeasure;
  }

  /** The font as the page writes it: the families come from the page's own variables. */
  function fontOf(font: TextFont): string {
    const known = fonts.get(font);

    if (known) {
      return known;
    }

    const { weight, px, variable } = CSS[font];
    const family = getComputedStyle(document.documentElement).getPropertyValue(variable).trim();
    const css = `${weight} ${px}px ${family || FALLBACK_FAMILY[variable]}`;

    fonts.set(font, css);

    return css;
  }

  return (text, font) => {
    const key = `${font}\u0000${text}`;
    const known = cache.get(key);

    if (known !== undefined) {
      return known;
    }

    if (cache.size >= CACHE_LIMIT) {
      cache.clear();
    }

    context.font = fontOf(font);

    const width = context.measureText(text).width;

    cache.set(key, width);

    return width;
  };
}

let current: Measurer | undefined;

/** What the map measures with: the canvas, made on first use. */
export const measure: Measurer = (text, font) => (current ??= createCanvasMeasure())(text, font);

/** Tests (and anything that cannot measure) replace the measurer; `undefined` goes back to the canvas. */
export function setMeasurer(next: Measurer | undefined): void {
  current = next;
}

/** Whether `text` fits in `pixels` as it is. */
export function fits(text: string, pixels: number, font: TextFont, measurer: Measurer = measure): boolean {
  return measurer(text, font) <= pixels;
}

/**
 * `text` as it is when it fits `pixels`; else its start with an ellipsis, when at least `MIN_NAME_CHARS` characters
 * and the ellipsis fit; else `''`.
 */
export function endTruncate(
  text: string,
  pixels: number,
  font: TextFont = 'name',
  measurer: Measurer = measure,
): string {
  if (fits(text, pixels, font, measurer)) {
    return text;
  }

  for (let length = text.length - 1; length >= MIN_NAME_CHARS; length--) {
    // The space before the cut is not written, and does not count as a character of the name.
    const head = text.slice(0, length).trimEnd();

    if (head.length >= MIN_NAME_CHARS && fits(`${head}${ELLIPSIS}`, pixels, font, measurer)) {
      return `${head}${ELLIPSIS}`;
    }
  }

  return '';
}

/** The names of a chain written as one text: `home / sam`. */
export const joinNames = (names: readonly string[]): string => names.join(CHAIN_SEPARATOR);

/**
 * The texts a name may be written as, whole, the most complete first: the whole chain, then its first folder (the one
 * that says what it is). A plain name is only itself.
 */
export function wholeTitles(names: readonly string[]): string[] {
  const full = joinNames(names);
  const first = names[0] ?? '';

  return full === first ? [full] : [full, first];
}

/**
 * The name of a tile cut to `pixels`. A chain is written whole when all of it fits, else by its first folder alone,
 * and when that does not fit either, by the start of the first folder with an ellipsis. Never the middle, never the tail.
 * `''` when not even four characters fit.
 */
export function fitTitle(
  names: readonly string[],
  pixels: number,
  font: TextFont = 'name',
  measurer: Measurer = measure,
): string {
  const whole = wholeTitles(names).find((title) => fits(title, pixels, font, measurer));

  return whole ?? endTruncate(names[0] ?? '', pixels, font, measurer);
}
