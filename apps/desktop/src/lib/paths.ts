/*
 * Path helpers that know both path styles: `/home/demo` and `C:\Users\demo`. The style is read from the path itself,
 * so a helper needs no operating system. A path with a drive letter is a Windows path, and everything else is read
 * as a POSIX one. Windows paths compare with ASCII letters folded and with `/` read as `\`, the same rule as the
 * backend (`docs/architecture.md`, "Windows (P1)").
 */

/** `C:` alone or followed by a separator: the start of every absolute Windows path. */
const DRIVE = /^[A-Za-z]:(?=[\\/]|$)/;

const ANY_SEPARATORS_AT_END = /[\\/]+$/;

/** Whether the path starts with a drive letter (`C:\Users`, `d:/x`, `C:`). */
export function isWindowsPath(path: string): boolean {
  return DRIVE.test(path);
}

/** The separator of the path's own style. */
export function separatorOf(path: string): '/' | '\\' {
  return isWindowsPath(path) ? '\\' : '/';
}

/** Whether the path starts at a root: `/...` or `C:\...` (a drive alone, `C:`, is not one). */
export function isAbsolutePath(path: string): boolean {
  return path.startsWith('/') || /^[A-Za-z]:[\\/]/.test(path);
}

/** The root of the path as the backend writes it: `/`, or `C:\` with the drive letter as given. */
export function rootOf(path: string): string {
  return isWindowsPath(path) ? `${path.slice(0, 2)}\\` : '/';
}

/** Whether the path is a root, `/` or `C:\`. */
export function isRootPath(path: string): boolean {
  const stripped = path.replace(ANY_SEPARATORS_AT_END, '');

  return stripped === '' || (isWindowsPath(path) && stripped.length <= 2);
}

/** The path without a separator at its end, except that a root keeps its own: `C:\Users\` is `C:\Users`, `C:\` stays. */
function withoutEnd(path: string): string {
  return isRootPath(path) ? rootOf(path) : path.replace(ANY_SEPARATORS_AT_END, '');
}

/** The last name of the path: `C:\Users\me\Downloads` gives `Downloads`. A root gives itself (`/`, `C:\`). */
export function baseName(path: string): string {
  const trimmed = withoutEnd(path);

  if (isRootPath(trimmed)) {
    return trimmed;
  }

  const cut = isWindowsPath(trimmed)
    ? Math.max(trimmed.lastIndexOf('\\'), trimmed.lastIndexOf('/'))
    : trimmed.lastIndexOf('/');

  return trimmed.slice(cut + 1);
}

/** The folder that contains `path`: `C:\Users\me` gives `C:\Users`, `C:\Users` gives `C:\`, and a root gives itself. */
export function parentPath(path: string): string {
  const trimmed = withoutEnd(path);

  if (isRootPath(trimmed)) {
    return trimmed;
  }

  const separators = isWindowsPath(trimmed) ? ['\\', '/'] : ['/'];
  const cut = Math.max(...separators.map((separator) => trimmed.lastIndexOf(separator)));

  if (cut < 0) {
    return path;
  }

  const parent = trimmed.slice(0, cut);

  // A root keeps its separator: "/" and "C:\".
  return parent === '' || isRootPath(parent) ? rootOf(parent === '' ? '/' : parent) : parent;
}

/** The path of a child from its folder's path and its name, with the folder's own separator. */
export function joinPath(parent: string, name: string): string {
  return `${parent.replace(ANY_SEPARATORS_AT_END, '')}${separatorOf(parent)}${name}`;
}

/** A path as it is compared: ASCII letters folded and `/` read as `\` on Windows, no separator at the end. */
function comparable(path: string, windows: boolean): string {
  if (!windows) {
    return withoutEnd(path);
  }

  return withoutEnd(path.replace(/\//g, '\\')).replace(/[A-Z]/g, (letter) => letter.toLowerCase());
}

/** Whether two paths name the same place: ignoring case and the kind of slash on Windows, and a separator at the end. */
export function isSamePath(a: string, b: string): boolean {
  const windows = isWindowsPath(a) || isWindowsPath(b);

  return comparable(a, windows) === comparable(b, windows);
}

/** Whether `path` is `ancestor` or lies below it. The comparison is by whole names: `/a/bc` is not under `/a/b`. */
export function isUnderPath(path: string, ancestor: string): boolean {
  const windows = isWindowsPath(path) || isWindowsPath(ancestor);
  const inner = comparable(path, windows);
  const outer = comparable(ancestor, windows);

  if (inner === outer) {
    return true;
  }

  const separator = windows ? '\\' : '/';

  return inner.startsWith(outer.endsWith(separator) ? outer : `${outer}${separator}`);
}
