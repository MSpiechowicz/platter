import { describe, expect, it } from 'vitest';
import { nodeAt, sketchTree, type Sketch } from './mock-sketch';
import { ruleView, suggestionsOf } from './mock-suggestions';
import type { SuggestionDto } from './types';

// The rule table of the backend, on small hand-made trees: what the backend's tests of the scan root, a pnpm workspace,
// the Downloads folder and the units of the reasons say, said again of the mock.

const NOW = 1_750_000_000;
const DAY = 86_400;
const GIB = 2 ** 30;
const OLD = NOW - 90 * DAY;

const suggestionsFor = (sketch: Sketch): SuggestionDto[] => suggestionsOf(sketchTree(sketch)).suggestions;
const idsFor = (sketch: Sketch): string[] => suggestionsFor(sketch).map((entry) => entry.id);

describe('the scan root', () => {
  it('is never a suggestion: a scan of a cache folder itself lists nothing of it', () => {
    const files = { '/var/cache/pacman/pkg/a.pkg': 400, '/var/cache/pacman/pkg/b.pkg': 100 };

    expect(suggestionsFor({ files, root: '/var/cache/pacman/pkg' })).toEqual([]);
    expect(idsFor({ files, root: '/var/cache' })).toEqual(['pacman-cache']);

    const home = { '/home/u/.cache/app/x': 500, '/home/u/Documents/d': 5 };

    expect(idsFor({ files: home, root: '/home/u/.cache' })).toEqual([]);
    expect(idsFor({ files: home, root: '/home/u' })).toEqual(['app-cache', 'documents']);
  });

  it('is still kept when a keep rule claims it, and says it would be claimed otherwise', () => {
    const files = { '/home/u/Documents/d': 500, '/home/u/.cache/app/x': 500 };

    expect(idsFor({ files, root: '/home/u/Documents' })).toEqual(['documents']);
    expect(ruleView(sketchTree({ files, root: '/home/u/Documents' })).rootClaimed).toBe(false);
    expect(ruleView(sketchTree({ files, root: '/home/u/.cache' })).rootClaimed).toBe(true);
    expect(ruleView(sketchTree({ files, root: '/home/u' })).rootClaimed).toBe(false);
  });
});

describe('node_modules of a pnpm workspace', () => {
  const MEMBER = '/home/u/ws/pkgs/a';
  const base = (extra: Record<string, number>, root = '/', manifest = 'package.json'): Sketch => ({
    files: {
      [`${MEMBER}/${manifest}`]: 1,
      [`${MEMBER}/node_modules/x/index.js`]: 5000,
      ...extra,
    },
    mtimes: { [`${MEMBER}/node_modules`]: OLD },
    root,
  });
  const WORKSPACE = { '/home/u/ws/pnpm-workspace.yaml': 1, '/home/u/ws/pnpm-lock.yaml': 1 };

  it('is suggested for a member that has no lockfile of its own, when a folder above holds both files', () => {
    const suggestions = suggestionsFor(base(WORKSPACE));

    expect(suggestions.find((entry) => entry.id === 'node-modules')).toMatchObject({
      verdict: 'safe',
      path: '~/ws/pkgs/a/node_modules',
    });
  });

  it('is suggested when the workspace root is the scan root, but not when it is above it', () => {
    expect(idsFor(base(WORKSPACE, '/home/u/ws'))).toEqual(['node-modules']);
    expect(idsFor(base(WORKSPACE, '/home/u/ws/pkgs'))).toEqual([]);
  });

  it('is not suggested with one of the files only, or when one is a folder', () => {
    expect(idsFor(base({ '/home/u/ws/pnpm-workspace.yaml': 1 }))).toEqual([]);
    expect(idsFor(base({ '/home/u/ws/pnpm-lock.yaml': 1 }))).toEqual([]);
    expect(idsFor(base({ '/home/u/ws/pnpm-workspace.yaml': 1, '/home/u/ws/pnpm-lock.yaml/x': 1 }))).toEqual(
      [],
    );
  });

  it('is not suggested under a tool folder, workspace or not', () => {
    const inTool = {
      '/home/u/.nvm/ws/pnpm-workspace.yaml': 1,
      '/home/u/.nvm/ws/pnpm-lock.yaml': 1,
      '/home/u/.nvm/ws/a/package.json': 1,
      '/home/u/.nvm/ws/a/node_modules/x/i.js': 5000,
    };

    expect(idsFor({ files: inTool, mtimes: { '/home/u/.nvm/ws/a/node_modules': OLD } })).toEqual([]);
  });

  it('still needs the package.json next to it', () => {
    expect(idsFor(base(WORKSPACE, '/', 'README.md'))).toEqual([]);
  });
});

describe('old downloads', () => {
  const files = {
    '/home/u/Pobrane/old.iso': 4000,
    '/home/u/Downloads/other.iso': 3000,
    '/home/u/Documents/d': 1,
  };
  const mtimes = {
    '/home/u/Pobrane/old.iso': NOW - 400 * DAY,
    '/home/u/Downloads/other.iso': NOW - 400 * DAY,
  };

  it('follow the folder the machine calls Downloads, and not another one', () => {
    const found = suggestionsFor({ files, mtimes, userDirs: { Downloads: 'Pobrane' } }).find(
      (entry) => entry.id === 'old-downloads',
    );

    expect(found).toMatchObject({ path: '~/Pobrane/old.iso', items: 1, allocated: 4000 });
  });

  it('are in ~/Downloads when the machine says nothing else', () => {
    const found = suggestionsFor({ files, mtimes }).find((entry) => entry.id === 'old-downloads');

    expect(found).toMatchObject({ path: '~/Downloads/other.iso', items: 1, allocated: 3000 });
  });
});

describe('the sizes in the reasons', () => {
  const tree = sketchTree({
    files: { '/home/u/vm/disk.img': 2 * GIB, '/home/u/x': 1 },
    categories: { '/home/u/vm/disk.img': 'diskImage' },
  });
  const reason = (units?: 'GB' | 'GiB'): string | undefined =>
    suggestionsOf(tree, [], [], units).suggestions.find((entry) => entry.id === 'disk-images')?.reason;

  it('are written in decimal units by default and when GB is asked', () => {
    expect(reason()).toMatch(/^1 image of 1\.1 GB or more in your home folder\./);
    expect(reason('GB')).toBe(reason());
  });

  it('are written in binary units when GiB is asked, as the backend writes them', () => {
    expect(reason('GiB')).toMatch(/^1 image of 1\.0 GiB or more in your home folder\./);
  });

  it('are binary in what the advisor says of a folder, as the backend does', () => {
    const view = ruleView(tree);
    const node = nodeAt(tree, '/home/u/vm/disk.img');

    expect(view.covering(node)?.reason).toContain('1.0 GiB or more');
  });
});
