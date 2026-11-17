import { describe, expect, it } from 'vitest';
import { isApiError } from './client';
import { OFFTOPIC_TEXT } from './mock-scan-question';
import { askSketch, DAY, GIB, KIB, MIB, NOW, pathsOf } from './mock-scan-question.testkit';
import type { Sketch } from './mock-sketch';

/*
 * What a person asking in their own words gets, on hand-made trees and without a model: a tool (docker, steam), a
 * personal folder by any of its names, a name that fits two folders, media, a folder with nothing to clean, and the
 * sentences of the answer. The cases and their strings are those of the backend's `question/tests/behaviour.rs`, on the
 * mock's own rules and tree. The words decide whatever they can: these questions are settled without the keyword
 * reader that stands in for the model, or (the bare names) by the folder names of the tree alone.
 */

const STALE = 'that choice is no longer available, so ask again';

/** Both Docker places, a Trash, an npm cache and a project: what "docker" must be told apart from. */
function dockerDisk(): Sketch {
  return {
    files: {
      '/home/u/Projekty/app/src/main.rs': MIB,
      '/var/lib/docker/overlay2/l/layer': 8 * GIB,
      '/home/u/.docker/desktop/vms/DockerDesktop/disk.raw': 12 * GIB,
      '/home/u/.docker/config.json': KIB,
      '/home/u/.local/share/Trash/files/old.iso': 2 * GIB,
      '/home/u/.npm/_cacache/x': GIB,
    },
  };
}

const seeded = (): Sketch => ({
  files: {
    '/home/u/Projekty/app/src/main.rs': MIB,
    '/home/u/Projekty/app/target/debug/x': 400 * MIB,
    '/home/u/Projekty/app/Cargo.toml': 1,
    '/home/u/Projekty/live/target/x': 90 * MIB,
    '/home/u/Projekty/live/Cargo.toml': 1,
    '/home/u/.local/share/Steam/steamapps/shadercache/s': 300 * MIB,
    '/home/u/.local/share/Steam/steamapps/common/g/data': 4 * GIB,
    '/home/u/.cache/huggingface/m.bin': GIB,
  },
});

describe('a tool is the tool and nothing else', () => {
  it.each([
    'can i safely delete the docker images',
    'czy mogę usunąć obrazy dockera?',
    'how big is my docker data?',
    'where is docker?',
  ])('%s is both Docker places and nothing else', (question) => {
    const answer = askSketch(dockerDisk(), question);

    expect([...pathsOf(answer)].sort()).toEqual(['/var/lib/docker', '~/.docker/desktop/vms']);
    expect(answer.clarify).toBeUndefined();

    for (const other of ['Trash', '.npm', 'pacman', '.cache', 'Steam', 'node_modules']) {
      expect(answer.answer, question).not.toContain(other);
    }
  });

  it('answers "can i safely delete the docker images" once, about both Docker places', () => {
    const answer = askSketch(dockerDisk(), 'can i safely delete the docker images');

    expect(answer.answer).toBe(
      'No, not without a look: “~/.docker/desktop/vms” (13 GB) and “/var/lib/docker” (8.6 GB) need your review.',
    );
    expect(answer.understood).toEqual({ intent: 'cleanable', kind: 'any', limit: 5 });
  });

  it('leads "how big is my docker data?" with what Docker takes', () => {
    const answer = askSketch(dockerDisk(), 'how big is my docker data?');

    expect(answer.answer).toBe(
      'Docker: 21 GB in 2 items. The biggest folder among the “Docker” folders is “~/.docker/desktop/vms” (13 GB), then “/var/lib/docker” (8.6 GB).',
    );
    expect(answer.understood.scope).toBeUndefined();
  });

  it('says where the rest of a tool is when its fixed place lies outside a home scan', () => {
    const note = '“/var/lib/docker” (Docker data) is outside this scan. Scan / to include it.';

    // A home scan holds the Docker Desktop disk, and Docker's own folder is outside it.
    for (const question of ['how big is my docker data?', 'can i safely delete the docker images']) {
      const answer = askSketch({ ...dockerDisk(), root: '/home/u' }, question);

      expect(answer.answer, question).toContain('~/.docker/desktop/vms');
      expect(pathsOf(answer).includes('/var/lib/docker'), question).toBe(false);
      expect(answer.answer.endsWith(note), `${question}: ${answer.answer}`).toBe(true);
      expect(
        answer.notes.filter((shown) => shown === note),
        question,
      ).toHaveLength(1);
    }

    // A scan of everything holds both places: nothing is outside it.
    expect(askSketch(dockerDisk(), 'how big is my docker data?').answer).not.toContain(
      'is outside this scan',
    );

    // A tool not found at all says where it lives instead, not this note.
    const nothing = askSketch({ ...seeded(), root: '/home/u' }, 'how big is my docker data?').answer;

    expect(nothing.startsWith('No file or folder called “docker” is in this scan')).toBe(true);
    expect(nothing).not.toContain('is outside this scan');
  });

  it('does not say a pick is outside the scan: a choice is only that tool', () => {
    const sketch: Sketch = {
      files: {
        '/home/u/go/pkg/mod/github.com/x/y.go': 3 * GIB,
        '/home/u/Projekty/site/package.json': 1,
        '/home/u/Projekty/site/package-lock.json': 1,
        '/home/u/Projekty/site/node_modules/m/x': 200 * MIB,
      },
      mtimes: { '/home/u/Projekty/site/node_modules': NOW - 200 * DAY },
      root: '/home/u',
    };
    const asked = askSketch(sketch, 'can I delete the modules?');
    const go = asked.clarify?.choices.find((choice) => choice.label === 'Go module cache');

    expect(go).toBeDefined();
    expect(askSketch(sketch, 'can I delete the modules?', go?.id).notes).toEqual([]);
  });

  it.each(['how big is my docker data?', 'can I delete the docker containers?'])(
    'says only that, and where Docker lives, when it is not in the scan: %s',
    (question) => {
      const answer = askSketch(seeded(), question);

      expect(answer.rows).toEqual([]);
      expect(
        answer.answer.startsWith(
          'No file or folder called “docker” is in this scan (“Docker data” lives in /var/lib/docker,',
        ),
      ).toBe(true);
      expect(answer.answer).not.toContain('cache');
      expect(answer.answer).not.toContain('Trash');
    },
  );

  it('lists a Docker place that could not be read, and says it could not be read', () => {
    const sketch: Sketch = {
      files: { ...seeded().files, '/var/lib/docker/x': 1 },
      unreadable: ['/var/lib/docker'],
    };

    expect(askSketch(sketch, 'where is docker?').answer).toBe(
      'Matches for “docker”: “/var/lib/docker” (could not be read).',
    );
    expect(askSketch(sketch, 'can I delete docker?').answer).toBe(
      'No: “/var/lib/docker” is left alone by Platter. Platter could not read it, so its size is unknown.',
    );
  });

  it("reads steam and rust as their rules' folders by any wording", () => {
    const steam = askSketch(seeded(), 'how much space does steam use?');

    expect(
      steam.answer.startsWith('Steam: 4.6 GB in 2 items. The biggest folder among the “Steam” folders is '),
    ).toBe(true);
    expect(pathsOf(steam)).toEqual([
      '~/.local/share/Steam/steamapps/common',
      '~/.local/share/Steam/steamapps/shadercache',
    ]);

    const rust = askSketch(seeded(), 'where are my rust build folders?');

    expect(pathsOf(rust)).toEqual(['~/Projekty/app/target', '~/Projekty/live/target']);
    expect(rust.understood.intent).toBe('find');
    expect(rust.answer.startsWith('Matches for “rust”: ')).toBe(true);
  });
});

describe('personal folders go through the user’s own', () => {
  const polishHome = (extra: Record<string, number> = {}): Sketch => ({
    files: {
      '/home/u/Pobrane/big.iso': 700 * MIB,
      '/home/u/Pobrane/notes.txt': MIB,
      '/home/u/Dokumenty/work/a.odt': 4 * MIB,
      '/home/u/.cache/platter-review4/t/home/me/Downloads/x': 16 * KIB,
      '/home/u/Documents/Codex/a.md': 4096,
      ...extra,
    },
    userDirs: { Downloads: 'Pobrane', Documents: 'Dokumenty', Pictures: 'Obrazy' },
  });

  it.each(['can I delete my Downloads?', 'can I delete my old downloads?', 'czy mogę usunąć Pobrane'])(
    '%s is the user’s own downloads folder, and a namesake elsewhere is never it',
    (question) => {
      const answer = askSketch(polishHome(), question);

      expect(answer.clarify, 'the user dir wins, nothing is asked').toBeUndefined();
      expect(answer.understood.scope?.path).toBe('~/Pobrane');
      expect(answer.answer).not.toContain('.cache');
    },
  );

  it('tells an English name for a folder that has another name here, and not the folder’s own name', () => {
    const english = askSketch(polishHome(), 'can I delete my Downloads?');
    const own = askSketch(polishHome(), 'czy mogę usunąć Pobrane');

    expect(english.notes).toContain('There is no “~/Downloads”. Your downloads folder is “~/Pobrane”.');
    expect(english.answer).toBe(
      '“~/Pobrane” holds 735 MB in 2 files. Platter has no rule for it. Deleting it is your call. There is no “~/Downloads”. Your downloads folder is “~/Pobrane”.',
    );
    expect(own.notes.some((note) => note.includes('There is no'))).toBe(false);
  });

  it('says a personal folder the scan does not hold is missing, and never takes a namesake for it', () => {
    const sketch: Sketch = {
      files: { '/home/u/.cache/fixture/Downloads/x': 16 * KIB, '/home/u/Projekty/a': MIB },
      userDirs: { Downloads: 'Pobrane' },
    };

    const answer = askSketch(sketch, 'can I delete my Downloads?');

    expect(answer.rows).toEqual([]);
    expect(answer.answer).toBe(
      'No file or folder called “Downloads” is in this scan (your downloads folder is “~/Pobrane”).',
    );
  });

  it('follows the machine’s own folders for Documents and Pictures, whatever the language of the question', () => {
    const folder = (question: string): string | undefined =>
      askSketch(polishHome({ '/home/u/Obrazy/a.png': MIB }), question).understood.scope?.path;

    expect(folder('can I delete my Documents?')).toBe('~/Dokumenty');
    expect(folder('can I delete my Dokumenty folder?')).toBe('~/Dokumenty');
    expect(folder('can I delete my Pictures?')).toBe('~/Obrazy');
  });

  it('keeps the folders the machine calls Documents, not the English name', () => {
    const answer = askSketch(polishHome(), 'can I delete my Documents?');

    expect(answer.rows).toEqual([
      expect.objectContaining({ path: '~/Dokumenty', verdict: 'keep', source: 'rule' }),
    ]);
  });

  it('takes an explicit path over a personal folder name, and says where the user’s folder is', () => {
    const answer = askSketch(polishHome(), 'is ~/Documents safe to delete?');

    expect(answer.understood.scope?.path).toBe('~/Documents');
    expect(answer.answer).toBe(
      '“~/Documents” holds 4.1 KB in 1 file. Platter has no rule for it. Deleting it is your call. Your documents folder is “~/Dokumenty”.',
    );
  });
});

describe('a name that fits two folders is asked back', () => {
  const twoBackups = (): Sketch => ({
    files: {
      '/home/u/work/backup/x': 300 * MIB,
      '/home/u/phone/backup/y': 250 * MIB,
      '/home/u/tiny/backup/z': MIB,
      '/home/u/Projekty/a': MIB,
    },
  });
  const question = 'can I delete the backup folder?';

  it('asks back with both folders of a similar size and no choice of the whole scan', () => {
    const answer = askSketch(twoBackups(), question);

    expect(answer.clarify?.prompt).toBe(
      'I found more than one folder called “backup”. Which one do you mean?',
    );
    expect(answer.clarify?.choices.map((choice) => choice.label)).toEqual([
      '~/work/backup',
      '~/phone/backup',
    ]);
    expect(answer.clarify?.choices.map((choice) => choice.id)).toEqual([0, 1]);
    expect(
      answer.clarify?.choices.every((choice) => choice.path === choice.label && choice.node !== undefined),
    ).toBe(true);
    // The prompt is the whole answer: no rows, no notes, no figures.
    expect(answer.answer).toBe(answer.clarify?.prompt);
    expect(answer.segments).toEqual([{ kind: 'text', text: answer.clarify?.prompt }]);
    expect(answer.rows).toEqual([]);
    expect(answer.notes).toEqual([]);
    expect(answer.more).toBe(0);
    expect(answer.understood).toEqual({ intent: 'cleanable', kind: 'any', limit: 5, scopeText: 'backup' });
  });

  it('takes the choice of a pick, and refuses a pick that fits no choice or a question that was not asked back', () => {
    expect(askSketch(twoBackups(), question, 1).understood.scope?.path).toBe('~/phone/backup');
    expect(askSketch(twoBackups(), question, 0).understood.scope?.path).toBe('~/work/backup');
    expect(askSketch(twoBackups(), question, 1).clarify).toBeUndefined();

    for (const refused of [
      () => askSketch(twoBackups(), question, 2),
      () => askSketch(twoBackups(), 'can I delete ~/work/backup?', 0),
      () => askSketch(twoBackups(), question, 1.5),
    ]) {
      try {
        refused();
      } catch (error) {
        expect(isApiError(error) && [error.code, error.message]).toEqual(['invalid', STALE]);

        continue;
      }

      throw new Error('expected a refusal');
    }
  });

  it('does not ask back when one folder of the name is clearly the largest', () => {
    const sketch = twoBackups();

    sketch.files['/home/u/big/backup/w'] = 5 * GIB;

    const answer = askSketch(sketch, question);

    expect(answer.clarify).toBeUndefined();
    expect(answer.understood.scope?.path).toBe('~/big/backup');
  });

  it('asks back for two unrelated tools that fit the words, and a pick answers about that tool alone', () => {
    const sketch = seeded();

    sketch.files['/home/u/Projekty/site/package.json'] = 1;
    sketch.files['/home/u/Projekty/site/package-lock.json'] = 1;
    sketch.files['/home/u/Projekty/site/node_modules/m/x'] = 200 * MIB;
    sketch.files['/home/u/go/pkg/mod/github.com/x/y.go'] = 3 * GIB;
    sketch.mtimes = { '/home/u/Projekty/site/node_modules': NOW - 200 * DAY };

    const asked = askSketch(sketch, 'can I delete the modules?');
    const labels = asked.clarify?.choices.map((choice) => choice.label) ?? [];

    expect(asked.clarify?.prompt).toBe('“modules” could mean more than one thing. Which one do you mean?');
    expect(labels).toContain('node_modules');
    expect(labels).toContain('Go module cache');
    expect(asked.rows).toEqual([]);

    const picked = askSketch(sketch, 'can I delete the modules?', labels.indexOf('Go module cache'));

    expect(pathsOf(picked)).toEqual(['~/go/pkg/mod']);
    expect(picked.clarify).toBeUndefined();
  });
});

describe('media', () => {
  const mediaDisk = (): Sketch => ({
    files: {
      '/home/u/Videos/film.mkv': 3 * GIB,
      '/home/u/Videos/clips/c.mp4': 100 * MIB,
      '/home/u/Pictures/trip/a.jpg': 50 * MIB,
      '/home/u/Pictures/b.png': 10 * MIB,
      '/home/u/Music/album/a.mp3': 40 * MIB,
      '/home/u/Projekty/a': MIB,
    },
    categories: { '/home/u/Videos': 'video', '/home/u/Pictures': 'image', '/home/u/Music': 'audio' },
  });

  it.each(['how much do my videos take?', 'ile miejsca zajmują filmy?'])(
    '%s leads with the video total and lists video folders only',
    (question) => {
      const answer = askSketch(mediaDisk(), question);

      expect(pathsOf(answer)).toEqual(['~/Videos']);
      expect(
        answer.answer.startsWith(
          'Video folders in this scan take 3.3 GB in total (1 folder). The biggest video folder in this scan is “~/Videos” (3.3 GB).',
        ),
      ).toBe(true);
      expect(answer.answer).not.toMatch(/Pictures|Music|Images/);
    },
  );

  it('keeps to the category the question names', () => {
    const photos = askSketch(mediaDisk(), 'how large are my photos?');
    const music = askSketch(mediaDisk(), 'how much does my music take up?');

    expect(photos.answer.startsWith('Image folders in this scan take ')).toBe(true);
    expect(photos.answer).toContain('“~/Pictures”');
    expect(photos.answer).not.toContain('Videos');
    expect(pathsOf(music)).toEqual(['~/Music']);
    expect(music.answer.startsWith('Audio folders in this scan take ')).toBe(true);
  });

  it('says that a media category that is not in the scan is not', () => {
    const answer = askSketch({ files: { '/home/u/Pobrane/big.iso': MIB } }, 'how much do my videos take?');

    expect(answer.answer).toBe('No video folders were found in this scan.');
  });
});

describe('a folder with nothing to clean', () => {
  it('says what it holds and where Platter stands', () => {
    const sketch = seeded();

    sketch.files['/home/u/Notes/n.txt'] = 4 * MIB;

    expect(askSketch(sketch, 'is ~/Notes safe to delete?').answer).toBe(
      '“~/Notes” holds 4.2 MB in 1 file. Platter has no rule for it. Deleting it is your call.',
    );
  });

  it('says that a rule keeps part of it', () => {
    const sketch: Sketch = {
      files: {
        '/home/u/.local/share/Steam/steamapps/common/g/data': 2 * GIB,
        '/home/u/Projekty/a': MIB,
      },
    };
    const answer = askSketch(sketch, 'can I delete ~/.local/share/Steam/steamapps?');

    expect(answer.answer).toContain(
      'Platter keeps part of it, such as “Steam library” (2.1 GB). Deleting the rest is your call.',
    );
    expect(answer.answer).not.toContain('Nothing to clean');
  });
});

describe('the sentences', () => {
  /** Caches of every standing, and forty small Python caches. */
  function manyCaches(): Sketch {
    const files: Record<string, number> = {
      '/home/u/.npm/_cacache/x': 5 * GIB,
      '/home/u/.bun/install/cache/x': 4 * GIB,
      '/home/u/.local/share/Trash/files/x': 3 * GIB,
      '/home/u/.cache/huggingface/m.bin': 8 * GIB,
      '/var/cache/pacman/pkg/a.zst': 2 * GIB,
      '/home/u/.docker/desktop/vms/vm/disk.raw': 20 * GIB,
    };

    for (let number = 0; number < 40; number++) {
      files[`/home/u/Projekty/p${number}/m/__pycache__/x.pyc`] = 30_000;
    }

    return { files };
  }

  it('collapses the small items of a generic answer, and names the biggest to review', () => {
    const answer = askSketch(manyCaches(), 'what can I clean up?');

    expect(answer.answer).toBe(
      'Safe to clean: 13 GB in 43 items, the largest being “~/.npm” (5.4 GB), “~/.bun/install/cache” (4.3 GB), “~/.local/share/Trash” (3.2 GB) and 40 smaller items such as “__pycache__”. For you to review: 32 GB in 3 items, the largest being “~/.docker/desktop/vms” (21 GB), “~/.cache” (8.6 GB) and 1 more item.',
    );
    expect(answer.answer, 'the count of unlisted matches is `more`, not text').not.toContain('not listed');
    expect(answer.more).toBeGreaterThan(0);
    expect(answer.rows, 'the limit is shared between safe and review').toHaveLength(5);
  });

  it('says how many more match when the rows of a subject were cut', () => {
    const files: Record<string, number> = {};
    const mtimes: Record<string, number> = {};

    for (let number = 0; number < 8; number++) {
      files[`/home/u/Projekty/p${number}/package.json`] = 1;
      files[`/home/u/Projekty/p${number}/package-lock.json`] = 1;
      files[`/home/u/Projekty/p${number}/node_modules/m/x`] = (10 + number) * MIB;
      mtimes[`/home/u/Projekty/p${number}/node_modules`] = NOW - 200 * DAY;
    }

    const answer = askSketch({ files, mtimes }, 'can I delete node_modules?');

    expect(answer.answer.startsWith('Yes: “~/Projekty/p7/node_modules” (18 MB), ')).toBe(true);
    expect(answer.answer).toContain(
      ' are safe to clean. Only the largest 5 are named here, and 3 more match.',
    );
    expect(answer.rows).toHaveLength(5);
    expect(answer.more).toBe(3);
  });

  it('gives a Polish question and a wish to clean everything one short note each', () => {
    const count = (notes: string[], note: string): number => notes.filter((one) => one === note).length;
    const english = 'Answers are in English.';
    const notFromHere = 'Platter does not delete anything from here. Use Cleanup to choose.';

    const polish = askSketch(seeded(), 'co mogę usunąć?');

    expect(count(polish.notes, english)).toBe(1);
    expect(polish.answer.endsWith(` ${english}`)).toBe(true);
    expect(polish.notes.some((note) => note.includes('Cleanup'))).toBe(false);

    const everything = askSketch(seeded(), 'clean everything');

    expect(count(everything.notes, notFromHere)).toBe(1);
    expect(everything.notes.some((note) => note.includes('English'))).toBe(false);

    const plain = askSketch(seeded(), 'what can I clean up?');

    expect(plain.notes.some((note) => note.includes('English') || note.includes('Cleanup'))).toBe(false);
  });

  it('tells a question with no word about disks it is off topic', () => {
    const answer = askSketch(seeded(), 'write me a poem');

    expect(answer.understood.intent).toBe('offtopic');
    expect(answer.rows).toEqual([]);
    expect(answer.answer).toBe(OFFTOPIC_TEXT);
  });

  it('says once that nothing is labelled, with the note for what to do', () => {
    const answer = askSketch(seeded(), 'what did the model label?');

    expect(answer.notes).toEqual(['Use “Label folders” in the Advisor view.']);
    expect(answer.answer).toBe(
      'The model has not labelled any folder in this scan. Use “Label folders” in the Advisor view.',
    );
  });

  it('answers about the tool named and nothing else when the scan holds none of it', () => {
    const answer = askSketch(manyCaches(), 'where is node_modules?');

    expect(answer.rows).toEqual([]);
    expect(answer.answer).toBe('No file or folder called “node_modules” is in this scan.');
  });
});
