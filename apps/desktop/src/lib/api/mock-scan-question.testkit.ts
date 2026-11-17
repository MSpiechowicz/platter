import { protection } from './mock-advisor';
import { answerQuestion, type ScanQuestionContext } from './mock-scan-question';
import { nodeAt, sketchTree, type Sketch } from './mock-sketch';
import { folderPathOf } from './mock-suggestions';
import type { MockNode } from './mock-tree';
import type { ScanAnswerDto } from './types';

/** The home folder of every sketch of these tests. */
export const HOME = '/home/u';
export const KIB = 1024;
export const MIB = 1024 * KIB;
export const GIB = 1024 * MIB;
export const NOW = 1_750_000_000;
export const DAY = 86_400;

/** The context of a question about a hand-made tree: the lookups the mock advisor gives, and a guard like its own. */
export function sketchContext(sketch: Sketch, extra: Partial<ScanQuestionContext> = {}): ScanQuestionContext {
  const tree = sketchTree(sketch);
  const absolute = (path: string): string =>
    path === '~' ? HOME : path.startsWith('~/') ? `${HOME}${path.slice(1)}` : path;
  const bySize = (a: MockNode, b: MockNode): number => b.allocated - a.allocated || a.id - b.id;

  /** The node an absolute or `~` path names, whatever it is. */
  const entryAt = (path: string): MockNode | null => {
    const full = absolute(path);
    // A scan that started below `/` holds only what lies below it.
    const inner =
      tree.rootPath === '/'
        ? full
        : full === tree.rootPath
          ? '/'
          : full.startsWith(`${tree.rootPath}/`)
            ? full.slice(tree.rootPath.length)
            : null;

    try {
      return inner === null ? null : nodeAt(tree, inner);
    } catch {
      return null;
    }
  };

  return {
    scan: 1,
    tree,
    user: [],
    labels: [],
    outdated: false,
    units: 'GB',
    lookup: {
      exact: (path) => {
        const node = entryAt(path);

        return node?.kind === 'dir' ? node : null;
      },
      entry: (path) => {
        const node = entryAt(path);

        return node?.kind === 'dir' || node?.kind === 'file' ? node : null;
      },
      search: (text, limit) => {
        const needle = text.toLowerCase();
        const folders = tree.nodes.filter((node) => node.kind === 'dir' && node.parent >= 0);
        const prefix = folders.filter((node) => node.name.toLowerCase().startsWith(needle)).sort(bySize);
        const within = folders
          .filter(
            (node) => !node.name.toLowerCase().startsWith(needle) && node.name.toLowerCase().includes(needle),
          )
          .sort(bySize);

        return [...prefix, ...within].slice(0, limit);
      },
      searchEntries: (text, limit) => {
        const needle = text.toLowerCase();
        const entries = tree.nodes.filter(
          (node) => (node.kind === 'dir' || node.kind === 'file') && node.parent >= 0,
        );
        const nameOf = (node: MockNode): string => node.name.toLowerCase();
        const exact = entries.filter((node) => nameOf(node) === needle).sort(bySize);
        const prefix = entries
          .filter((node) => nameOf(node) !== needle && nameOf(node).startsWith(needle))
          .sort(bySize);
        const within = entries
          .filter((node) => !nameOf(node).startsWith(needle) && nameOf(node).includes(needle))
          .sort(bySize);

        return [...exact, ...prefix, ...within].slice(0, limit);
      },
    },
    guard: (node) => {
      if (node.parent < 0) {
        return 'This is the folder that was scanned, so Platter never suggests cleaning it.';
      }

      if (node.unreadable) {
        return 'Platter could not read this folder, so it cannot tell what is in it.';
      }

      return protection(folderPathOf(tree, node), HOME) === null
        ? null
        : 'This is a protected place, or inside one. Platter does not clean it.';
    },
    summary: () => {
      throw new Error('not asked');
    },
    ...extra,
  };
}

/** Asks a question about a sketch, as the words alone read it. */
export function askSketch(sketch: Sketch, question: string, pick?: number): ScanAnswerDto {
  return answerQuestion(sketchContext(sketch), question, pick);
}

/** The paths of the rows of an answer, in order. */
export const pathsOf = (answer: ScanAnswerDto): string[] => answer.rows.map((row) => row.path);
