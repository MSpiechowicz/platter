/** The path of a child of a folder, from the folder's path (as `getPath` returns it) and the child's name. */
export function childPath(parent: string, name: string): string {
  const separator = parent.includes('\\') && !parent.includes('/') ? '\\' : '/';

  return `${parent.replace(/[\\/]+$/, '')}${separator}${name}`;
}

/** The folder that contains `path`: "/home/demo" gives "/home", "/home" gives "/". */
export function parentPath(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  const cut = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));

  if (cut < 0) {
    return path;
  }

  const parent = trimmed.slice(0, cut);

  // A root keeps its separator: "/" and "C:\".
  return parent === '' || /^[A-Za-z]:$/.test(parent) ? `${parent}${trimmed[cut]}` : parent;
}

/** The full path of every breadcrumb of a `getPath` answer, root first: the last is `path` itself. */
export function crumbPaths(path: string, depth: number): string[] {
  const paths = [path];

  while (paths.length < depth) {
    paths.unshift(parentPath(paths[0] as string));
  }

  return paths;
}
