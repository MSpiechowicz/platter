import { joinPath, parentPath } from '../../paths';

/** The path of a child of a folder, from the folder's path (as `getPath` returns it) and the child's name. */
export const childPath = joinPath;

export { parentPath };

/** The full path of every breadcrumb of a `getPath` answer, root first: the last is `path` itself. */
export function crumbPaths(path: string, depth: number): string[] {
  const paths = [path];

  while (paths.length < depth) {
    paths.unshift(parentPath(paths[0] as string));
  }

  return paths;
}
