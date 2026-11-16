/*
 * Backend strings (paths, reasons, error messages, tool output) are shown as plain text, and only after these
 * helpers: a name can hold characters that hide or reorder what the user reads.
 */

/** Control characters, invisible formatting (bidi marks and overrides, zero-width), line and paragraph separators. */
const HIDDEN = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/gu;

/** Makes every hidden character visible as `\u{202E}`. */
export function visibleText(text: string): string {
  return text.replace(HIDDEN, (character) => {
    const code = character.codePointAt(0) as number;

    return `\\u{${code.toString(16).toUpperCase()}}`;
  });
}

/**
 * Shortens a long path in the middle ("/home/demo/…/deep/folder"), counting characters (code points), not code
 * units. The cut is made before the escaping, so an escape is never cut in half.
 */
export function shortenMiddle(text: string, max: number): string {
  const characters = Array.from(text);

  if (characters.length <= max || max < 3) {
    return text;
  }

  const keep = max - 1;
  const head = Math.ceil(keep / 2);
  const tail = keep - head;

  return `${characters.slice(0, head).join('')}…${characters.slice(characters.length - tail).join('')}`;
}

/** What a path looks like in the interface: cut in the middle when long, with hidden characters made visible. */
export function shownPath(path: string, max = 64): string {
  return visibleText(shortenMiddle(path, max));
}
