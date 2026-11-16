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

  it('keeps a directory when no child reaches the share, and does not swap it for a smaller one', () => {
    // 790 of 1000 is 79%, just under the threshold; one child directory would only swap `home` for a smaller folder.
    expect(names({ files: { '/h/home/user/data.bin': 790, '/h/home/readme': 210 } }, '/h')).toEqual(['home']);
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

    // `home` is dominated by `sam`, which is dominated by nothing; there are fewer than eight candidates, so `sam`
    // (largest, with four child folders) is expanded too, and its children are not dominated either.
    expect(names({ files }, '/')).toEqual(['.local', '.docker', 'misc', 'Projekty', 'var', 'usr', 'etc']);
  });
});
