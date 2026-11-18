import { describe, expect, it } from 'vitest';
import { largestDirs } from './mock-summary';
import { nodeAt, sketchTree, type Sketch } from './mock-sketch';

// The scenarios of the backend's tests of `largest_dirs` (`crates/platter-core/src/query/tests.rs`), so the mock
// and the backend cannot answer differently for the same folders.

function names(sketch: Sketch, folder: string): string[] {
  const tree = sketchTree(sketch);

  return largestDirs(tree, nodeAt(tree, folder)).map((dir) => dir.name);
}

describe('largestDirs', () => {
  it('leaves out a directory dominated by its child', () => {
    // /h/home/user holds 1000 of its parent's 1100 bytes (about 91%), so only the deeper one is listed.
    expect(
      names({ files: { '/h/home/user/data.bin': 1000, '/h/home/readme': 100, '/h/other/x.bin': 50 } }, '/h'),
    ).toEqual(['user', 'other']);
  });

  it('keeps a directory when no child reaches the share and it is not most of the node', () => {
    // 790 of home's 1000 is 79%, just under the threshold, and home is 1000 of the folder's 1500: only 67%.
    expect(
      names({ files: { '/h/home/user/data.bin': 790, '/h/home/readme': 210, '/h/other/x.bin': 500 } }, '/h'),
    ).toEqual(['home', 'other']);
  });

  it('replaces a directory holding at least 80% of the node by its child directories, even without a dominant child', () => {
    // `a` is 800 of 1000 (exactly 80%) and its children hold 50% and 37.5% of it, so no child dominates it. Seven small
    // folders make eight candidates, so nothing is expanded afterwards: without the rule `a` would stay in the list.
    const files: Record<string, number> = { '/r/a/a1/x.bin': 400, '/r/a/a2/x.bin': 300, '/r/a/note': 100 };

    for (let i = 0; i < 8; i++) {
      files[`/r/s${i}/x.bin`] = 25;
    }

    expect(names({ files }, '/r')).toEqual(['a1', 'a2', 's0', 's1', 's2', 's3', 's4', 's5']);
  });

  it('keeps a directory holding just under 80% of the node when no child dominates it', () => {
    const files: Record<string, number> = { '/r/a/a1/x.bin': 400, '/r/a/a2/x.bin': 300, '/r/a/note': 90 };

    for (let i = 0; i < 7; i++) {
      files[`/r/s${i}/x.bin`] = 30;
    }

    expect(names({ files }, '/r')).toEqual(['a', 's0', 's1', 's2', 's3', 's4', 's5', 's6']);
  });

  it('replaces a directory holding most of the node by its only child directory', () => {
    // home is all of `/h` and has one child directory, `user`, which is only 79% of it.
    expect(names({ files: { '/h/home/user/data.bin': 790, '/h/home/readme': 210 } }, '/h')).toEqual(['user']);
  });

  it('keeps a big folder of only files, however much of the node it holds', () => {
    expect(names({ files: { '/r/big/a.bin': 500, '/r/big/b.bin': 400, '/r/s/x.bin': 100 } }, '/r')).toEqual([
      'big',
      's',
    ]);
  });

  it('keeps a folder of loose files with one tiny subfolder, at 85% of the node', () => {
    // `a` is 100 of 117 (about 85%), but its only subfolder holds 5% of it: the bytes are in the files, so `a` stays.
    expect(names({ files: { '/r/a/x.bin': 95, '/r/a/sub/y.bin': 5, '/r/s/z.bin': 17 } }, '/r')).toEqual([
      'a',
      's',
    ]);
  });

  it('replaces a folder at 85% of the node when its subfolders hold 60% of it', () => {
    // The two subfolders hold 30 + 30 of the folder's 100, and neither dominates it.
    expect(
      names({ files: { '/r/a/x.bin': 40, '/r/a/b/y.bin': 30, '/r/a/c/y.bin': 30, '/r/s/z.bin': 17 } }, '/r'),
    ).toEqual(['b', 'c', 's']);
  });

  it('drops a directory whose child holds exactly 80%', () => {
    expect(names({ files: { '/h/home/user/data.bin': 800, '/h/home/readme': 200 } }, '/h')).toEqual(['user']);
  });

  it('follows a dominant chain to its bottom', () => {
    expect(
      names({ files: { '/h/a/b/c/data.bin': 1000, '/h/a/note': 10, '/h/z/data.bin': 5 } }, '/h'),
    ).toEqual(['c', 'z']);
  });

  it('follows a dominant chain for at most six levels', () => {
    // d1 is level 1, so d6 is level 6, the deepest a candidate may be.
    expect(names({ files: { '/h/d1/d2/d3/d4/d5/d6/d7/d8/data.bin': 1000 } }, '/h')).toEqual(['d6']);
  });

  it('fills the list after leaving dominated directories out', () => {
    const files: Record<string, number> = {};

    for (let i = 0; i < 5; i++) {
      files[`/r/c${i}/g${i}/data.bin`] = 1000 + i;
    }

    for (let i = 0; i < 5; i++) {
      files[`/r/s${i}/data.bin`] = 10 + i;
    }

    expect(names({ files }, '/r')).toEqual(['g4', 'g3', 'g2', 'g1', 'g0', 's4', 's3', 's2']);
  });

  it('expands the largest folders to fill the list, and orders equal sizes by id', () => {
    const files: Record<string, number> = {
      '/r/a/a1/x.bin': 300,
      '/r/a/a2/x.bin': 300,
      '/r/a/a3/x.bin': 300,
      '/r/b/b1/x.bin': 200,
      '/r/b/b2/x.bin': 190,
      '/r/b/note': 100,
      '/r/d/d1/x.bin': 150,
      '/r/d/d2/x.bin': 150,
      '/r/d/d3/x.bin': 150,
      '/r/c/x.bin': 100,
    };

    // 4 candidates -> expand a (6) -> expand b (7) -> expand d (9); the eight largest remain and `c` falls out.
    expect(names({ files }, '/r')).toEqual(['a1', 'a2', 'a3', 'b1', 'b2', 'd1', 'd2', 'd3']);
  });

  it('does not expand a folder that would not add one', () => {
    expect(
      names(
        { files: { '/r/a/a1/x.bin': 400, '/r/a/a2/x.bin': 300, '/r/b/b1/x.bin': 150, '/r/b/y.bin': 100 } },
        '/r',
      ),
    ).toEqual(['a1', 'a2', 'b']);
  });

  it('applies the dominance rule again after expanding', () => {
    // `a` splits into a1 (500, all of it in a1x) and a2 (400); the dominated a1 is replaced.
    expect(
      names({ files: { '/r/a/a1/a1x/x.bin': 500, '/r/a/a2/x.bin': 400, '/r/b/x.bin': 50 } }, '/r'),
    ).toEqual(['a1x', 'a2', 'b']);
  });

  it('returns fewer than eight when nothing can be expanded', () => {
    expect(names({ files: { '/r/a/x.bin': 300, '/r/b/x.bin': 200, '/r/c/x.bin': 100 } }, '/r')).toEqual([
      'a',
      'b',
      'c',
    ]);
  });

  it('never lists empty directories, unreadable ones or mounts, and does not expand a folder of empty ones', () => {
    const sketch: Sketch = {
      files: { '/r/real/x.bin': 100 },
      dirs: ['/r/empty', '/r/shell/e1', '/r/shell/e2'],
      own: { '/r/shell': 4096 },
      mounts: ['/r/usb'],
      unreadable: ['/r/locked'],
    };

    expect(names(sketch, '/r')).toEqual(['shell', 'real']);
  });

  it('breaks ties between equal sizes by id, so the answer is always the same', () => {
    const files: Record<string, number> = {};

    for (let i = 0; i < 4; i++) {
      for (let j = 0; j < 4; j++) {
        files[`/r/d${i}/e${j}/x.bin`] = 10;
      }
    }

    // d0 is expanded first (lowest id of four equal folders), then d1; the remaining d2 and d3 are the largest,
    // then the e folders of d0 (lower ids) before those of d1.
    expect(names({ files }, '/r')).toEqual(['d2', 'd3', 'e0', 'e1', 'e2', 'e3', 'e0', 'e1']);
  });

  it('on a disk where one folder holds 97% lists that folder once, then the small ones, like the backend', () => {
    // A `/`-shaped disk: /home holds 97%, all of it in one user folder whose biggest folders hold 53% and 21%.
    const files: Record<string, number> = {
      '/home/sam/.local/x.bin': 530_000,
      '/home/sam/.docker/x.bin': 210_000,
      '/home/sam/Projekty/x.bin': 100_000,
      '/home/sam/misc/x.bin': 130_000,
      '/var/x.bin': 20_000,
      '/usr/x.bin': 9_000,
      '/etc/x.bin': 1_000,
    };

    // `home` is dominated by `sam`. `sam` holds 97% of the disk and has child folders, so it is replaced by them too,
    // and none of those is dominated or holds 80% of the disk.
    expect(names({ files }, '/')).toEqual(['.local', '.docker', 'misc', 'Projekty', 'var', 'usr', 'etc']);
  });
});
