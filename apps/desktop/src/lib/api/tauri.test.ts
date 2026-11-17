import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({ invoke, Channel: vi.fn() }));

import { isApiError } from './client';
import { createMockApi } from './mock';
import { createTauriApi } from './tauri';

describe('the Tauri history commands', () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(null);
  });

  it('asks for the overview by the number of days, and restores a cleanup by its id', async () => {
    const overview = { days: 90, samples: [] };
    const restored = { restored: 2, items: [] };

    invoke.mockResolvedValueOnce(overview).mockResolvedValueOnce(restored);

    const api = createTauriApi();

    expect(await api.historyOverview(90)).toBe(overview);
    expect(await api.restoreHistoryCleanup(5)).toBe(restored);
    expect(invoke.mock.calls).toEqual([
      ['history_overview', { days: 90 }],
      ['restore_history_cleanup', { cleanup: 5 }],
    ]);
  });

  it('turns a refusal into an API error with its code', async () => {
    invoke.mockRejectedValueOnce({ code: 'invalid', message: 'a cleanup is running' });

    await expect(createTauriApi().restoreHistoryCleanup(5)).rejects.toMatchObject({
      code: 'invalid',
      message: 'a cleanup is running',
    });
  });
});

describe('the Tauri update commands', () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(null);
  });

  it('checks for an update and restarts the app', async () => {
    const answer = { current: '0.1.0', update: null };

    invoke.mockResolvedValueOnce(answer);

    const api = createTauriApi();

    expect(await api.checkUpdate()).toBe(answer);

    await api.restartApp();

    expect(invoke.mock.calls).toEqual([
      ['check_update', undefined],
      ['restart_app', undefined],
    ]);
  });

  it('hands the progress channel of an install to the caller, and resolves when it is installed', async () => {
    const seen: unknown[] = [];

    invoke.mockImplementationOnce(
      async (_command: string, args: { onProgress: { onmessage: (m: unknown) => void } }) => {
        args.onProgress.onmessage({ kind: 'downloading', received: 5, total: null });
        args.onProgress.onmessage({ kind: 'installing' });
      },
    );

    await createTauriApi().installUpdate((progress) => seen.push(progress));

    expect(invoke.mock.calls[0]?.[0]).toBe('install_update');
    expect(seen).toEqual([{ kind: 'downloading', received: 5, total: null }, { kind: 'installing' }]);
  });

  it('turns the refusal to install into an API error', async () => {
    invoke.mockRejectedValueOnce({ code: 'invalid', message: 'There is no update to install.' });

    await expect(createTauriApi().installUpdate(() => {})).rejects.toMatchObject({ code: 'invalid' });
  });

  it('opens the release page by its name', async () => {
    await createTauriApi().openExternal('releases');

    expect(invoke).toHaveBeenCalledWith('open_external', { link: 'releases' });
  });
});

describe('the Tauri advisor commands', () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(null);
  });

  it('starts and cancels labelling by scan id only', async () => {
    const api = createTauriApi();

    await api.labelFolders(7);
    await api.cancelLabelling();

    expect(invoke.mock.calls).toEqual([
      ['label_folders', { scan: 7 }],
      ['cancel_labelling', undefined],
    ]);
  });

  it('reads the labels and asks about a folder by node id', async () => {
    const labels = { scan: 7, labelled: 0 };
    const answer = { node: 12, source: 'guard' };

    invoke.mockResolvedValueOnce(labels).mockResolvedValueOnce(answer);

    const api = createTauriApi();

    expect(await api.getAdvisorLabels(7)).toBe(labels);
    expect(await api.askAdvisor(7, 12)).toBe(answer);
    expect(invoke.mock.calls).toEqual([
      ['get_advisor_labels', { scan: 7 }],
      ['ask_advisor', { scan: 7, node: 12 }],
    ]);
  });

  it('sends the units with the question about a folder, and nothing else', async () => {
    const api = createTauriApi();

    await api.askAdvisor(7, 12, 'GiB');
    await api.askAdvisor(7, 12, 'GB');

    expect(invoke.mock.calls).toEqual([
      ['ask_advisor', { scan: 7, node: 12, units: 'GiB' }],
      ['ask_advisor', { scan: 7, node: 12, units: 'GB' }],
    ]);
  });

  it('asks a question with the scan id, the text and the units, and nothing else', async () => {
    const answer = { scan: 7, answer: 'text', rows: [] };

    invoke.mockResolvedValueOnce(answer);

    expect(await createTauriApi().askScan(7, 'what is big in ~/Projekty?', 'GiB')).toBe(answer);
    expect(invoke.mock.calls).toEqual([
      ['ask_scan', { scan: 7, question: 'what is big in ~/Projekty?', units: 'GiB' }],
    ]);
  });

  it('sends the id of a choice as `pick` with the same question, and leaves it out without one', async () => {
    invoke.mockResolvedValue({ scan: 7, answer: 'text', rows: [] });

    const api = createTauriApi();

    await api.askScan(7, 'can I delete the backup folder?', 'GB', 1);
    await api.askScan(7, 'can I delete the backup folder?', 'GB', 0);
    await api.askScan(7, 'can I delete the backup folder?', 'GB', undefined);

    expect(invoke.mock.calls).toEqual([
      ['ask_scan', { scan: 7, question: 'can I delete the backup folder?', units: 'GB', pick: 1 }],
      ['ask_scan', { scan: 7, question: 'can I delete the backup folder?', units: 'GB', pick: 0 }],
      ['ask_scan', { scan: 7, question: 'can I delete the backup folder?', units: 'GB' }],
    ]);
    expect(Object.keys(invoke.mock.calls[2]?.[1] as object)).not.toContain('pick');
  });

  it('sends the advisor options, and no model', async () => {
    const options = { enabled: true, unloadAfter: 'fiveMinutes', logQuestions: true, useGpu: true } as const;

    await createTauriApi().configureAdvisor(options);

    expect(invoke.mock.calls).toEqual([['configure_advisor', { options }]]);
  });

  it('sends the log option with the advisor options, and clears the log without arguments', async () => {
    const options = { enabled: true, unloadAfter: 'fiveMinutes', logQuestions: false, useGpu: true } as const;
    const api = createTauriApi();

    await api.configureAdvisor(options);
    await api.clearQuestionLog();

    expect(invoke.mock.calls).toEqual([
      ['configure_advisor', { options }],
      ['clear_question_log', undefined],
    ]);
  });

  it('answers a graphics card problem by its id and the choice, and clears the log with no arguments', async () => {
    const status = { state: 'loaded' };

    invoke.mockResolvedValueOnce(status);

    const api = createTauriApi();

    expect(await api.answerGpuProblem(1_760_000_000_123, 'useCpu')).toBe(status);

    await api.answerGpuProblem(1_760_000_000_123, 'tryAgain');
    await api.clearGpuLog();

    expect(invoke.mock.calls).toEqual([
      ['answer_gpu_problem', { id: 1_760_000_000_123, choice: 'useCpu' }],
      ['answer_gpu_problem', { id: 1_760_000_000_123, choice: 'tryAgain' }],
      ['clear_gpu_log', {}],
    ]);
  });

  it('has every command the mock has', () => {
    expect(Object.keys(createTauriApi()).sort()).toEqual(Object.keys(createMockApi()).sort());
  });

  it('searches folders with the text and limit', async () => {
    const found = [{ node: 3, path: '/home/me', allocated: 10 }];

    invoke.mockResolvedValueOnce(found);

    expect(await createTauriApi().findFolders(7, '~/docs', 20)).toBe(found);
    expect(invoke).toHaveBeenCalledWith('find_folders', { scan: 7, query: '~/docs', limit: 20 });
  });

  it('searches folders and files with the text and limit', async () => {
    const found = [{ node: 3, path: '/home/me/old.iso', allocated: 10, kind: 'file' }];

    invoke.mockResolvedValueOnce(found);

    expect(await createTauriApi().findEntries(7, '~/Down', 8)).toBe(found);
    expect(invoke).toHaveBeenCalledWith('find_entries', { scan: 7, query: '~/Down', limit: 8 });
  });

  it('lists, adds, removes and resets rules by node id and rule id only', async () => {
    const list = { rules: [{ id: 1, path: '~/Videos', verdict: 'keep' }] };

    invoke.mockResolvedValue(list);

    const api = createTauriApi();

    expect(await api.listUserRules()).toBe(list);
    expect(await api.addUserRule(7, 12, 'safe')).toBe(list);
    await api.removeUserRule(1);
    await api.resetUserRules();

    expect(invoke.mock.calls).toEqual([
      ['list_user_rules', undefined],
      ['add_user_rule', { scan: 7, node: 12, verdict: 'safe' }],
      ['remove_user_rule', { id: 1 }],
      ['reset_user_rules', undefined],
    ]);
  });

  it('turns a rejection of the backend into an error with its code', async () => {
    invoke.mockRejectedValueOnce({ code: 'invalid', message: 'The scan is outdated.' });

    const error = await createTauriApi()
      .labelFolders(7)
      .then(
        () => null,
        (reason: unknown) => reason,
      );

    expect(isApiError(error) && [error.code, error.message]).toEqual(['invalid', 'The scan is outdated.']);
  });
});
