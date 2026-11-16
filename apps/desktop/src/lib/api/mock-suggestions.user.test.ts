import { describe, expect, it } from 'vitest';
import {
  advisorNodes,
  ruleView,
  suggestionsOf,
  type MockLabel,
  type UserRuleInput,
} from './mock-suggestions';
import { buildTree, childrenOf, type MockNode, type MockTree } from './mock-tree';

const NOW = 1_750_000_000;
const HOME = '/home/demo';

function treeOf(root: string): MockTree {
  return buildTree({ seed: 20240607, root, oneFileSystem: true, bytes: 186_000_000_000, now: NOW });
}

/** The node at a path below the scan root, by names. */
function at(tree: MockTree, path: string): MockNode {
  let node = tree.nodes[0] as MockNode;

  for (const name of path.split('/').filter((part) => part !== '')) {
    const next = childrenOf(tree, node).find((child) => child.name === name);

    expect(next, `${name} of ${path}`).toBeDefined();
    node = next as MockNode;
  }

  return node;
}

const keep = (folder: string): UserRuleInput => ({ verdict: 'keep', folder: `${HOME}/${folder}` });
const safe = (folder: string): UserRuleInput => ({ verdict: 'safe', folder: `${HOME}/${folder}` });

function listing(tree: MockTree, user: UserRuleInput[], labels: MockLabel[] = []) {
  return suggestionsOf(tree, labels, user).suggestions;
}

const idsOf = (tree: MockTree, user: UserRuleInput[]): string[] =>
  listing(tree, user).map((entry) => entry.id);

describe('the user rules in the mock suggestions', () => {
  const tree = treeOf(HOME);

  it('has no user suggestion without rules, and every suggestion says who decided', () => {
    const suggestions = listing(tree, []);

    expect(suggestions.some((entry) => entry.id.startsWith('user-'))).toBe(false);
    expect(suggestions.every((entry) => entry.source === 'rule')).toBe(true);
  });

  it('lists the folders of the user to keep as user-keep: verdict keep, source user, the exact folder', () => {
    const kept = listing(tree, [keep('Projekty/blog')]).find((entry) => entry.id === 'user-keep');

    expect(kept).toMatchObject({
      verdict: 'keep',
      source: 'user',
      title: 'Folders you keep',
      path: '~/Projekty/blog',
      items: 1,
      nodes: [at(tree, 'Projekty/blog').id],
      reason: 'Chosen by you: 1 folder to always keep. Nothing inside is suggested or cleaned.',
    });
  });

  it('lists the safe folders of the user as user-safe: verdict safe, counted in the safe total', () => {
    const user = [safe('Projekty/blog'), safe('Projekty/web-shop')];
    const { suggestions, totals } = suggestionsOf(tree, [], user);
    const listed = suggestions.find((entry) => entry.id === 'user-safe');
    const safeSum = suggestions
      .filter((entry) => entry.verdict === 'safe')
      .reduce((sum, entry) => sum + entry.allocated, 0);

    expect(listed).toMatchObject({ verdict: 'safe', source: 'user', items: 2, path: '2 folders you chose' });
    // The outermost match wins, so what a safe folder holds is counted once, in the folder.
    expect(totals.safe).toBe(safeSum);
  });

  describe('precedence', () => {
    // [what is checked, the user's rules, ids that must be listed, ids that must not be]
    const table: [string, UserRuleInput[], string[], string[]][] = [
      [
        'a keep of the user wins over a built-in review rule on the same folder',
        [keep('.cache')],
        ['user-keep'],
        ['app-cache'],
      ],
      [
        'a keep of the user wins over a built-in safe rule on the same folder',
        [keep('.npm')],
        ['user-keep'],
        ['npm-cache'],
      ],
      [
        'a built-in rule wins over a safe of the user on the same folder',
        [safe('.cache')],
        ['app-cache'],
        ['user-safe'],
      ],
      [
        'a built-in safe rule wins over a safe of the user on the same folder',
        [safe('.npm')],
        ['npm-cache'],
        ['user-safe'],
      ],
      [
        'a safe of the user stands on a folder no built-in rule has',
        [safe('Projekty/blog')],
        ['user-safe'],
        [],
      ],
      ['the outermost of two keeps wins', [keep('Projekty'), keep('Projekty/blog')], ['user-keep'], []],
      ['a built-in rule below a kept folder is not entered', [keep('Projekty/platter')], ['user-keep'], []],
    ];

    it.each(table)('%s', (_name, user, present, absent) => {
      const ids = idsOf(tree, user);

      for (const id of present) {
        expect(ids).toContain(id);
      }

      for (const id of absent) {
        expect(ids).not.toContain(id);
      }
    });

    it('counts a keep once: the outer folder of two keeps is the only match', () => {
      const kept = listing(tree, [keep('Projekty'), keep('Projekty/blog')]).find(
        (entry) => entry.id === 'user-keep',
      );

      expect(kept?.items).toBe(1);
      expect(kept?.nodes).toEqual([at(tree, 'Projekty').id]);
    });

    it('never enters a kept folder: the build output of a kept project is not matched', () => {
      const target = at(tree, 'Projekty/platter/target').id;
      const sibling = at(tree, 'Projekty/rust-playground/target').id;

      expect(ruleView(tree, []).matched.has(target)).toBe(true);
      expect(ruleView(tree, [keep('Projekty/platter')]).matched.has(target)).toBe(false);
      expect(ruleView(tree, [keep('Projekty/platter')]).matched.has(sibling)).toBe(true);
    });
  });

  describe('a folder that holds a kept folder', () => {
    it('is entered, not claimed as a whole: the built-in rule on ~/.cache gives way when a user keep is inside it', () => {
      const ids = idsOf(tree, [keep('.cache/google-chrome')]);

      expect(ids).toContain('user-keep');
      expect(ids).not.toContain('app-cache');
    });

    it('applies to a safe of the user too: it is not claimed while a kept folder lies inside it', () => {
      const ids = idsOf(tree, [safe('Projekty'), keep('Projekty/platter')]);

      expect(ids).toContain('user-keep');
      expect(ids).not.toContain('user-safe');
      // What is inside and not kept is still looked at on its own.
      expect(
        listing(tree, [safe('Projekty'), keep('Projekty/platter')]).find((e) => e.id === 'rust-target')
          ?.nodes,
      ).toContain(at(tree, 'Projekty/rust-playground/target').id);
    });

    it('holds for a built-in keep too: a safe of the user on the home folder is not claimed around ~/Documents', () => {
      expect(idsOf(tree, [{ verdict: 'safe', folder: HOME }])).not.toContain('user-safe');
    });

    it('does not apply to a kept folder the scan does not have', () => {
      expect(idsOf(tree, [keep('.cache/no-such-folder')])).toContain('app-cache');
    });
  });

  it('suggests nothing in a scan that starts inside a folder the user keeps', () => {
    const inside = treeOf(`${HOME}/Projekty/blog`);

    expect(suggestionsOf(inside, [], [keep('Projekty')])).toEqual({
      suggestions: [],
      totals: { safe: 0, review: 0, keep: 0 },
    });
  });

  it('keeps a model label out of the advisor group when a rule of the user covers or holds its folder', () => {
    const label = (path: string): MockLabel => ({
      node: at(tree, path).id,
      verdict: 'review',
      reason: 'A reason.',
    });
    const labels = [label('Projekty/blog/src'), label('Projekty/web-shop/src')];

    expect(advisorNodes(tree, labels, []).map((node) => node.id)).toHaveLength(2);
    expect(advisorNodes(tree, labels, [keep('Projekty/blog')]).map((node) => node.id)).toEqual([
      at(tree, 'Projekty/web-shop/src').id,
    ]);
    expect(advisorNodes(tree, labels, [safe('Projekty/web-shop/src')]).map((node) => node.id)).toEqual([
      at(tree, 'Projekty/blog/src').id,
    ]);
  });

  it('answers who covers a folder: the user for its own rule, the built-in table otherwise', () => {
    const view = ruleView(tree, [keep('Projekty/blog')]);

    expect(view.covering(at(tree, 'Projekty/blog/src'))).toMatchObject({
      id: 'user-keep',
      verdict: 'keep',
      source: 'user',
    });
    expect(view.covering(at(tree, '.cache/pip'))).toMatchObject({ id: 'app-cache', source: 'rule' });
  });
});
