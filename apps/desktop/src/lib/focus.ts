/**
 * Tells the stylesheet how the user last interacted: `<html data-input="pointer">` after a mouse, pen or
 * touch press, `"keyboard"` after a key press.
 *
 * Focus rings follow it (app.css): they show only for the keyboard. `:focus-visible` alone is not enough,
 * because WebKitGTK (the Tauri window on Linux) also reports it for a clicked button, after the window
 * regains focus, and for focus that script moves (opening a menu, arrow keys in a list).
 */
export type InputModality = 'pointer' | 'keyboard';

const MODIFIER_KEYS = new Set(['Shift', 'Control', 'Alt', 'Meta', 'AltGraph', 'CapsLock']);

/** Starts tracking on the document and returns a function that stops it. */
export function trackInputModality(root: HTMLElement = document.documentElement): () => void {
  const set = (modality: InputModality): void => {
    root.dataset.input = modality;
  };

  const onPointer = (): void => set('pointer');

  const onKey = (event: KeyboardEvent): void => {
    // A modifier alone (or a shortcut such as Ctrl+C) is not navigation.
    if (!MODIFIER_KEYS.has(event.key) && !event.ctrlKey && !event.metaKey) {
      set('keyboard');
    }
  };

  const target = root.ownerDocument;

  // Capture: a component that stops the event must not hide it from here.
  target.addEventListener('pointerdown', onPointer, true);
  target.addEventListener('mousedown', onPointer, true);
  target.addEventListener('keydown', onKey, true);

  return () => {
    target.removeEventListener('pointerdown', onPointer, true);
    target.removeEventListener('mousedown', onPointer, true);
    target.removeEventListener('keydown', onKey, true);
  };
}
