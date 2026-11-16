/** With this many folders or fewer the list is a short one and says so, instead of leaving the card blank. */
const FEW_FOLDERS = 2;

/** The one-line note under a short list of largest folders; nothing when the list is long enough to stand alone. */
export function fewFoldersNote(count: number): string | undefined {
  if (count > FEW_FOLDERS) {
    return undefined;
  }

  if (count === 0) {
    return 'No folders here';
  }

  return `Only ${count} ${count === 1 ? 'folder' : 'folders'} here`;
}
