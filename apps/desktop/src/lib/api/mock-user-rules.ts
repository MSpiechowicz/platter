import { ApiRequestError, type Api } from './client';
import { protection } from './mock-advisor';
import { folderPathOf, homeOf, shortenHome, type UserRuleInput } from './mock-suggestions';
import type { MockTree } from './mock-tree';
import type { ScanId, UserRulesDto } from './types';

/*
 * The mock's stand-in for the user-rule commands of `src-tauri/src/commands/rules.rs` and `user_rules.rs`
 * (docs/architecture.md, "Suggestions" and "Files on disk"): the list, add, remove and reset, and what the rest of
 * the mock needs to know about the rules (`view`).
 *
 * Still different from the backend:
 * - The rules live in memory, not in a file: nothing is read again, so a file edited by hand or one that breaks is
 *   not noticed. A broken file exists only as the `broken` option, and `resetUserRules` is the only way out of it.
 * - `list_user_rules` has no scan, so the display path uses the demo home folder (`/home/demo`) for `~`.
 * - The protected places for `safe` are the mock advisor's hand-copied list, and the refusal does not name the place.
 * - A folder whose name is not valid text cannot happen: the mock's names are plain.
 */

/** The most rules the list holds. */
export const MAX_USER_RULES = 256;

/** The home folder the rule list shows as `~`. */
export const MOCK_HOME = '/home/demo';

const DEFAULT_PROBLEM = 'The rules file is not valid: expected value at line 1 column 1.';

/**
 * The rules the `rules` URL hook starts with: exact folders of the demo disk, `keep` and `safe`. Some are folders the
 * demo tree has and some are not (a rule does not need its folder to be in a scan), and there are enough of them to
 * fill the card and make its list scroll.
 */
export const DEMO_USER_RULES: readonly UserRuleInput[] = [
  { verdict: 'keep', folder: `${MOCK_HOME}/Documents` },
  { verdict: 'safe', folder: `${MOCK_HOME}/.local/share/containers` },
  { verdict: 'keep', folder: `${MOCK_HOME}/Videos/recordings` },
  { verdict: 'keep', folder: `${MOCK_HOME}/Projekty/platter` },
  { verdict: 'safe', folder: `${MOCK_HOME}/.cache/thumbnails` },
  { verdict: 'keep', folder: `${MOCK_HOME}/Pictures` },
  { verdict: 'keep', folder: `${MOCK_HOME}/Music` },
  { verdict: 'safe', folder: `${MOCK_HOME}/Projekty/ocpp-bridge/target` },
  { verdict: 'keep', folder: `${MOCK_HOME}/Projekty/web-shop` },
  { verdict: 'keep', folder: `${MOCK_HOME}/Videos/films` },
  { verdict: 'safe', folder: `${MOCK_HOME}/Downloads/installers` },
  { verdict: 'keep', folder: `${MOCK_HOME}/.var/app/com.valvesoftware.Steam` },
  { verdict: 'keep', folder: `${MOCK_HOME}/Projekty/blog` },
  { verdict: 'safe', folder: `${MOCK_HOME}/Projekty/rust-playground/.venv` },
  { verdict: 'keep', folder: '/opt/android-sdk' },
  { verdict: 'safe', folder: '/var/tmp/builds' },
];

export interface MockUserRulesOptions {
  /** The rules the list starts with, in this order. */
  rules?: readonly UserRuleInput[];
  /** The rules file cannot be used; a text says why (a default one for `true`). */
  broken?: boolean | string;
}

/** The rules as the rest of the mock sees them: none while the file is broken, as in the backend. */
export interface UserRulesView {
  /** Counts the changes of the rules; starts at 0. */
  generation: number;
  rules: readonly UserRuleInput[];
  problem?: string;
}

export interface MockUserRulesHost {
  /** The tree of a scan; rejects with `notFound` for an unknown scan. */
  treeOf(scan: ScanId): MockTree;
  /** The scan has not finished. */
  isRunning(scan: ScanId): boolean;
  /** A command changed (or tried to change) the rules: the cleanup plans that were waiting are dropped. */
  changed(): void;
}

export type MockUserRulesApi = Pick<
  Api,
  'listUserRules' | 'addUserRule' | 'removeUserRule' | 'resetUserRules'
>;

export type MockUserRules = MockUserRulesApi & {
  /** The rules now, for suggestions, plans and the advisor. */
  view(): UserRulesView;
};

interface Stored extends UserRuleInput {
  id: number;
}

const fail = (code: 'notFound' | 'invalid', message: string): never => {
  throw new ApiRequestError(code, message);
};

/** `safe` is refused for the home folder, the scan root and every protected place. */
function checkSafeAllowed(tree: MockTree, folder: string, isScanRoot: boolean): void {
  const home = homeOf(tree);

  if (isScanRoot) {
    fail(
      'invalid',
      'this is the folder that was scanned; Platter never cleans it, so it cannot be marked safe',
    );
  }

  if (home === folder) {
    fail('invalid', 'this is your home folder; Platter never cleans it, so it cannot be marked safe');
  }

  if (home === null) {
    fail('invalid', 'Platter does not know your home folder, so it cannot tell which places are protected');
  }

  if (protection(folder, home) !== null) {
    fail(
      'invalid',
      'this is, holds or lies inside a protected place; Platter never cleans it, so it cannot be marked safe',
    );
  }
}

export function createMockUserRules(
  host: MockUserRulesHost,
  options: MockUserRulesOptions = {},
): MockUserRules {
  let rules: Stored[] = (options.rules ?? []).map((rule, index) => ({ ...rule, id: index + 1 }));
  let nextId = rules.length + 1;
  let generation = 0;
  let problem: string | undefined =
    options.broken === undefined || options.broken === false
      ? undefined
      : options.broken === true
        ? DEFAULT_PROBLEM
        : options.broken;

  if (problem !== undefined) {
    rules = [];
  }

  const dto = (): UserRulesDto => ({
    rules: rules.map((rule) => ({
      id: rule.id,
      path: shortenHome(rule.folder, MOCK_HOME),
      verdict: rule.verdict,
    })),
    ...(problem === undefined ? {} : { problem }),
  });

  /** `invalid` while the file is broken: with rules that cannot be read nothing may be planned or changed. */
  const usable = (): void => {
    if (problem !== undefined) {
      fail(
        'invalid',
        `Your rules file cannot be used, so Platter makes no cleanup plan: ${problem} Fix the file, or start over in the Advisor view.`,
      );
    }
  };

  const changed = (): UserRulesDto => {
    host.changed();

    return dto();
  };

  return {
    view: () => ({
      generation,
      rules: rules.map(({ verdict, folder }) => ({ verdict, folder })),
      ...(problem === undefined ? {} : { problem }),
    }),

    async listUserRules() {
      return dto();
    },

    async addUserRule(scan, node, verdict) {
      const tree = host.treeOf(scan);

      if (host.isRunning(scan)) {
        return fail('notFound', `scan ${scan} has not finished`);
      }

      const target = tree.nodes[node];

      if (!target) {
        return fail('notFound', `node ${node} does not exist`);
      }

      if (target.kind !== 'dir') {
        return fail('invalid', 'only a folder can have a rule');
      }

      const folder = folderPathOf(tree, target);

      if (folder === '/') {
        return fail('invalid', 'A folder in the rules file is not acceptable: / (the root folder)');
      }

      if (verdict === 'safe') {
        checkSafeAllowed(tree, folder, target.parent < 0);
      }

      usable();

      const existing = rules.find((rule) => rule.folder === folder);

      if (existing) {
        // A folder that has a rule gets the new verdict and keeps its place.
        if (existing.verdict !== verdict) {
          existing.verdict = verdict;
          generation++;
        }

        return changed();
      }

      if (rules.length >= MAX_USER_RULES) {
        return fail('invalid', `The rules file lists more than ${MAX_USER_RULES} rules.`);
      }

      rules = [...rules, { id: nextId++, verdict, folder }];
      generation++;

      return changed();
    },

    async removeUserRule(id) {
      usable();

      const kept = rules.filter((rule) => rule.id !== id);

      if (kept.length !== rules.length) {
        rules = kept;
        generation++;
      }

      return changed();
    },

    async resetUserRules() {
      if (problem === undefined) {
        return fail('invalid', 'The rules file is not broken.');
      }

      problem = undefined;
      rules = [];
      generation++;

      return changed();
    },
  };
}
