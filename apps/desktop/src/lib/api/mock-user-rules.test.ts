import { describe, expect, it } from 'vitest';
import type { Api } from './client';
import { isApiError } from './client';
import { createMockApi, type MockOptions } from './mock';
import { MAX_USER_RULES } from './mock-user-rules';
import type { NodeId, ScanEvent, ScanId, SuggestionDto, UserRulesDto } from './types';

const NOW = 1_750_000_000;
const FAST = { durationMs: 0, now: () => NOW };

function scan(api: Api, root = '/'): Promise<ScanId> {
  return new Promise((resolve, reject) => {
    api
      .startScan({ root, oneFileSystem: true }, (event: ScanEvent) => {
        if (event.type === 'finished') {
          resolve(event.scanId);
        } else if (event.type === 'failed' || event.type === 'cancelled') {
          reject(new Error(`scan ended with ${event.type}`));
        }
      })
      .catch(reject);
  });
}

/** The node id of a folder of the scan, found by its exact path. */
async function folder(api: Api, scanId: ScanId, path: string): Promise<NodeId> {
  const [match] = await api.findFolders(scanId, path, 1);

  expect(match?.path).toBe(path);

  return (match as { node: NodeId }).node;
}

async function refusal(promise: Promise<unknown>): Promise<{ code: string; message: string }> {
  try {
    await promise;
  } catch (error) {
    if (isApiError(error)) {
      return error;
    }

    throw error;
  }

  throw new Error('expected a rejection');
}

async function ready(options: MockOptions = {}, root = '/'): Promise<{ api: Api; id: ScanId }> {
  const api = createMockApi({ ...FAST, ...options });

  return { api, id: await scan(api, root) };
}

const pathsOf = (list: UserRulesDto): string[] => list.rules.map((rule) => rule.path);
const suggestion = (list: SuggestionDto[], id: string): SuggestionDto | undefined =>
  list.find((entry) => entry.id === id);

describe('the user rules: list, add, remove', () => {
  it('starts empty and adds a rule by node id, shown with ~ for the home folder, in the order made', async () => {
    const { api, id } = await ready();

    expect(await api.listUserRules()).toEqual({ rules: [] });

    const first = await api.addUserRule(id, await folder(api, id, '/home/demo/Videos'), 'keep');
    const second = await api.addUserRule(id, await folder(api, id, '/var/tmp'), 'safe');

    expect(first.rules).toEqual([{ id: 1, path: '~/Videos', verdict: 'keep' }]);
    expect(second.rules).toEqual([
      { id: 1, path: '~/Videos', verdict: 'keep' },
      { id: 2, path: '/var/tmp', verdict: 'safe' },
    ]);
    expect(await api.listUserRules()).toEqual(second);
  });

  it('gives a folder that already has a rule the new verdict, in the same place, without a second rule', async () => {
    const { api, id } = await ready();
    const downloads = await folder(api, id, '/home/demo/Downloads');

    await api.addUserRule(id, downloads, 'keep');
    await api.addUserRule(id, await folder(api, id, '/var/tmp'), 'keep');

    const changed = await api.addUserRule(id, downloads, 'safe');

    expect(changed.rules).toEqual([
      { id: 1, path: '~/Downloads', verdict: 'safe' },
      { id: 2, path: '/var/tmp', verdict: 'keep' },
    ]);
  });

  it('removes a rule by id, ignores an id that is not there, and never gives an id twice', async () => {
    const { api, id } = await ready();
    const videos = await folder(api, id, '/home/demo/Videos');

    await api.addUserRule(id, videos, 'keep');

    expect((await api.removeUserRule(99)).rules).toHaveLength(1);
    expect((await api.removeUserRule(1)).rules).toEqual([]);
    expect((await api.addUserRule(id, videos, 'keep')).rules).toEqual([
      { id: 2, path: '~/Videos', verdict: 'keep' },
    ]);
  });

  it('starts with the rules the option gives', async () => {
    const { api } = await ready({
      userRules: { rules: [{ verdict: 'keep', folder: '/home/demo/Documents' }] },
    });

    expect(pathsOf(await api.listUserRules())).toEqual(['~/Documents']);
  });
});

describe('the user rules: what is refused', () => {
  it('says notFound for an unknown scan, an unknown node and a scan that has not finished', async () => {
    const { api, id } = await ready();

    expect((await refusal(api.addUserRule(99, 1, 'keep'))).code).toBe('notFound');
    expect((await refusal(api.addUserRule(id, 9_999_999, 'keep'))).code).toBe('notFound');

    const running = createMockApi({ durationMs: 5000 });
    const started = await running.startScan({ root: '/', oneFileSystem: true }, () => undefined);

    expect((await refusal(running.addUserRule(started, 1, 'keep'))).code).toBe('notFound');
    await running.cancelScan(started);
  });

  it('says invalid for a file and for the root folder, for either verdict', async () => {
    const { api, id } = await ready();
    const { items } = await api.getChildren(
      id,
      await folder(api, id, '/home/demo/Downloads'),
      0,
      50,
      'sizeDesc',
    );
    const file = items.find((item) => item.kind === 'file');

    expect(file).toBeDefined();
    expect((await refusal(api.addUserRule(id, (file as { id: NodeId }).id, 'keep'))).code).toBe('invalid');
    expect((await refusal(api.addUserRule(id, 0, 'keep'))).message).toContain('the root folder');
    expect((await refusal(api.addUserRule(id, 0, 'safe'))).code).toBe('invalid');
    expect((await api.listUserRules()).rules).toEqual([]);
  });

  it('refuses safe for the home folder, the scan root and protected places, and says why', async () => {
    const { api, id } = await ready();

    expect((await refusal(api.addUserRule(id, await folder(api, id, '/home/demo'), 'safe'))).message).toMatch(
      /home folder/,
    );
    expect((await refusal(api.addUserRule(id, await folder(api, id, '/usr'), 'safe'))).message).toMatch(
      /protected place/,
    );
    expect(
      (await refusal(api.addUserRule(id, await folder(api, id, '/home/demo/Documents'), 'safe'))).message,
    ).toMatch(/protected place/);

    const inFolder = await ready({}, '/home/demo');

    expect((await refusal(inFolder.api.addUserRule(inFolder.id, 0, 'safe'))).message).toMatch(
      /folder that was scanned/,
    );
    expect((await api.listUserRules()).rules).toEqual([]);
  });

  it('lets keep through where safe is refused: the home folder and a protected place can be kept', async () => {
    const { api, id } = await ready();

    await api.addUserRule(id, await folder(api, id, '/home/demo'), 'keep');

    expect(
      (await api.addUserRule(id, await folder(api, id, '/usr'), 'keep')).rules.map((rule) => rule.path),
    ).toEqual(['~', '/usr']);
  });

  it('holds at most 256 rules: the 257th folder is invalid, a verdict change at the limit is not', async () => {
    const full = Array.from({ length: MAX_USER_RULES }, (_, index) => ({
      verdict: 'keep' as const,
      folder: `/srv/rule-${index}`,
    }));
    const { api, id } = await ready({ userRules: { rules: full } });
    const downloads = await folder(api, id, '/home/demo/Downloads');

    expect((await api.listUserRules()).rules).toHaveLength(MAX_USER_RULES);
    expect((await refusal(api.addUserRule(id, downloads, 'keep'))).code).toBe('invalid');

    await api.removeUserRule(1);

    expect((await api.addUserRule(id, downloads, 'keep')).rules).toHaveLength(MAX_USER_RULES);
    expect((await api.addUserRule(id, downloads, 'safe')).rules.at(-1)).toMatchObject({ verdict: 'safe' });
  });
});

describe('the user rules: a rules file that cannot be used', () => {
  const broken = { userRules: { broken: 'The rules file is not valid: expected value at line 1 column 1.' } };

  it('lists no rules and the problem, and refuses to add or remove', async () => {
    const { api, id } = await ready(broken);

    expect(await api.listUserRules()).toEqual({
      rules: [],
      problem: 'The rules file is not valid: expected value at line 1 column 1.',
    });

    const added = await refusal(api.addUserRule(id, await folder(api, id, '/home/demo/Videos'), 'keep'));

    expect(added.code).toBe('invalid');
    expect(added.message).toContain('Your rules file cannot be used');
    expect((await refusal(api.removeUserRule(1))).code).toBe('invalid');
  });

  it('refuses every cleanup plan, before anything else is looked at, and still shows the built-in suggestions', async () => {
    const { api, id } = await ready(broken);
    const { suggestions } = await api.getSuggestions(id);

    expect(suggestion(suggestions, 'rust-target')).toBeDefined();

    for (const plan of [
      api.planCleanup(id, ['rust-target']),
      api.planCleanup(id, []),
      api.planCleanup(id, ['no-such-rule']),
      api.planMarkedCleanup(id, [await folder(api, id, '/var/tmp')]),
      api.planMarkedCleanup(id, []),
    ]) {
      const error = await refusal(plan);

      expect(error.code).toBe('invalid');
      expect(error.message).toContain('Your rules file cannot be used, so Platter makes no cleanup plan');
      expect(error.message).toContain('start over in the Advisor view');
    }
  });

  it('can be reset, once: the list is empty, plans work again, and a file that is not broken is invalid', async () => {
    const { api, id } = await ready(broken);

    expect(await api.resetUserRules()).toEqual({ rules: [] });
    expect((await api.planCleanup(id, ['rust-target'])).groups).toHaveLength(1);
    expect((await refusal(api.resetUserRules())).code).toBe('invalid');
  });

  it('cannot be reset while it is not broken, even with rules in it', async () => {
    const { api } = await ready({ userRules: { rules: [{ verdict: 'keep', folder: '/srv/data' }] } });

    expect((await refusal(api.resetUserRules())).message).toBe('The rules file is not broken.');
    expect((await api.listUserRules()).rules).toHaveLength(1);
  });
});

describe('the user rules and cleanup plans', () => {
  it('drops a plan that was waiting when a rule changes', async () => {
    const { api, id } = await ready();
    const plan = await api.planCleanup(id, ['rust-target']);

    await api.addUserRule(id, await folder(api, id, '/home/demo/Videos'), 'keep');

    expect((await refusal(api.startCleanup(plan.planId, () => undefined))).code).toBe('notFound');
  });

  it('plans a safe folder of the user as group user-safe: verdict safe, action trash, never a delete', async () => {
    const { api, id } = await ready();
    const target = await folder(api, id, '/home/demo/.local/share/containers');

    await api.addUserRule(id, target, 'safe');

    const { suggestions, totals } = await api.getSuggestions(id);
    const listed = suggestion(suggestions, 'user-safe');

    expect(listed).toMatchObject({
      verdict: 'safe',
      source: 'user',
      items: 1,
      path: '~/.local/share/containers',
    });
    expect(totals.safe).toBeGreaterThanOrEqual(listed?.allocated ?? Infinity);

    const plan = await api.planCleanup(id, ['user-safe']);

    expect(plan.groups).toHaveLength(1);
    expect(plan.groups[0]).toMatchObject({
      ruleId: 'user-safe',
      verdict: 'safe',
      action: 'trash',
      items: 1,
      needsElevation: false,
      samplePaths: ['/home/demo/.local/share/containers'],
    });
    expect(plan.groups[0]).not.toHaveProperty('command');
  });

  it('never plans a kept folder: user-keep is a known rule whose folders come back refused', async () => {
    const { api, id } = await ready();

    await api.addUserRule(id, await folder(api, id, '/home/demo/Videos'), 'keep');

    const plan = await api.planCleanup(id, ['user-keep']);

    expect(plan.groups).toEqual([]);
    expect(plan.refused).toEqual([
      { ruleId: 'user-keep', path: '/home/demo/Videos', reason: expect.any(String) as string },
    ]);
  });

  it('refuses a marked folder inside a kept folder of the user', async () => {
    const { api, id } = await ready();

    await api.addUserRule(id, await folder(api, id, '/home/demo/Videos'), 'keep');

    const plan = await api.planMarkedCleanup(id, [await folder(api, id, '/home/demo/Videos/films')]);

    expect(plan.groups).toEqual([]);
    expect(plan.refused.map((entry) => entry.path)).toEqual(['/home/demo/Videos/films']);
  });
});
