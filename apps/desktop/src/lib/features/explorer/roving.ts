/**
 * Arrow-key movement in a group where only one item is in the tab order ("roving tabindex"):
 * Tab enters and leaves the group, the arrow keys move inside it.
 */
const NEXT_VERTICAL = ['ArrowDown'];
const PREVIOUS_VERTICAL = ['ArrowUp'];
const NEXT_BOTH = ['ArrowDown', 'ArrowRight'];
const PREVIOUS_BOTH = ['ArrowUp', 'ArrowLeft'];

/** The item the key moves to from `current`, or `undefined` when the key does not move or `current` is not in the group. */
export function neighbour<T extends Element>(
  key: string,
  items: readonly T[],
  current: Element | null,
  axes: 'vertical' | 'both' = 'vertical',
): T | undefined {
  const at = items.findIndex((item) => item === current);

  if (at < 0) {
    return undefined;
  }

  if ((axes === 'both' ? NEXT_BOTH : NEXT_VERTICAL).includes(key)) {
    return items[Math.min(items.length - 1, at + 1)];
  }

  if ((axes === 'both' ? PREVIOUS_BOTH : PREVIOUS_VERTICAL).includes(key)) {
    return items[Math.max(0, at - 1)];
  }

  if (key === 'Home') {
    return items[0];
  }

  return key === 'End' ? items[items.length - 1] : undefined;
}

/** Moves the focus for an arrow key press and reports whether it did. */
export function moveFocus(
  event: KeyboardEvent,
  items: readonly (HTMLElement | SVGElement)[],
  axes: 'vertical' | 'both' = 'vertical',
): boolean {
  const target = neighbour(event.key, items, document.activeElement, axes);

  if (!target) {
    return false;
  }

  event.preventDefault();
  target.focus();

  return true;
}
