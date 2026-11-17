import { describe, expect, it } from 'vitest';
import { interpret, readQuestion, type Reading } from './mock-question-reader';
import { fold, quotedPath, roleOfName, signals, wordsOfQuestion } from './mock-question-signals';
import type { ScanKind } from './types';

/*
 * The mock's reading of a question is the backend's (`question/reading.rs`, `platter_ai::reading`): the words come
 * before the model, and the model only fills what they leave open. The evaluation set is the backend's own
 * (`question/tests/reading.json`: 60 natural questions, English and Polish), read with no model at all.
 */

/** The subject as the evaluation set writes it: `none`, `path:`, `role:`, `rule:<tool>`, `name:`. */
function symbolOf(reading: Reading): string {
  const { subject } = reading;

  switch (subject.kind) {
    case 'none':
      return 'none';
    case 'path':
      return `path:${subject.text}`;
    case 'role':
      return `role:${subject.role.toLowerCase()}`;
    case 'rules':
      return `rule:${(subject.ids[0] ?? '').split('-')[0]}`;
    case 'name':
      return `name:${subject.text}`;
  }
}

// question, intent, kind, subject
const EVALUATION: readonly (readonly [string, string, ScanKind, string])[] = [
  ['which folder does take the biggest amount of data?', 'largest', 'any', 'none'],
  ['can i safely delete the docker images', 'cleanable', 'any', 'rule:docker'],
  ["what's eating my disk", 'largest', 'any', 'none'],
  ['what can I clean up?', 'cleanable', 'any', 'none'],
  ['can I delete node_modules?', 'cleanable', 'any', 'rule:node'],
  ['is it safe to remove the Steam shader cache', 'cleanable', 'any', 'rule:steam'],
  ['why is ~/.cache so big?', 'explain', 'any', 'path:~/.cache'],
  ['where are my rust build folders?', 'find', 'any', 'rule:rust'],
  ['how much do my videos take?', 'largest', 'media', 'none'],
  ['what did the model label?', 'labelled', 'any', 'none'],
  ['can I delete my Downloads?', 'cleanable', 'any', 'role:downloads'],
  ['is ~/Documents safe to delete?', 'cleanable', 'any', 'path:~/Documents'],
  ['what are my biggest caches?', 'largest', 'cache', 'none'],
  ['clean everything', 'cleanable', 'any', 'none'],
  ['co zajmuje najwięcej miejsca?', 'largest', 'any', 'none'],
  ['co mogę usunąć?', 'cleanable', 'any', 'none'],
  ['czy mogę usunąć obrazy dockera?', 'cleanable', 'any', 'rule:docker'],
  ['write me a poem', 'offtopic', 'any', 'none'],
  ['how big is my docker data?', 'largest', 'any', 'rule:docker'],
  ['where is docker?', 'find', 'any', 'rule:docker'],
  ['how much space does steam use?', 'largest', 'any', 'rule:steam'],
  ["what's in ~/.cache?", 'explain', 'any', 'path:~/.cache'],
  ['can I delete ~/Pobrane?', 'cleanable', 'any', 'path:~/Pobrane'],
  ['jakie foldery są największe?', 'largest', 'any', 'none'],
  ['ile miejsca zajmują filmy?', 'largest', 'media', 'none'],
  ['show me the 3 biggest folders in ~/Projekty', 'largest', 'any', 'path:~/Projekty'],
  ['how much space is the trash using?', 'largest', 'any', 'rule:trash'],
  ['wyczyść kosz', 'cleanable', 'any', 'rule:trash'],
  ['can I get rid of the pip stuff', 'cleanable', 'any', 'rule:python'],
  ['why is my home folder so big?', 'explain', 'any', 'path:~'],
  ['where is node_modules?', 'find', 'any', 'rule:node'],
  ['dlaczego .cache jest taki duży?', 'explain', 'any', 'name:.cache'],
  ['gdzie jest docker?', 'find', 'any', 'rule:docker'],
  ['what are the biggest build folders?', 'largest', 'build', 'none'],
  ['how much does my music take up?', 'largest', 'media', 'none'],
  ['can I delete my old downloads?', 'cleanable', 'any', 'role:downloads'],
  ['what is in my Documents folder?', 'explain', 'any', 'role:documents'],
  ['biggest folders in Projekty', 'largest', 'any', 'name:Projekty'],
  ['tell me something about this disk', 'general', 'any', 'none'],
  ['what is the capital of France?', 'offtopic', 'any', 'none'],
  ['which directory is hogging the most storage?', 'largest', 'any', 'none'],
  ['is it okay to wipe the docker containers?', 'cleanable', 'any', 'rule:docker'],
  ['biggest folders in /var', 'largest', 'any', 'path:/var'],
  ['czy mogę skasować pliki w ~/Pobrane?', 'cleanable', 'any', 'path:~/Pobrane'],
  ['why does ~/.local take so much space?', 'explain', 'any', 'path:~/.local'],
  ['where is my steam library?', 'find', 'any', 'rule:steam'],
  ['how large are my photos?', 'largest', 'media', 'none'],
  ['gdzie są moje foldery node_modules?', 'find', 'any', 'rule:node'],
  ['what caches can I remove?', 'cleanable', 'cache', 'none'],
  ['how much space would I free by emptying the trash', 'cleanable', 'any', 'rule:trash'],
  ['show me what the model labelled in ~/Projekty', 'labelled', 'any', 'path:~/Projekty'],
  ['what is inside ~/Pobrane?', 'explain', 'any', 'path:~/Pobrane'],
  ['dlaczego folder docker jest taki duży?', 'explain', 'any', 'rule:docker'],
  ['what takes space inside the Faugus folder', 'largest', 'any', 'name:Faugus'],
  ['can you tell me a joke', 'offtopic', 'any', 'none'],
  ['czy bezpiecznie usunąć cache cargo?', 'cleanable', 'any', 'rule:cargo'],
  ['anything big in my Downloads?', 'largest', 'any', 'role:downloads'],
  ["where's my pycache", 'find', 'any', 'rule:python'],
  ['whats the biggest cache', 'largest', 'cache', 'none'],
  ['what should I delete first?', 'cleanable', 'any', 'none'],
];

describe('the reading of a question: the words first', () => {
  it('reads every question of the evaluation set that names no folder by a bare name, from the words alone', () => {
    const misses = EVALUATION.filter(([, , , subject]) => !subject.startsWith('name:')).flatMap(
      ([question, intent, kind, subject]) => {
        const reading = readQuestion(question, null);
        const got = [reading.query.intent, reading.query.kind, symbolOf(reading)];

        return got.join(' / ') === [intent, kind, subject].join(' / ')
          ? []
          : [`${question}: expected ${intent} / ${kind} / ${subject}, read ${got.join(' / ')}`];
      },
    );

    expect(misses).toEqual([]);
  });

  it('reads a folder named by a bare name when the keyword reader stands in for the model', () => {
    const names = new Set(['projekty', 'faugus', '.cache']);
    const read = (question: string): string => {
      const reading = readQuestion(question, interpret(question, names));

      return `${reading.query.intent} / ${symbolOf(reading)}`;
    };

    expect(read('biggest folders in Projekty')).toBe('largest / name:Projekty');
    expect(read('what takes space inside the Faugus folder')).toBe('largest / name:Faugus');
    expect(read('dlaczego .cache jest taki duży?')).toBe('explain / name:.cache');
  });

  it('settles the intent and the subject by the words alone, so that the model is not needed', () => {
    const complete = (question: string): boolean => readQuestion(question, null).complete;

    expect(complete('can i safely delete the docker images')).toBe(true);
    expect(complete('why is ~/.cache so big?')).toBe(true);
    expect(complete('can I delete my Downloads?')).toBe(true);
    expect(complete('write me a poem')).toBe(true);

    // No subject, or a bare name: the model is asked.
    expect(complete('what takes the most space?')).toBe(false);
    expect(complete('biggest folders in Projekty')).toBe(false);
    // A subject and no intent word: the model is asked for the intent.
    expect(complete('tell me something about ~/Projekty')).toBe(false);
  });

  it('puts the words before a model that reads the wrong intent, scope, kind and limit', () => {
    const wrong = { intent: 'cleanable', scope: 'obrazy', kind: 'media', limit: 9 } as const;
    const tool = readQuestion('czy mogę usunąć obrazy dockera?', wrong);

    expect(symbolOf(tool)).toBe('rule:docker');
    expect(tool.query).toMatchObject({ intent: 'cleanable', kind: 'any', limit: 9 });

    const misread = readQuestion('which folder does take the biggest amount of data?', {
      intent: 'explain',
      scope: '~/Videos',
      kind: 'cache',
      limit: 9,
    });

    expect(misread.query).toMatchObject({ intent: 'largest', scope: '', kind: 'any', limit: 9 });
    expect(symbolOf(misread)).toBe('none');
  });

  it('calls a question with no word about disks off topic, whatever the model read', () => {
    for (const model of [
      { intent: 'general', scope: '', kind: 'any', limit: 5 },
      { intent: 'largest', scope: '~/Projekty', kind: 'any', limit: 5 },
      { intent: 'cleanable', scope: '', kind: 'cache', limit: 5 },
    ] as const) {
      expect(readQuestion('write me a poem', model).query.intent).toBe('offtopic');
    }
  });

  it('keeps a typed path from being a word about caches, and a question that names a folder from being media', () => {
    expect(signals('biggest in /var/cache').kind).toBeNull();
    expect(signals('what are my biggest caches?').kind).toBe('cache');
    expect(signals('how much do my videos take?').media).toBe('video');
    expect(signals('how much does my Videos folder take?').media).toBeNull();
    expect(signals('can I delete my videos?').roles).toEqual(['Videos']);
    expect(signals('biggest things in ~/Videos').media).toBeNull();
  });

  it('folds the Polish letters, repairs a typo of a few words and never repairs a short word', () => {
    expect(fold('Usunąć ŁÓDŹ')).toBe('usunac lodz');
    expect(wordsOfQuestion('czy mogę usunąć?')).toEqual(['czy', 'moge', 'usunac']);
    expect(wordsOfQuestion('dokcer')).toEqual(['docker']);
    expect(wordsOfQuestion('biggset foldres')).toEqual(['biggest', 'foldres']);
    expect(wordsOfQuestion('steak')).toEqual(['steak']);
    expect(roleOfName('Pobrane')).toBe('Downloads');
    expect(roleOfName('Dokumenty')).toBe('Documents');
    expect(roleOfName('Obrazy')).toBe('Pictures');
    expect(roleOfName('Projekty')).toBeNull();
  });

  it('reads a number of rows next to top, biggest or folders, 1 to 10', () => {
    expect(signals('show me the 3 biggest folders').limit).toBe(3);
    expect(signals('top 7 caches').limit).toBe(7);
    expect(signals('node 18 build folders').limit).toBeNull();
    expect(signals('biggest folders').limit).toBeNull();
    expect(signals('top 99 folders').limit).toBeNull();
  });

  it('takes a quoted text as a path only when it is `~`, starts with `~/`, or is `/` and more', () => {
    const subject = (question: string) => readQuestion(question, null).subject;

    expect(subject('how big is "~/My Files/a b.txt"?')).toEqual({ kind: 'path', text: '~/My Files/a b.txt' });
    expect(subject('how big is " ~/My Files "')).toEqual({ kind: 'path', text: '~/My Files' });
    expect(subject('how big is "/My Files/a"')).toEqual({ kind: 'path', text: '/My Files/a' });
    expect(subject('how big is "~foo" in ~/Pobrane')).toEqual({ kind: 'path', text: '~/Pobrane' });
    expect(subject('how big is "notes" and "~/a b"')).toEqual({ kind: 'path', text: '~/a b' });
  });

  it('opens a quote only at the start or after white space, so a stray quote does not pair with a later one', () => {
    const subject = (question: string) => readQuestion(question, null).subject;

    expect(subject('it\'s 5" wide, how big is "~/x/report."')).toEqual({ kind: 'path', text: '~/x/report.' });
    expect(subject('is say" big "~/x/foo)"')).toEqual({ kind: 'path', text: '~/x/foo)' });
    expect(subject('"~/a b" is big')).toEqual({ kind: 'path', text: '~/a b' });
    expect(quotedPath('size of 5"~/a b"')).toBeNull();
    expect(quotedPath('size of "~/a b')).toBeNull();
  });

  it('does not read a word with a slash in it as the name of a file', () => {
    expect(signals('what about docs/readme.txt').aboutDisk).toBe(false);
    expect(signals('what about readme.txt').aboutDisk).toBe(true);
  });
});
