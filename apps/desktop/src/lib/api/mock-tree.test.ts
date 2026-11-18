import { describe, expect, it } from 'vitest';
import { CATEGORIES } from './mock-categories';
import { buildTree, childrenOf, isDirectory, type MockNode, type MockTree } from './mock-tree';
import type { Category } from './types';

const NOW = 1_750_000_000;

function treeOf(root: string, extra: { os?: 'windows' | 'macos' } = {}): MockTree {
  return buildTree({ seed: 20240607, root, oneFileSystem: true, bytes: 186_000_000_000, now: NOW, ...extra });
}

/** The node at a path below the scan root, by names. */
function at(tree: MockTree, path: string): MockNode {
  let node = tree.nodes[0] as MockNode;

  for (const name of path.split('/').filter((part) => part !== '')) {
    const next = childrenOf(tree, node).find((child) => child.name === name);

    expect(next, `${name} of ${path}`).toBeDefined();
    node = next as MockNode;
  }

  return node;
}

/** Every node below `node`, depth first. */
function below(tree: MockTree, node: MockNode): MockNode[] {
  return childrenOf(tree, node).flatMap((child) => [child, ...below(tree, child)]);
}

const isFile = (node: MockNode): boolean => node.kind === 'file';

/** The folders of a tree with the names of the folders from the root to them. */
function folders(tree: MockTree): { node: MockNode; path: string[] }[] {
  const found: { node: MockNode; path: string[] }[] = [];

  const visit = (node: MockNode, path: string[]): void => {
    found.push({ node, path });
    childrenOf(tree, node)
      .filter((child) => child.kind === 'dir')
      .forEach((child) => visit(child, [...path, child.name]));
  };

  visit(tree.nodes[0] as MockNode, []);

  return found;
}

describe('the categories of the demo tree', () => {
  const tree = treeOf('/');

  it('gives a file the category of its name when no folder above it imposes one, so every name maps back to its category', () => {
    const outside = (path: string): MockNode[] => below(tree, at(tree, path)).filter(isFile);

    for (const [path, category] of [
      ['home/demo/Videos', 'video'],
      ['home/demo/Pictures', 'image'],
      ['home/demo/Music', 'audio'],
      ['opt', 'app'],
      ['usr/lib', 'app'],
      ['var/lib/docker', 'diskImage'],
      ['var/lib/systemd/coredump', 'other'],
      ['var/cache/pacman/pkg', 'package'],
      ['.snapshots/recent', 'archive'],
    ] as const) {
      const files = outside(path);

      expect(files.length, path).toBeGreaterThan(0);
      expect(
        files.every((file) => file.category === category),
        path,
      ).toBe(true);
    }
  });

  it('imposes the category of a well-known folder on everything below it, the outermost folder winning', () => {
    // `.cache` holds files that are named as images or archives, and Steam holds a folder called `shadercache`.
    for (const path of ['home/demo/.cache', 'home/demo/.npm', 'home/demo/.cargo/registry']) {
      const node = at(tree, path);

      expect(node.category, path).toBe('package');
      expect(
        below(tree, node)
          .filter((entry) => entry.kind !== 'dir')
          .every((entry) => entry.category === 'package'),
        path,
      ).toBe(true);
    }

    const steam = at(tree, 'home/demo/.local/share/Steam');
    const cache = at(tree, 'home/demo/.local/share/Steam/steamapps/shadercache');

    expect(steam.category).toBe('app');
    expect(below(tree, steam).every((entry) => entry.kind === 'dir' || entry.category === 'app')).toBe(true);
    expect(cache.category).toBe('app');
  });

  it('gives a folder the category that holds most of its bytes, and `other` when it holds none', () => {
    for (const { node } of folders(tree)) {
      const bytes = new Map<Category, number>();

      for (const entry of below(tree, node).filter((item) => !isDirectory(item))) {
        bytes.set(entry.category, (bytes.get(entry.category) ?? 0) + entry.allocated);
      }

      const heaviest = [...bytes].reduce<[Category, number] | null>((best, entry) => {
        const ahead =
          best === null ||
          entry[1] > best[1] ||
          (entry[1] === best[1] && CATEGORIES.indexOf(entry[0]) < CATEGORIES.indexOf(best[0]));

        return ahead ? entry : best;
      }, null);

      if (heaviest && heaviest[1] > 0) {
        expect(node.category, node.name).toBe(heaviest[0]);
      }
    }

    expect(at(tree, 'srv').category).not.toBe('other');
    expect(at(tree, 'media').category).toBe('other');
    expect(at(tree, 'root').category).toBe('other');
  });

  it('keeps the category a name imposes on a folder that holds no bytes', () => {
    // The names of hard links that the scan did not charge: no bytes below, but still a store.
    const store = folders(tree).find(({ node }) => node.name === '.pnpm-store');

    expect(store).toBeDefined();
    expect(
      below(tree, store?.node as MockNode).every((entry) => entry.allocated === 0 || entry.kind === 'dir'),
    ).toBe(true);
    expect(store?.node.category).toBe('package');
  });

  it('takes the category from the name of the scan root too', () => {
    const cache = treeOf('/home/demo/.cache');

    expect(cache.nodes[0]?.category).toBe('package');
    expect(
      cache.nodes.filter((node) => node.kind === 'file').every((node) => node.category === 'package'),
    ).toBe(true);
  });

  it('calls a folder next to a project marker build output', () => {
    const projects = folders(tree).filter(({ node }) =>
      childrenOf(tree, node).some((child) => child.name === 'Cargo.toml'),
    );

    expect(projects.length).toBeGreaterThan(0);

    for (const { node } of projects) {
      const target = childrenOf(tree, node).find((child) => child.name === 'target');

      expect(target?.category, node.name).toBe('build');
    }

    // `src` has no rule, so it has what is in it: code.
    expect(at(tree, 'home/demo/Projekty/platter/src').category).toBe('code');
  });

  it('sorts the marker files by their names', () => {
    const project = at(tree, 'home/demo/Projekty/platter');
    const marker = (name: string): MockNode | undefined =>
      childrenOf(tree, project).find((child) => child.name === name);

    expect(marker('Cargo.toml')?.category).toBe('code');
    expect(marker('package.json')?.category).toBe('code');
    expect(marker('pnpm-lock.yaml')?.category).toBe('code');
  });
});

describe('hard links in the demo tree', () => {
  const tree = treeOf('/');
  const stores = folders(tree).filter(({ node }) => node.name === '.pnpm-store');

  it('puts the other names of a shared file in a store that has no bytes', () => {
    expect(stores.length).toBeGreaterThan(0);

    for (const { node } of stores) {
      const partners = below(tree, node).filter(isFile);

      expect(partners.length).toBeGreaterThan(0);
      expect(
        partners.every((file) => file.allocated === 0 && file.apparent === 0 && file.sharedFiles === 1),
      ).toBe(true);
    }
  });

  it('charges the name the scan meets first and gives the later ones nothing', () => {
    for (const { node, path } of stores) {
      const project = path.slice(0, -1).join('/');
      const modules = at(tree, `${project}/node_modules`);
      const originals = below(tree, modules).filter((file) => file.sharedFiles === 1);
      const partners = below(tree, node).filter(isFile);

      expect(originals.length, project).toBe(partners.length);
      expect(originals.every((file) => file.allocated > 0 && file.apparent > 0)).toBe(true);
      expect(Math.max(...originals.map((file) => file.id))).toBeLessThan(
        Math.min(...partners.map((file) => file.id)),
      );
    }
  });

  it('counts both names in the shared files of a folder, as the backend does', () => {
    const { node, path } = stores[0] as { node: MockNode; path: string[] };
    const project = at(tree, path.slice(0, -1).join('/'));
    const modules = at(tree, `${path.slice(0, -1).join('/')}/node_modules`);

    expect(modules.sharedFiles).toBeGreaterThan(0);
    expect(node.sharedFiles).toBe(modules.sharedFiles);
    expect(project.sharedFiles).toBe(modules.sharedFiles + node.sharedFiles);
    expect(tree.nodes[0]?.sharedFiles).toBe(
      folders(tree)
        .map(({ node: folder }) => folder)
        .filter((folder) => folder.name === 'node_modules' || folder.name === '.pnpm-store')
        .reduce((sum, folder) => sum + folder.sharedFiles, 0),
    );
  });

  it('adds up the sizes of a folder as its own block and what is charged below it', () => {
    for (const { node } of folders(tree).filter(({ node: folder }) => folder.childCount > 0)) {
      const charged = childrenOf(tree, node).reduce((sum, child) => sum + child.allocated, 0);

      expect(node.allocated, node.name).toBe(4096 + charged);
    }
  });
});

describe('the personal folders of the demo tree', () => {
  it('names Downloads, for every demo system', () => {
    expect(treeOf('/').userDirs?.Downloads).toBe('Downloads');
    expect(treeOf('/home/demo').userDirs?.Downloads).toBe('Downloads');
    expect(treeOf('C:\\', { os: 'windows' }).userDirs?.Downloads).toBe('Downloads');
    expect(treeOf('/', { os: 'macos' }).userDirs?.Downloads).toBe('Downloads');
  });
});
