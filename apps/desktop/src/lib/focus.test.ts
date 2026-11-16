import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { trackInputModality } from './focus';

describe('trackInputModality', () => {
  let stop: () => void;

  beforeEach(() => {
    delete document.documentElement.dataset.input;
    stop = trackInputModality();
  });

  afterEach(() => stop());

  const press = (key: string, init: KeyboardEventInit = {}): void => {
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...init }));
  };

  it('says "pointer" after a mouse press, so a clicked control shows no focus ring', () => {
    document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));

    expect(document.documentElement.dataset.input).toBe('pointer');
  });

  it('says "keyboard" after Tab or an arrow key, and "pointer" again after the next click', () => {
    press('Tab');
    expect(document.documentElement.dataset.input).toBe('keyboard');

    document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    expect(document.documentElement.dataset.input).toBe('pointer');

    press('ArrowDown');
    expect(document.documentElement.dataset.input).toBe('keyboard');
  });

  it('ignores a modifier on its own and shortcuts', () => {
    document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));

    press('Shift');
    press('c', { ctrlKey: true });

    expect(document.documentElement.dataset.input).toBe('pointer');
  });

  it('stops tracking when asked to', () => {
    stop();
    press('Tab');

    expect(document.documentElement.dataset.input).toBeUndefined();
  });
});
