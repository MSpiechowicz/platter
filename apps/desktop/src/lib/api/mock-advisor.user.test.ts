import { describe, expect, it } from 'vitest';
import type { Api } from './client';
import { createMockApi, type MockOptions } from './mock';
import { FakeTimers } from './mock-advisor.testkit';
import type { AdvisorLabelsDto, NodeId, ScanEvent, ScanId } from './types';

const NOW = 1_750_000_000;
const LOAD_MS = 400;
const LABEL_MS = 300;

function scan(api: Api): Promise<ScanId> {
  return new Promise((resolve, reject) => {
    api
      .startScan({ root: '/', oneFileSystem: true }, (event: ScanEvent) => {
        if (event.type === 'finished') {
          resolve(event.scanId);
        }
      })
      .catch(reject);
  });
}

async function ready(options: MockOptions = {}): Promise<{ api: Api; id: ScanId; timers: FakeTimers }> {
  const timers = new FakeTimers();
  const api = createMockApi({
    durationMs: 0,
    now: () => NOW,
    advisor: { timers, state: 'loaded' },
    ...options,
  });

  return { api, timers, id: await scan(api) };
}

async function folder(api: Api, id: ScanId, path: string): Promise<NodeId> {
  const [match] = await api.findFolders(id, path, 1);

  expect(match?.path).toBe(path);

  return (match as { node: NodeId }).node;
}

async function labelAll(api: Api, timers: FakeTimers, id: ScanId): Promise<AdvisorLabelsDto> {
  const { candidates } = await api.getAdvisorLabels(id);

  await api.labelFolders(id);
  timers.advance(LOAD_MS + LABEL_MS * (candidates + 1));

  return api.getAdvisorLabels(id);
}

describe('askAdvisor with the rules of the user', () => {
  it('answers with source user and its own words for a folder the user keeps, with the model not downloaded', async () => {
    const { api, id } = await ready({ advisor: { state: 'none' } });
    const node = await folder(api, id, '/home/demo/Projekty/blog');

    await api.addUserRule(id, node, 'keep');

    const answer = await api.askAdvisor(id, node);

    expect(answer).toMatchObject({
      verdict: 'keep',
      source: 'user',
      path: '/home/demo/Projekty/blog',
      reason: expect.stringContaining('always keep') as string,
    });
    expect(answer.detail).toMatch(/^Your rule: Folders you keep\. [\d.]+ [KMGT]?B\./);
  });

  it('answers a folder inside a kept folder with the rule that covers it', async () => {
    const { api, id } = await ready();

    await api.addUserRule(id, await folder(api, id, '/home/demo/Projekty/blog'), 'keep');

    expect(await api.askAdvisor(id, await folder(api, id, '/home/demo/Projekty/blog/src'))).toMatchObject({
      verdict: 'keep',
      source: 'user',
    });
  });

  it('answers safe to clean for a safe folder of the user', async () => {
    const { api, id } = await ready();
    const node = await folder(api, id, '/home/demo/.local/share/containers');

    await api.addUserRule(id, node, 'safe');

    const answer = await api.askAdvisor(id, node);

    expect(answer).toMatchObject({ verdict: 'safe', source: 'user' });
    expect(answer.detail).toMatch(/^Your rule: Folders you marked safe\. [\d.]+ [KMGT]?B\./);
  });

  it('lets the built-in rule answer where it wins over a safe of the user', async () => {
    const { api, id } = await ready();
    const node = await folder(api, id, '/home/demo/.npm');

    await api.addUserRule(id, node, 'safe');

    expect(await api.askAdvisor(id, node)).toMatchObject({ verdict: 'safe', source: 'rule' });
  });

  it('refuses to ask or label while the rules file is broken, so a kept folder never reaches the model', async () => {
    const { api, id } = await ready({
      userRules: { broken: true },
    });
    const node = await folder(api, id, '/home/demo/.npm');

    await expect(api.askAdvisor(id, node)).rejects.toMatchObject({ code: 'invalid' });
    await expect(api.labelFolders(id)).rejects.toMatchObject({ code: 'invalid' });
  });
});

describe('labels with the rules of the user', () => {
  it('says what the user decided about the folder of a label, and stops saying it when the rule goes', async () => {
    const { api, id, timers } = await ready();
    const labelled = await labelAll(api, timers, id);
    const target = labelled.labels.find((entry) => entry.verdict === 'review');

    expect(target).toBeDefined();
    expect(labelled.labels.every((entry) => entry.decided === undefined)).toBe(true);

    const rules = await api.addUserRule(id, (target as { node: NodeId }).node, 'safe');
    const decided = await api.getAdvisorLabels(id);

    expect(decided.labels.find((entry) => entry.node === target?.node)?.decided).toBe('safe');

    await api.removeUserRule(rules.rules[0]?.id as number);

    expect((await api.getAdvisorLabels(id)).labels.every((entry) => entry.decided === undefined)).toBe(true);
  });

  it('says keep when the folder has a keep rule, and counts the rule in the first step', async () => {
    const { api, id, timers } = await ready();
    const labelled = await labelAll(api, timers, id);
    const target = labelled.labels[0];

    await api.addUserRule(id, (target as { node: NodeId }).node, 'keep');

    const after = await api.getAdvisorLabels(id);

    expect(after.labels.find((entry) => entry.node === target?.node)?.decided).toBe('keep');
    expect(after.rules.folders).toBe(labelled.rules.folders + 1);
  });

  it('leaves a folder with a rule of the user out of the candidates', async () => {
    const { api, id, timers } = await ready();
    const before = await api.getAdvisorLabels(id);
    const labelled = await labelAll(api, timers, id);
    const target = labelled.labels[0] as { node: NodeId };

    await api.addUserRule(id, target.node, 'keep');

    expect((await api.getAdvisorLabels(id)).candidates).toBe(before.candidates - 1);
  });
});
