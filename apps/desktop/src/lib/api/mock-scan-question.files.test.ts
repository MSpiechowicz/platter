import { describe, expect, it } from 'vitest';
import { isApiError } from './client';
import { answerQuestion } from './mock-scan-question';
import { askSketch, DAY, GIB, MIB, NOW, sketchContext } from './mock-scan-question.testkit';
import type { Sketch } from './mock-sketch';

/*
 * A question about a file, on hand-made trees and without a model: a typed path, a quoted path with white space, a bare
 * file name, a name that is a file and a folder, and the sentences of the answer (docs/architecture.md, "A file").
 */

const disk = (): Sketch => ({
  files: {
    '/home/u/Downloads/old.iso': 4 * GIB,
    '/home/u/Downloads/new.txt': MIB,
    '/home/u/My Files/a b.txt': 3 * MIB,
    '/home/u/Documents/report.pdf': MIB,
    '/home/u/dup/same.dat': 2 * GIB,
    '/home/u/other/same.dat': GIB,
    '/home/u/x/big.dat': 2 * GIB,
    '/home/u/y/big.dat': 100 * MIB,
    '/home/u/solo/report': MIB,
    '/home/u/a/report/x': 10 * MIB,
    '/home/u/pics/photo.jpg': MIB,
    '/home/u/photo.jpg.d/x': 5 * MIB,
    '/home/u/hello.dat': 5 * MIB,
    '/home/u/sub/hello.dat': 9 * MIB,
    '/home/u/Mine/keep.txt': MIB,
  },
  mtimes: {
    '/home/u/Downloads/old.iso': NOW - 400 * DAY,
    '/home/u/Downloads/new.txt': NOW - 3 * DAY,
  },
});

describe('a file named by its path', () => {
  it('is one row marked as a file, with what it is and where Platter stands', () => {
    const answer = askSketch(disk(), 'how big is ~/Downloads/old.iso');

    expect(answer.rows).toEqual([
      expect.objectContaining({ path: '~/Downloads/old.iso', file: true, verdict: 'review', source: 'rule' }),
    ]);
    expect(answer.understood.scope).toMatchObject({ path: '~/Downloads/old.iso', file: true });
    expect(answer.answer).toContain(
      '“~/Downloads/old.iso” is a file of 4.3 GB, last changed 1 year ago. It needs your review (“Old downloads”).',
    );
    expect(answer.segments[0]).toMatchObject({ kind: 'path', node: answer.rows[0]?.node });
  });

  it('is answered the same way whatever the intent is', () => {
    for (const question of [
      'how big is ~/Downloads/old.iso',
      'can i safely delete ~/Downloads/old.iso?',
      'why is ~/Downloads/old.iso so big?',
    ]) {
      const answer = askSketch(disk(), question);

      expect(answer.rows, question).toHaveLength(1);
      expect(answer.rows[0]?.file, question).toBe(true);
      expect(answer.answer.startsWith('“~/Downloads/old.iso” is a file of 4.3 GB'), question).toBe(true);
    }
  });

  it('tells how long ago it changed in days, and leaves the age out without a modification time', () => {
    expect(askSketch(disk(), 'how big is ~/Downloads/new.txt').answer).toContain(
      'is a file of 1.0 MB, last changed 3 days ago.',
    );

    const sketch = disk();

    sketch.mtimes = { '/home/u/Downloads/new.txt': 0 };

    expect(askSketch(sketch, 'how big is ~/Downloads/new.txt').answer).toContain('is a file of 1.0 MB.');
  });

  it.each([
    [0, 'today'],
    [1, '1 day ago'],
    [30, '30 days ago'],
    [31, '1 month ago'],
    [75, '2 months ago'],
    [364, '12 months ago'],
    [365, '1 year ago'],
    [800, '2 years ago'],
  ])('is %d days old: %s', (days, words) => {
    const sketch = disk();

    sketch.mtimes = { '/home/u/Downloads/new.txt': NOW - days * DAY };

    expect(askSketch(sketch, 'how big is ~/Downloads/new.txt').answer).toContain(`last changed ${words}.`);
  });

  it('says a protected file is left alone, with the guard words', () => {
    const answer = askSketch(disk(), 'is ~/Documents/report.pdf safe to delete?');

    expect(answer.rows[0]).toMatchObject({ file: true, verdict: 'keep', source: 'guard' });
    expect(answer.answer).toContain('Platter leaves it alone. This is a protected place, or inside one.');
  });

  it('says no rule covers a file nothing decides about', () => {
    expect(askSketch(disk(), 'how big is ~/Downloads/new.txt').answer).toContain(
      'No rule covers it, so deleting it is your call.',
    );
  });

  it('says what the rule says after a review, and nothing after a safe or a kept file', () => {
    const review = askSketch(disk(), 'how big is ~/Downloads/old.iso');

    expect(review.answer).toBe(
      '“~/Downloads/old.iso” is a file of 4.3 GB, last changed 1 year ago. It needs your review (“Old downloads”). ' +
        '1 file in Downloads not modified for 365 days or more. These are personal files, so only you can tell ' +
        'whether you still need them.',
    );

    const kept = answerQuestion(
      sketchContext(disk(), { user: [{ verdict: 'keep', folder: '/home/u/Mine' }] }),
      'how big is ~/Mine/keep.txt',
    );

    expect(kept.rows[0]).toMatchObject({ file: true, verdict: 'keep', source: 'user' });
    expect(kept.answer).toBe(
      '“~/Mine/keep.txt” is a file of 1.0 MB, last changed 1 day ago. It is kept (“Folders you keep”).',
    );
  });

  it('reads a path in straight quotes as one path, so a name with a space is found', () => {
    const answer = askSketch(disk(), 'is "~/My Files/a b.txt" safe to delete?');

    expect(answer.rows.map((row) => row.path)).toEqual(['~/My Files/a b.txt']);
    expect(answer.answer.startsWith('“~/My Files/a b.txt” is a file of 3.1 MB')).toBe(true);
  });

  it('says a path that is neither a file nor a folder is not in the scan', () => {
    const answer = askSketch(disk(), 'how big is ~/Downloads/missing.iso');

    expect(answer.rows).toEqual([]);
    expect(answer.notes).toContain('No file or folder called “~/Downloads/missing.iso” is in this scan');
  });
});

describe('a file named by its path in a search', () => {
  it('is one row marked as a file and says what was looked for, never a scope', () => {
    const answer = askSketch(disk(), 'where is ~/Downloads/old.iso');

    expect(answer.understood.intent).toBe('find');
    expect(answer.understood.scope).toBeUndefined();
    expect(answer.understood.scopeText).toBe('~/Downloads/old.iso');
    expect(answer.rows).toEqual([expect.objectContaining({ path: '~/Downloads/old.iso', file: true })]);
    expect(answer.answer.startsWith('“~/Downloads/old.iso” is a file of 4.3 GB')).toBe(true);
  });
});

describe('a file named by its bare name', () => {
  it('is that file when it is the only one called that, ignoring case', () => {
    const answer = askSketch(disk(), 'how big is OLD.ISO');

    expect(answer.rows.map((row) => row.path)).toEqual(['~/Downloads/old.iso']);
    expect(answer.rows[0]?.file).toBe(true);
  });

  it('is the largest alone when the others are a quarter of it or less', () => {
    const answer = askSketch(disk(), 'how big is big.dat');

    expect(answer.clarify).toBeUndefined();
    expect(answer.rows.map((row) => row.path)).toEqual(['~/x/big.dat']);
  });

  it('asks back between files of a similar size', () => {
    const asked = askSketch(disk(), 'how big is same.dat');

    expect(asked.clarify?.prompt).toBe(
      'I found more than one file called “same.dat”. Which one do you mean?',
    );
    expect(asked.clarify?.choices.map((choice) => choice.label)).toEqual([
      '~/dup/same.dat',
      '~/other/same.dat',
    ]);

    const picked = askSketch(disk(), 'how big is same.dat', 1);

    expect(picked.rows.map((row) => row.path)).toEqual(['~/other/same.dat']);
    expect(picked.rows[0]?.file).toBe(true);
  });

  it('is the file at `~/<name>` alone, even when another of that name is of a similar size', () => {
    const answer = askSketch(disk(), 'how big is hello.dat');

    expect(answer.clarify).toBeUndefined();
    expect(answer.rows.map((row) => row.path)).toEqual(['~/hello.dat']);
    expect(answer.rows[0]?.file).toBe(true);
  });

  it('beats a folder that only holds the name', () => {
    const answer = askSketch(disk(), 'how big is photo.jpg');

    expect(answer.clarify).toBeUndefined();
    expect(answer.rows.map((row) => row.path)).toEqual(['~/pics/photo.jpg']);
  });

  it('asks back between a file and a folder of that name, then answers about the one picked', () => {
    const asked = askSketch(disk(), 'how big is report');

    expect(asked.clarify?.prompt).toBe(
      'I found more than one file or folder called “report”. Which one do you mean?',
    );
    expect(asked.clarify?.choices.map((choice) => choice.label)).toEqual([
      '~/a/report (folder)',
      '~/solo/report (file)',
    ]);
    expect(asked.rows).toEqual([]);

    const file = askSketch(disk(), 'how big is report', 1);

    expect(file.rows[0]).toMatchObject({ path: '~/solo/report', file: true });
    expect(file.answer.startsWith('“~/solo/report” is a file of 1.0 MB')).toBe(true);

    const folder = askSketch(disk(), 'how big is report', 0);

    expect(folder.rows[0]?.file).toBeUndefined();
  });

  it('offers at most 4 choices, folders first, keeping room for a file', () => {
    const files: Record<string, number> = { '/home/u/z/item': GIB };

    for (const name of ['a', 'b', 'c', 'd', 'e']) {
      files[`/home/u/${name}/item/x`] = GIB;
    }

    const asked = askSketch({ files }, 'how big is item');
    const labels = asked.clarify?.choices.map((choice) => choice.label) ?? [];

    expect(labels).toHaveLength(4);
    expect(labels.slice(0, 3).every((label) => label.endsWith('(folder)'))).toBe(true);
    expect(labels[3]).toBe('~/z/item (file)');
  });

  it('refuses a choice that is not one of those asked', () => {
    try {
      askSketch(disk(), 'how big is report', 7);
      expect.unreachable();
    } catch (error) {
      expect(isApiError(error)).toBe(true);
    }
  });
});

describe('find', () => {
  it('finds a file by its name, as a row marked as a file', () => {
    const answer = askSketch(disk(), 'where is the file old.iso?');

    expect(answer.rows).toEqual([expect.objectContaining({ path: '~/Downloads/old.iso', file: true })]);
    expect(answer.answer).toBe('Matches for “old.iso”: “~/Downloads/old.iso” (4.3 GB).');
  });

  it('needs no other word about disks when the name looks like a file name', () => {
    for (const question of ['where is old.iso?', 'where is OLD.ISO', 'gdzie jest old.iso?']) {
      const answer = askSketch(disk(), question);

      expect(answer.understood.intent, question).toBe('find');
      expect(
        answer.rows.map((row) => [row.path, row.file]),
        question,
      ).toEqual([['~/Downloads/old.iso', true]]);
    }
  });

  it('still reads a question with no disk word and no file-like name as off topic', () => {
    for (const question of ['where is my poem?', 'write me a poem about v1.', 'what is 3 plus 4']) {
      expect(askSketch(disk(), question).understood.intent, question).toBe('offtopic');
    }
  });

  it('lists folders and files together, the exact name first', () => {
    const answer = askSketch(disk(), 'where is the file photo.jpg?');

    expect(answer.rows.map((row) => [row.path, row.file === true])).toEqual([
      ['~/pics/photo.jpg', true],
      ['~/photo.jpg.d', false],
    ]);
  });

  it('says a name that is neither a file nor a folder is not in the scan', () => {
    const answer = askSketch(disk(), 'where is the folder called nothing-xyz?');

    expect(answer.rows).toEqual([]);
    expect(answer.notes).toEqual(['No file or folder called “nothing-xyz” is in this scan']);
  });
});
