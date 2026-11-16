export interface TooltipContent {
  /** Bold first line. */
  title?: string;
  /** Plain text when there is no title or list. */
  text?: string;
  /** Bullet lines. */
  items?: string[];
  /** Small print under a divider. */
  note?: string;
}

class TooltipState {
  target = $state<HTMLElement | null>(null);
  content = $state<TooltipContent>({});

  show(target: HTMLElement, content: TooltipContent): void {
    this.content = content;
    this.target = target;
  }

  hide(target: HTMLElement): void {
    if (this.target === target) {
      this.target = null;
    }
  }
}

export const tip = new TooltipState();

function normalise(content: string | TooltipContent): TooltipContent {
  return typeof content === 'string' ? { text: content } : content;
}

/**
 * `use:tooltip={...}` shows a tooltip on hover and keyboard focus.
 * The text is drawn by <TooltipHost> on the page root, so no card can clip it.
 */
export function tooltip(node: HTMLElement, initial: string | TooltipContent) {
  let content = normalise(initial);

  const show = (): void => tip.show(node, content);
  const hide = (): void => tip.hide(node);

  node.addEventListener('mouseenter', show);
  node.addEventListener('mouseleave', hide);
  node.addEventListener('focus', show);
  node.addEventListener('blur', hide);

  return {
    update(next: string | TooltipContent): void {
      content = normalise(next);

      if (tip.target === node) {
        tip.content = content;
      }
    },
    destroy(): void {
      hide();
      node.removeEventListener('mouseenter', show);
      node.removeEventListener('mouseleave', hide);
      node.removeEventListener('focus', show);
      node.removeEventListener('blur', hide);
    },
  };
}
