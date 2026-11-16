/** Runs `callback` after the browser has painted the current state, so a CSS transition starts from it. Returns a cancel function. */
export function afterPaint(callback: () => void): () => void {
  let second = 0;
  const first = requestAnimationFrame(() => {
    second = requestAnimationFrame(callback);
  });

  return () => {
    cancelAnimationFrame(first);
    cancelAnimationFrame(second);
  };
}
