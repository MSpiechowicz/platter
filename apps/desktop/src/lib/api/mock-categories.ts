import type { Category } from './types';

// The mock's copy of the backend's content categories: `crates/platter-core/src/model/category.rs` and its tables in
// `category_table.rs`. `mock-categories.test.ts` reads the Rust tables and compares them with these, so the two cannot
// drift. Extensions are lowercase without the dot, and directory names match exactly.

/** In the backend's index order, which breaks ties between categories of equal size. */
export const CATEGORIES: readonly Category[] = [
  'video',
  'image',
  'audio',
  'document',
  'archive',
  'code',
  'build',
  'package',
  'app',
  'diskImage',
  'data',
  'other',
];

type Table = readonly (readonly [Category, readonly string[]])[];

/** An extension that means two things, told apart by the file's apparent size. */
export interface SizeRule {
  extension: string;
  /** Files strictly larger than this many bytes are `large`. */
  above: number;
  small: Category;
  large: Category;
}

const MIB = 1024 * 1024;

export const SIZE_RULES: readonly SizeRule[] = [
  { extension: 'raw', above: 256 * MIB, small: 'image', large: 'diskImage' },
];

/** File extension to category. */
export const EXTENSIONS: Table = [
  [
    'video',
    ['mp4', 'mkv', 'avi', 'mov', 'wmv', 'flv', 'webm', 'm4v', 'mpg', 'mpeg', 'm2ts', 'vob', '3gp', 'ogv'],
  ],
  [
    'image',
    [
      'jpg',
      'jpeg',
      'png',
      'gif',
      'bmp',
      'webp',
      'svg',
      'tif',
      'tiff',
      'heic',
      'heif',
      'avif',
      'cr2',
      'nef',
      'arw',
      'dng',
      'psd',
      'xcf',
      'ico',
    ],
  ],
  ['audio', ['mp3', 'flac', 'wav', 'ogg', 'oga', 'opus', 'm4a', 'aac', 'wma', 'aiff', 'mid', 'midi', 'ape']],
  [
    'document',
    [
      'pdf',
      'doc',
      'docx',
      'odt',
      'rtf',
      'txt',
      'md',
      'xls',
      'xlsx',
      'ods',
      'ppt',
      'pptx',
      'odp',
      'epub',
      'mobi',
      'tex',
      'log',
    ],
  ],
  [
    'archive',
    ['zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', 'zst', '7z', 'rar', 'lz4', 'lzma', 'cab', 'cpio', 'br'],
  ],
  [
    'code',
    [
      'rs',
      'py',
      'js',
      'mjs',
      'cjs',
      'jsx',
      'tsx',
      'svelte',
      'vue',
      'c',
      'h',
      'cc',
      'cpp',
      'hpp',
      'cs',
      'java',
      'kt',
      'go',
      'rb',
      'php',
      'swift',
      'lua',
      'sh',
      'bash',
      'zsh',
      'fish',
      'html',
      'css',
      'scss',
      'json',
      'yaml',
      'yml',
      'toml',
      'xml',
      'sql',
      'ts',
      'zig',
      'hs',
      'ex',
      'exs',
      'dart',
      'gd',
    ],
  ],
  [
    'build',
    ['o', 'obj', 'a', 'lib', 'rlib', 'rmeta', 'pdb', 'ilk', 'class', 'pyc', 'pyo', 'd', 'wasm', 'map'],
  ],
  ['package', ['deb', 'rpm', 'apk', 'flatpak', 'snap', 'whl', 'egg', 'gem', 'crate', 'jar', 'nupkg']],
  [
    'app',
    [
      'exe',
      'msi',
      'pkg',
      'run',
      'dll',
      'so',
      'dylib',
      'node',
      'appimage',
      'ucas',
      'utoc',
      'pak',
      'ress',
      'bundle',
      'unity3d',
      'assets',
      'assetbundle',
      'resource',
      'wem',
      'bnk',
      'bank',
      'bk2',
      'vpk',
      'gcf',
      'forge',
      'arc',
      'rpf',
      'big',
      'wad',
      'pck',
      'uasset',
      'umap',
      'upk',
      'cook',
      'foz',
      'pack',
    ],
  ],
  ['diskImage', ['iso', 'img', 'dmg', 'vmdk', 'vdi', 'vhd', 'vhdx', 'qcow2', 'qcow', 'wim', 'squashfs']],
  [
    'data',
    [
      'sqlite',
      'sqlite3',
      'db',
      'mdb',
      'jsonl',
      'parquet',
      'csv',
      'npy',
      'npz',
      'h5',
      'hdf5',
      'safetensors',
      'gguf',
      'ckpt',
      'pt',
      'pth',
      'onnx',
      'tflite',
    ],
  ],
];

/** Multi-part suffixes that beat the last-extension lookup, so `foo.pkg.tar.zst` is a package and not a `.zst` archive. */
export const COMPOUND_EXTENSIONS: Table = [
  ['package', ['pkg.tar.zst', 'pkg.tar.xz', 'pkg.tar.gz', 'pkg.tar']],
];

/** Location rules that always apply: the directory imposes its category on everything below it. A pattern is `name`, `prefix*`, `*suffix` or `parent/name`. */
export const DIRECTORY_NAMES: Table = [
  ['code', ['.git']],
  [
    'app',
    [
      'steamapps',
      'Steam',
      '.steam',
      'compatdata',
      'shadercache',
      'Proton*',
      '.wine',
      'Faugus*',
      'lutris',
      'heroic',
      'Games',
    ],
  ],
  [
    'package',
    [
      '.cache',
      'Cache',
      'CachedData',
      'GPUCache',
      'Code Cache',
      '__pycache__',
      '.npm',
      '.bun',
      '.cargo/registry',
      '.rustup',
      '.gradle',
      '.m2',
      '.nuget',
      'go/pkg',
      '.pnpm-store',
      '.yarn',
      'site-packages',
      'node_modules',
      '.venv',
      'venv',
      'bower_components',
    ],
  ],
  [
    'build',
    [
      '.nuxt',
      '.svelte-kit',
      '.parcel-cache',
      '.tox',
      '.pytest_cache',
      '.mypy_cache',
      '.ruff_cache',
      '.angular',
      '*.dSYM',
    ],
  ],
];

/** Location rules for names that are common outside of software projects: they apply only next to a project marker. */
export const PROJECT_DIRECTORY_NAMES: Table = [
  ['build', ['target', 'build', 'dist', 'out', '.next', '.turbo', 'cmake-build-*']],
];

/** Names whose presence in a directory's listing makes the directory a software project. */
export const PROJECT_MARKERS: readonly string[] = [
  '.git',
  'package.json',
  'Cargo.toml',
  'CMakeLists.txt',
  'pyproject.toml',
  'pom.xml',
  'build.gradle',
  'build.gradle.kts',
  'go.mod',
  'Makefile',
  'meson.build',
];

/** Longer extensions are never looked up. */
const MAX_EXTENSION = 12;

/** Lowercases the letters A to Z only, as the backend does: `toLowerCase` would also change letters like `İ`. */
const asciiLower = (text: string): string => text.replace(/[A-Z]/g, (letter) => letter.toLowerCase());

/** The part after the last dot, unless the dot is the first or last character or the part is too long. */
function extensionOf(name: string): string | null {
  const dot = name.lastIndexOf('.');
  const extension = name.slice(dot + 1);

  if (dot <= 0 || extension === '' || extension.length > MAX_EXTENSION) {
    return null;
  }

  return extension;
}

const BY_EXTENSION: ReadonlyMap<string, Category> = new Map(
  EXTENSIONS.flatMap(([category, extensions]) =>
    extensions.map((extension): [string, Category] => [extension, category]),
  ),
);

const BY_SIZE: ReadonlyMap<string, SizeRule> = new Map(SIZE_RULES.map((rule) => [rule.extension, rule]));

/** Whether `name` is some stem, a dot, and then `suffix` (the stem must not be empty), ignoring case. */
function hasSuffix(name: string, suffix: string): boolean {
  const dot = name.length - suffix.length - 1;

  return dot > 0 && name[dot] === '.' && asciiLower(name.slice(dot + 1)) === suffix;
}

/**
 * Category of a file with the given apparent `size`, judged by its name's extension. Names without a usable extension
 * are `other`. The size only matters for the extensions in `SIZE_RULES`.
 */
export function ofFile(name: string, size: number): Category {
  for (const [category, suffixes] of COMPOUND_EXTENSIONS) {
    if (suffixes.some((suffix) => hasSuffix(name, suffix))) {
      return category;
    }
  }

  const extension = extensionOf(name);

  if (extension === null) {
    return 'other';
  }

  const key = asciiLower(extension);
  const rule = BY_SIZE.get(key);

  if (rule) {
    return size > rule.above ? rule.large : rule.small;
  }

  return BY_EXTENSION.get(key) ?? 'other';
}

type Wild = { kind: 'prefix' | 'suffix'; text: string };

interface DirectoryRule {
  category: Category;
  /** Name the parent directory must have. */
  parent: string | null;
  /** Applies only to project-like directories. */
  projectOnly: boolean;
  /** Set for a `prefix*` or `*suffix` name. */
  wild: Wild | null;
}

/** The rules in table order: the always-on ones first, then the project ones. Of the rules that match, the first wins. */
function directoryRules(): { exact: Map<string, DirectoryRule[]>; wild: DirectoryRule[] } {
  const exact = new Map<string, DirectoryRule[]>();
  const wild: DirectoryRule[] = [];

  const tables: [Table, boolean][] = [
    [DIRECTORY_NAMES, false],
    [PROJECT_DIRECTORY_NAMES, true],
  ];

  for (const [table, projectOnly] of tables) {
    for (const [category, patterns] of table) {
      for (const pattern of patterns) {
        const slash = pattern.indexOf('/');
        const parent = slash < 0 ? null : pattern.slice(0, slash);
        const name = slash < 0 ? pattern : pattern.slice(slash + 1);
        const rule = { category, parent, projectOnly };

        if (name.endsWith('*')) {
          wild.push({ ...rule, wild: { kind: 'prefix', text: name.slice(0, -1) } });
        } else if (name.startsWith('*')) {
          wild.push({ ...rule, wild: { kind: 'suffix', text: name.slice(1) } });
        } else {
          exact.set(name, [...(exact.get(name) ?? []), { ...rule, wild: null }]);
        }
      }
    }
  }

  return { exact, wild };
}

const BY_DIRECTORY = directoryRules();

function wildMatches(wild: Wild, name: string): boolean {
  return wild.kind === 'prefix' ? name.startsWith(wild.text) : name.endsWith(wild.text);
}

/**
 * Category the directory `name` (inside a directory called `parent`, if known) imposes on its whole subtree, if it
 * matches a location rule. `projectLike` says whether the directory sits next to a project marker. It is called at
 * most once, and only when a rule that needs the answer matches by name.
 */
export function ofDirectory(
  parent: string | null,
  name: string,
  projectLike: () => boolean,
): Category | null {
  let project: boolean | undefined;

  const applies = (rule: DirectoryRule): boolean => {
    const projectMatches = !rule.projectOnly || (project ??= projectLike());

    return (rule.parent === null || rule.parent === parent) && projectMatches;
  };

  const wild = BY_DIRECTORY.wild.filter((rule) => rule.wild && wildMatches(rule.wild, name));
  const found = [...(BY_DIRECTORY.exact.get(name) ?? []), ...wild].find(applies);

  return found?.category ?? null;
}

/** Whether `name` marks the directory that contains it as a software project. */
export function isProjectMarker(name: string): boolean {
  return PROJECT_MARKERS.includes(name);
}
