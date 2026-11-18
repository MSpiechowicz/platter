import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  CATEGORIES,
  COMPOUND_EXTENSIONS,
  DIRECTORY_NAMES,
  EXTENSIONS,
  PROJECT_DIRECTORY_NAMES,
  PROJECT_MARKERS,
  SIZE_RULES,
  isProjectMarker,
  ofDirectory,
  ofFile,
} from './mock-categories';
import type { Category } from './types';

const MIB = 1024 * 1024;

// The tables of the backend, read from its source: the mock's copy must say the same.
const rust = readFileSync(
  resolve(process.cwd(), '../../crates/platter-core/src/model/category_table.rs'),
  'utf8',
)
  .split('\n')
  .map((line) => line.replace(/\/\/.*$/, ''))
  .join('\n');

/** The body of `pub const NAME ... = &[ ... ];`. */
function bodyOf(name: string): string {
  const start = rust.indexOf(`pub const ${name}:`);

  expect(start, `${name} is in category_table.rs`).toBeGreaterThanOrEqual(0);

  return rust.slice(start, rust.indexOf('];', start));
}

const strings = (text: string): string[] =>
  [...text.matchAll(/"([^"]*)"/g)].map((match) => match[1] as string);

/** `Video` is `video` and `DiskImage` is `diskImage`, the wire names. */
const wireName = (name: string): Category => (name.charAt(0).toLowerCase() + name.slice(1)) as Category;

/** A table of `(Category::X, &["a", "b"])` rows. */
function rowsOf(name: string): [Category, string[]][] {
  return [...bodyOf(name).matchAll(/\(\s*Category::(\w+)\s*,\s*&\[([^\]]*)\]\s*,?\s*\)/g)].map((row) => [
    wireName(row[1] as string),
    strings(row[2] as string),
  ]);
}

const asRows = (table: readonly (readonly [Category, readonly string[]])[]): [Category, string[]][] =>
  table.map(([category, names]) => [category, [...names]]);

describe('the mock category tables', () => {
  it.each([
    ['EXTENSIONS', EXTENSIONS],
    ['COMPOUND_EXTENSIONS', COMPOUND_EXTENSIONS],
    ['DIRECTORY_NAMES', DIRECTORY_NAMES],
    ['PROJECT_DIRECTORY_NAMES', PROJECT_DIRECTORY_NAMES],
  ])('%s equal the backend table, row by row', (name, table) => {
    const expected = rowsOf(name);

    expect(expected.length).toBeGreaterThan(0);
    expect(asRows(table)).toEqual(expected);
  });

  it('PROJECT_MARKERS equal the backend list', () => {
    const expected = strings(bodyOf('PROJECT_MARKERS'));

    expect(expected.length).toBeGreaterThan(0);
    expect([...PROJECT_MARKERS]).toEqual(expected);
  });

  it('SIZE_RULES equal the backend rules', () => {
    const unit = /const MIB: u64 = (\d+) \* (\d+);/.exec(rust);
    const mib = Number(unit?.[1]) * Number(unit?.[2]);
    const rules = [
      ...bodyOf('SIZE_RULES').matchAll(
        /extension:\s*"(\w+)",\s*above:\s*(\d+)\s*\*\s*MIB,\s*small:\s*Category::(\w+),\s*large:\s*Category::(\w+)/g,
      ),
    ].map((rule) => ({
      extension: rule[1],
      above: Number(rule[2]) * mib,
      small: wireName(rule[3] as string),
      large: wireName(rule[4] as string),
    }));

    expect(mib).toBe(MIB);
    expect(rules.length).toBeGreaterThan(0);
    expect([...SIZE_RULES]).toEqual(rules);
  });

  it('lists the categories in the backend order', () => {
    const declared = /pub enum Category \{([^}]*)\}/.exec(
      readFileSync(resolve(process.cwd(), '../../crates/platter-core/src/model/category.rs'), 'utf8').replace(
        /\/\/.*$/gm,
        '',
      ),
    );
    const names = [...(declared?.[1] ?? '').matchAll(/^\s*(\w+),\s*$/gm)].map((match) =>
      wireName(match[1] as string),
    );

    expect([...CATEGORIES]).toEqual(names);
  });

  it('gives an extension to one category only', () => {
    const all = [
      ...EXTENSIONS.flatMap(([, extensions]) => extensions),
      ...SIZE_RULES.map((rule) => rule.extension),
    ];

    expect(new Set(all).size).toBe(all.length);
  });
});

describe('ofFile', () => {
  it('classifies by extension, ignoring case', () => {
    expect(ofFile('movie.MKV', 1000)).toBe('video');
    expect(ofFile('photo.jpeg', 1000)).toBe('image');
    expect(ofFile('backup.tar.zst', 1000)).toBe('archive');
    expect(ofFile('main.rs', 1000)).toBe('code');
    expect(ofFile('tool.AppImage', 1000)).toBe('app');
    expect(ofFile('sharedassets0.resS', 1000)).toBe('app');
  });

  it('calls a name without a usable extension other', () => {
    for (const name of [
      'Makefile',
      '.bashrc',
      'trailing.',
      'weird.unknownext',
      'data.bin',
      'a.verylongextension',
    ]) {
      expect(ofFile(name, 1000), name).toBe('other');
    }
  });

  it('lowercases ASCII letters only', () => {
    expect(ofFile('x.İSO', 1000)).toBe('other');
    expect(ofFile('x.ISO', 1000)).toBe('diskImage');
  });

  it('calls .raw an image up to 256 MiB and a disk image above', () => {
    expect(ofFile('IMG_0001.RAW', 0)).toBe('image');
    expect(ofFile('IMG_0001.RAW', 256 * MIB)).toBe('image');
    expect(ofFile('IMG_0001.RAW', 256 * MIB + 1)).toBe('diskImage');
  });

  it('prefers a compound extension that has a stem', () => {
    expect(ofFile('linux-6.1-1-x86_64.pkg.tar.zst', 1000)).toBe('package');
    expect(ofFile('tool.PKG.TAR.XZ', 1000)).toBe('package');
    expect(ofFile('pkg.tar.zst', 1000)).toBe('archive');
    expect(ofFile('.pkg.tar.zst', 1000)).toBe('archive');
    expect(ofFile('notpkg.tar.zst', 1000)).toBe('archive');
  });
});

describe('ofDirectory', () => {
  const never = (): boolean => {
    throw new Error('the project probe must not run');
  };

  it('imposes a category for the names that always apply, under any parent', () => {
    expect(ofDirectory(null, '.git', never)).toBe('code');
    expect(ofDirectory('anything', 'node_modules', never)).toBe('package');
    expect(ofDirectory(null, 'Proton-GE-9', never)).toBe('app');
    expect(ofDirectory(null, 'App.dSYM', never)).toBe('build');
    expect(ofDirectory(null, '.pnpm-store', never)).toBe('package');
  });

  it('matches whole names and is case-sensitive', () => {
    for (const name of ['src', 'Documents', 'cache', 'node_module', 'My .git', 'Gamesx', 'Proto', 'dSYM']) {
      expect(
        ofDirectory(null, name, () => true),
        name,
      ).toBeNull();
    }
  });

  it('needs the named parent for a parent/name rule', () => {
    expect(ofDirectory('.cargo', 'registry', never)).toBe('package');
    expect(ofDirectory('go', 'pkg', never)).toBe('package');
    expect(ofDirectory('other', 'registry', never)).toBeNull();
    expect(ofDirectory(null, 'pkg', never)).toBeNull();
    expect(ofDirectory('.cargo', 'bin', never)).toBeNull();
  });

  it('applies the project rules next to a marker only', () => {
    for (const name of ['target', 'build', 'dist', 'out', '.next', '.turbo', 'cmake-build-debug']) {
      expect(
        ofDirectory('proj', name, () => true),
        name,
      ).toBe('build');
      expect(
        ofDirectory('proj', name, () => false),
        `${name} outside a project`,
      ).toBeNull();
    }
  });

  it('asks whether the directory is project-like at most once, and only for a project rule', () => {
    let calls = 0;
    const probe = (): boolean => {
      calls++;

      return true;
    };

    expect(ofDirectory(null, 'src', probe)).toBeNull();
    expect(ofDirectory(null, 'node_modules', probe)).toBe('package');
    expect(calls).toBe(0);
    expect(ofDirectory(null, 'build', probe)).toBe('build');
    expect(calls).toBe(1);
  });
});

describe('isProjectMarker', () => {
  it('recognises exact names', () => {
    for (const name of ['.git', 'package.json', 'Cargo.toml', 'CMakeLists.txt', 'pyproject.toml']) {
      expect(isProjectMarker(name), name).toBe(true);
    }

    for (const name of ['cargo.toml', 'package.json.bak', 'src', '']) {
      expect(isProjectMarker(name), name).toBe(false);
    }
  });
});
