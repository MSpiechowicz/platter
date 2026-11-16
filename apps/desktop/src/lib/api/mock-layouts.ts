import type { Category } from './types';

/** A group of generated files. Sizes are drawn from a log-normal distribution and scaled to the bytes of the directory. */
export interface FileGroup {
  count: number;
  /** One category, or a list to pick from per file. */
  category: Category | Category[];
  /** Share of the directory's bytes when the directory also has sub-directories. */
  weight?: number;
  /** Log-normal spread of file sizes: 0.5 is even, 1.6 is a few giants and many small files. */
  sigma?: number;
  /** Mean age of the files in days. */
  ageDays?: number;
  /** Split the files into sub-directories of `size` files each. */
  bucket?: { size: number; name: (index: number) => string };
}

export interface DirSpec {
  name: string;
  /** Share among siblings. Only the ratios matter. */
  weight: number;
  kids?: DirSpec[];
  files?: FileGroup;
  /** Another filesystem is mounted here: an empty `mount` node when "stay on one filesystem" is on. */
  mount?: boolean;
  /** A directory the scan could not list: it arrives empty, with no size, and marked `unreadable`. */
  unreadable?: boolean;
  /** A directory with nothing in it at all (not even its own block counted). */
  empty?: boolean;
}

const dir = (name: string, weight: number, kids: DirSpec[]): DirSpec => ({ name, weight, kids });

const unreadable = (name: string): DirSpec => ({ name, weight: 0, unreadable: true });

const empty = (name: string): DirSpec => ({ name, weight: 0, empty: true });

const leaf = (name: string, weight: number, files: FileGroup): DirSpec => ({ name, weight, files });

const numbered =
  (prefix: string) =>
  (i: number): string =>
    `${prefix}${String(i + 1).padStart(2, '0')}`;

/** A typical software project: sources, a build directory and installed dependencies. */
function project(
  name: string,
  weight: number,
  parts: { src: number; target?: number; modules?: number },
): DirSpec {
  const kids: DirSpec[] = [
    leaf('src', parts.src, {
      count: 70,
      category: 'code',
      sigma: 0.9,
      ageDays: 60,
      bucket: { size: 24, name: numbered('mod') },
    }),
  ];

  if (parts.target) {
    kids.push(
      dir('target', parts.target, [
        leaf('debug', 0.7, {
          count: 110,
          category: 'build',
          sigma: 1.4,
          ageDays: 30,
          bucket: { size: 40, name: numbered('deps-') },
        }),
        leaf('release', 0.3, { count: 40, category: 'build', sigma: 1.4, ageDays: 40 }),
      ]),
    );
  }

  if (parts.modules) {
    kids.push(
      leaf('node_modules', parts.modules, {
        count: 150,
        category: 'package',
        sigma: 1.3,
        ageDays: 90,
        bucket: { size: 30, name: numbered('pkg-') },
      }),
    );
  }

  return dir(name, weight, kids);
}

/** Contents of a home directory, weighted by typical GB. */
function homeKids(): DirSpec[] {
  return [
    dir('Projekty', 214, [
      project('platter', 100, { src: 4, target: 70, modules: 26 }),
      project('rust-playground', 62, { src: 3, target: 59 }),
      project('web-shop', 52, { src: 5, modules: 47 }),
    ]),
    dir('.local', 190, [
      dir('share', 190, [
        dir('Steam', 142, [
          leaf('steamapps', 120, {
            count: 90,
            category: 'app',
            sigma: 1.8,
            ageDays: 120,
            bucket: { size: 30, name: numbered('game-') },
          }),
          leaf('shadercache', 22, { count: 60, category: 'other', sigma: 1.1, ageDays: 40 }),
        ]),
        leaf('containers', 31, { count: 45, category: ['diskImage', 'other'], sigma: 1.5, ageDays: 150 }),
        leaf('Trash', 17, {
          count: 40,
          category: ['document', 'image', 'archive'],
          sigma: 1.4,
          ageDays: 210,
        }),
      ]),
    ]),
    dir('Videos', 133, [
      leaf('recordings', 92, { count: 38, category: 'video', sigma: 0.9, ageDays: 240 }),
      leaf('films', 41, { count: 14, category: 'video', sigma: 0.5, ageDays: 700 }),
    ]),
    leaf('Downloads', 96, {
      count: 120,
      category: ['archive', 'document', 'diskImage', 'video', 'package'],
      sigma: 1.6,
      ageDays: 420,
    }),
    leaf('Pictures', 58, {
      count: 360,
      category: 'image',
      sigma: 0.7,
      ageDays: 900,
      bucket: { size: 60, name: (i) => `20${String(19 + i).padStart(2, '0')}` },
    }),
    leaf('Music', 21, {
      count: 160,
      category: 'audio',
      sigma: 0.6,
      ageDays: 1200,
      bucket: { size: 20, name: numbered('album-') },
    }),
    dir('.cache', 47, [
      leaf('google-chrome', 14, { count: 140, category: 'other', sigma: 1.2, ageDays: 14 }),
      leaf('pip', 12, { count: 70, category: 'archive', sigma: 1.1, ageDays: 70 }),
      leaf('thumbnails', 6, { count: 120, category: 'image', sigma: 0.5, ageDays: 40 }),
      leaf('other', 15, { count: 60, category: 'other', sigma: 1.3, ageDays: 60 }),
    ]),
    leaf('.var', 39, { count: 50, category: ['app', 'other'], sigma: 1.4, ageDays: 100 }),
    leaf('Documents', 12, {
      count: 120,
      category: ['document', 'archive'],
      sigma: 1.3,
      ageDays: 500,
      bucket: { size: 40, name: numbered('folder-') },
    }),
    leaf('.config', 3, { count: 120, category: ['code', 'other'], sigma: 1.2, ageDays: 300 }),
  ];
}

/** A whole root filesystem: the home directory plus system directories. */
function systemKids(): DirSpec[] {
  return [
    dir('home', 812, [dir('demo', 812, homeKids())]),
    dir('.snapshots', 310, [
      leaf('older than 30 days', 268, {
        count: 60,
        category: 'archive',
        sigma: 1.2,
        ageDays: 400,
        bucket: { size: 20, name: (i) => String(412 + i) },
      }),
      leaf('recent', 42, { count: 24, category: 'archive', sigma: 1.1, ageDays: 12 }),
    ]),
    dir('var', 164, [
      dir('lib', 118, [
        leaf('docker', 96, {
          count: 80,
          category: 'diskImage',
          sigma: 1.5,
          ageDays: 90,
          bucket: { size: 20, name: numbered('overlay-') },
        }),
        leaf('flatpak', 22, { count: 50, category: 'app', sigma: 1.4, ageDays: 60 }),
      ]),
      dir('cache', 38, [
        dir('pacman', 38, [leaf('pkg', 38, { count: 140, category: 'package', sigma: 1.2, ageDays: 150 })]),
      ]),
      dir('log', 4.1, [leaf('journal', 4.1, { count: 24, category: 'other', sigma: 0.4, ageDays: 20 })]),
      leaf('tmp', 3.9, { count: 30, category: 'other', sigma: 1.4, ageDays: 5 }),
    ]),
    dir('usr', 41, [
      leaf('lib', 28, { count: 160, category: 'app', sigma: 1.5, ageDays: 80 }),
      leaf('share', 13, { count: 110, category: ['document', 'image', 'other'], sigma: 1.3, ageDays: 80 }),
    ]),
    leaf('swap', 34, { count: 1, category: 'other', sigma: 0.1, ageDays: 3 }),
    leaf('opt', 18, { count: 40, category: 'app', sigma: 1.3, ageDays: 200 }),
    unreadable('root'),
    empty('media'),
    leaf('srv', 29, { count: 40, category: ['archive', 'document'], sigma: 1.3, ageDays: 300 }),
    {
      name: 'mnt',
      weight: 60,
      mount: true,
      kids: [leaf('backup', 60, { count: 30, category: 'archive', sigma: 1.0 })],
    },
  ];
}

/** A backup or archive disk. */
function archiveKids(): DirSpec[] {
  return [
    leaf('borg', 640, { count: 70, category: 'archive', sigma: 1.1, ageDays: 300 }),
    leaf('photos', 210, {
      count: 300,
      category: 'image',
      sigma: 0.7,
      ageDays: 1500,
      bucket: { size: 50, name: (i) => `20${String(12 + i).padStart(2, '0')}` },
    }),
    leaf('documents', 40, { count: 100, category: ['document', 'archive'], sigma: 1.2, ageDays: 900 }),
  ];
}

/** The scan root of the skewed demo disk (`?root=/skew`), and the bytes it holds. */
export const SKEW_ROOT = '/skew';
export const SKEW_BYTES = 1600 * 1_000_000_000;

/**
 * A real, heavily skewed disk: one folder holds 97% of it, three more hold 1.3%, 1.3% and 0.2%, then a 17 MB one
 * and fourteen items that add up to a few KB. Weights are GB.
 */
function skewKids(): DirSpec[] {
  const tail = (name: string, weight: number): DirSpec =>
    leaf(name, weight, { count: 3, category: 'other', sigma: 0.8, ageDays: 200 });
  const tiny = Array.from({ length: 14 }, (_, index) =>
    leaf(`stub-${index + 1}`, 0.000001, { count: 1, category: 'other', sigma: 0.1, ageDays: 400 }),
  );

  return [
    dir('home', 1550, [
      leaf('vm-images', 1060, { count: 18, category: 'diskImage', sigma: 1.1, ageDays: 150 }),
      leaf('Videos', 136, { count: 60, category: 'video', sigma: 1.0, ageDays: 300 }),
      leaf('Games', 101, {
        count: 120,
        category: 'app',
        sigma: 1.6,
        ageDays: 100,
        bucket: { size: 30, name: numbered('game-') },
      }),
      dir('Projekty', 96, [
        project('platter', 60, { src: 3, target: 50, modules: 7 }),
        project('web-shop', 36, { src: 4, modules: 32 }),
      ]),
      leaf('Downloads', 14, { count: 80, category: ['archive', 'document'], sigma: 1.5, ageDays: 400 }),
      leaf('Pictures', 12, { count: 200, category: 'image', sigma: 0.7, ageDays: 900 }),
      leaf('Music', 7, { count: 90, category: 'audio', sigma: 0.6, ageDays: 1200 }),
      leaf('.cache', 6.7, { count: 90, category: 'other', sigma: 1.2, ageDays: 14 }),
      leaf('Documents', 5.7, { count: 80, category: 'document', sigma: 1.3, ageDays: 500 }),
      ...Array.from({ length: 24 }, (_, index) => tail(`misc-${index + 1}`, 0.2)),
    ]),
    dir('var', 21, [
      leaf('lib', 12, { count: 300, category: 'package', sigma: 1.4, ageDays: 120 }),
      leaf('cache', 6, { count: 200, category: 'other', sigma: 1.2, ageDays: 20 }),
      leaf('log', 3, { count: 80, category: 'other', sigma: 1.0, ageDays: 10 }),
    ]),
    dir('usr', 21, [
      leaf('lib', 14, { count: 300, category: 'app', sigma: 1.5, ageDays: 80 }),
      leaf('share', 7, { count: 250, category: ['document', 'image', 'other'], sigma: 1.3, ageDays: 80 }),
    ]),
    leaf('opt', 3.6, { count: 120, category: 'app', sigma: 1.3, ageDays: 200 }),
    leaf('etc', 0.017, { count: 90, category: 'other', sigma: 0.8, ageDays: 100 }),
    ...tiny,
    ...['boot', 'dev', 'proc', 'run', 'sys', 'tmp', 'mnt'].map((name) => ({ name, weight: 0, mount: true })),
    unreadable('root'),
    unreadable('.snapshots'),
    empty('srv'),
  ];
}

/** The scan root of the demo disk that mirrors a real laptop (`?root=/real`), and the bytes it holds. */
export const REAL_ROOT = '/real';
export const REAL_BYTES = 1596 * 1_000_000_000;

/**
 * A real disk, as one user's laptop has it: `/home` holds 97%, all of it in one home directory, whose biggest
 * folders hold chains of single-child folders (`.local/share/Steam/steamapps/common`, `.docker/desktop/vms/0/data`).
 * Beside it are `var`, `usr`, `opt`, a 17 MB `etc`, fourteen tiny items and a row of folders with no size. Weights are GB.
 */
function realKids(): DirSpec[] {
  const files = (count: number, category: Category | Category[], sigma = 1.1): FileGroup => ({
    count,
    category,
    sigma,
    ageDays: 150,
  });
  const game = (name: string, weight: number): DirSpec =>
    leaf(name, weight, { count: 40, category: 'app', sigma: 1.5, ageDays: 90 });
  const tail = (name: string, weight: number): DirSpec => leaf(name, weight, files(3, 'other', 0.8));
  const tiny = Array.from({ length: 14 }, (_, index) =>
    leaf(`stub-${index + 1}`, 0.000001, { count: 1, category: 'other', sigma: 0.1, ageDays: 400 }),
  );
  // Games with long names, some of them chains of single-child folders (`Game / Content / Paks`).
  const packed = (name: string, weight: number): DirSpec =>
    dir(name, weight, [dir('Content', weight, [leaf('Paks', weight, files(12, 'app', 0.9))])]);
  const longNamed = [
    packed('Total War WARHAMMER III', 14),
    packed("Sid Meier's Civilization VI", 10),
    game('Car Mechanic Simulator 2021', 8),
    packed('Pax Dei', 6),
    packed("No Man's Sky BANKS", 5),
  ];
  const projects = [
    ['platter', 28],
    ['web-shop', 14],
    ['rust-playground', 11],
    ['dotfiles', 8],
    ['ocpp-bridge', 7],
    ['game-jam', 6],
    ['blog', 5],
    ['notes', 4],
    ['scripts', 4],
    ['ml-experiments', 4],
    ['homelab', 3],
    ['tauri-test', 2],
    ['cli-tools', 2],
    ['ansible', 1.5],
    ['sandbox', 1.5],
    ['old-site', 1],
  ] as const;

  return [
    dir('home', 1550, [
      dir('sam', 1550, [
        dir('.local', 823, [
          dir('share', 812, [
            dir('Steam', 790, [
              dir('steamapps', 760, [
                dir('common', 700, [
                  game('Cyberpunk 2077', 120),
                  game('Baldurs Gate 3', 150),
                  game('Elden Ring', 60),
                  game('Satisfactory', 45),
                  game('Factorio', 40),
                  game('Hades II', 30),
                  game('Stellaris', 28),
                  game('Dota 2', 60),
                  game('Counter-Strike 2', 40),
                  game('Valheim', 25),
                  game('Terraria', 20),
                  game('Hollow Knight', 10),
                  ...longNamed,
                  ...Array.from({ length: 285 }, (_, index) =>
                    leaf(`small-game-${index + 1}`, 29 / 285, files(4, 'app', 0.8)),
                  ),
                ]),
                leaf('downloading', 40, files(30, 'app')),
                leaf('shadercache', 20, files(50, 'other')),
              ]),
              leaf('shadercache', 20, files(40, 'other')),
              leaf('compatibilitytools.d', 10, files(20, 'app')),
            ]),
            leaf('applications', 12, files(40, 'app')),
            leaf('icons', 10, files(120, 'image')),
          ]),
          leaf('bin', 6, files(20, 'app')),
          leaf('state', 5, files(30, 'other')),
        ]),
        dir('.docker', 336, [
          dir('desktop', 330, [
            dir('vms', 328, [
              dir('0', 326, [
                leaf('data', 325, { count: 9, category: 'diskImage', sigma: 0.08, ageDays: 40 }),
                leaf('logs', 1, files(10, 'other')),
              ]),
              leaf('1-old', 2, files(4, 'diskImage')),
            ]),
            leaf('cache', 2, files(30, 'other')),
          ]),
          leaf('buildx', 3, files(20, 'other')),
          leaf('mcp', 3, files(20, 'other')),
        ]),
        dir('Faugus', 136, [
          leaf('prefixes', 112, files(60, 'app', 1.4)),
          leaf('runners', 18, files(40, 'app')),
          leaf('shared', 6, files(20, 'other')),
        ]),
        dir(
          'Projekty',
          102,
          projects.map(([name, weight]) =>
            project(
              name,
              weight,
              name === 'platter' ? { src: 3, target: 20, modules: 5 } : { src: 2, target: 1 },
            ),
          ),
        ),
        dir('.cache', 97, [
          leaf('google-chrome', 30, files(140, 'other', 1.2)),
          leaf('yay', 24, files(70, 'package')),
          leaf('mesa_shader_cache', 14, files(120, 'other')),
          leaf('pip', 12, files(70, 'archive')),
          leaf('thumbnails', 9, files(120, 'image', 0.5)),
          leaf('other', 8, files(60, 'other')),
        ]),
        leaf('.config', 14, files(200, ['code', 'other'], 1.2)),
        leaf('Games', 12, files(60, 'app', 1.5)),
        leaf('.codex', 7.1, files(80, 'other')),
        leaf('.npm', 6.7, files(150, 'package')),
        leaf('.bun', 5.7, files(90, 'package')),
        ...Array.from({ length: 30 }, (_, index) => tail(`misc-${index + 1}`, 0.35)),
      ]),
    ]),
    dir('var', 21, [
      leaf('lib', 12, files(300, 'package', 1.4)),
      leaf('cache', 6, files(200, 'other', 1.2)),
      leaf('log', 3, files(80, 'other', 1)),
    ]),
    dir('usr', 21, [leaf('lib', 14, files(300, 'app', 1.5)), leaf('share', 7, files(250, 'other', 1.3))]),
    leaf('opt', 3.6, files(144, 'app', 1.3)),
    leaf('etc', 0.017, files(90, 'other', 0.8)),
    ...tiny,
    ...['boot', 'dev', 'proc', 'run', 'sys', 'tmp', 'mnt'].map((name) => ({ name, weight: 0, mount: true })),
    unreadable('root'),
    unreadable('.snapshots'),
    empty('srv'),
  ];
}

/**
 * A disk like a single-user laptop: 95% in `/home`, and 90% of that in one home directory. The home directory is
 * the usual demo home, so the Overview has a chain of folders to break down.
 */
function dominantLayout(): DirSpec[] {
  return [
    dir('home', 95, [
      dir('sam', 90, homeKids()),
      leaf('shared', 10, { count: 40, category: ['archive', 'video'], sigma: 1.2, ageDays: 300 }),
    ]),
    leaf('var', 3, { count: 60, category: ['package', 'other'], sigma: 1.2, ageDays: 40 }),
    leaf('opt', 2, { count: 30, category: 'app', sigma: 1.3, ageDays: 200 }),
  ];
}

/** Picks what the demo disk looks like for a given scan root. `dominant` is the lopsided layout of `?dominant`. */
export function layoutFor(root: string, dominant = false): DirSpec[] {
  if (dominant) {
    return dominantLayout();
  }

  if (root === SKEW_ROOT) {
    return skewKids();
  }

  if (root === REAL_ROOT) {
    return realKids();
  }

  if (root === '/') {
    return systemKids();
  }

  if (root.startsWith('/home')) {
    return homeKids();
  }

  return archiveKids();
}
