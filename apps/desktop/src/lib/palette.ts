/** Neutral colour of "everything else" (`--slate`): the Explorer's "N smaller items" and the Overview's "other". */
export const SLATE = '#66728a';

/**
 * One colour per folder, handed out biggest first. These are the accent hues of app.css
 * (`--cyan`, `--pink`, ...). No green: green means "safe" in this app, and nothing here is.
 */
export const PALETTE: readonly string[] = [
  '#3ec5d8',
  '#d6739f',
  '#d9a441',
  '#8f86d9',
  '#5b8fd6',
  '#c98a6b',
  '#b58bd0',
  '#cdbf94',
];

/**
 * The palette colour of every item, by size rank: the biggest gets the first colour and so on, then the palette
 * repeats. Equal sizes keep their listed order. The result is in the order of `items`.
 * The Explorer map and the Overview stack both colour their folders with this rule.
 */
export function rankColors<T>(items: readonly T[], sizeOf: (item: T) => number): string[] {
  const colors: string[] = [];

  items
    .map((item, index) => ({ index, size: sizeOf(item) }))
    .sort((a, b) => b.size - a.size || a.index - b.index)
    .forEach(({ index }, rank) => {
      colors[index] = PALETTE[rank % PALETTE.length] as string;
    });

  return colors;
}
