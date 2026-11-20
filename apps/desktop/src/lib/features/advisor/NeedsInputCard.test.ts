import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import { ApiRequestError, type Api } from '../../api/client';
import type { AdvisorLabelsDto, AdvisorStatusDto, UserRulesDto } from '../../api/types';
import { AdvisorStore } from '../../state/advisor.svelte';
import { RulesStore } from '../../state/rules.svelte';
import { SettingsStore } from '../../state/settings.svelte';
import { label, labelsOf, statusOf } from './fixtures';
import NeedsInputCard from './NeedsInputCard.svelte';

function setup(
  extra: {
    scan?: { id: number; outdated: boolean } | undefined;
    labels?: AdvisorLabelsDto | undefined;
    status?: AdvisorStatusDto | undefined;
    loading?: boolean;
    enabled?: boolean;
    addUserRule?: Api['addUserRule'];
  } = {},
) {
  const api = {
    labelFolders: vi.fn().mockResolvedValue(undefined),
    cancelLabelling: vi.fn().mockResolvedValue(undefined),
    addUserRule: extra.addUserRule ?? vi.fn().mockResolvedValue({ rules: [] } satisfies UserRulesDto),
  } as unknown as Api;
  const settings = new SettingsStore(null);

  settings.set('advisorEnabled', extra.enabled ?? true);

  const store = new AdvisorStore(api, settings);
  const rulesChanged = vi.fn();
  const rules = new RulesStore(api, rulesChanged);
  const onretry = vi.fn();
  const props = {
    scan: 'scan' in extra ? extra.scan : { id: 1, outdated: false },
    labels: 'labels' in extra ? extra.labels : labelsOf(),
    loading: extra.loading ?? false,
    status: 'status' in extra ? extra.status : statusOf('loaded'),
    store,
    rules,
    onretry,
  };

  const view = render(NeedsInputCard, { props });

  return { ...view, api, store, rules, rulesChanged, onretry, props };
}

describe('NeedsInputCard', () => {
  it('asks for a scan first when there is none', () => {
    setup({ scan: undefined, labels: undefined });

    expect(screen.getByText('Scan a disk or folder first.')).toBeTruthy();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('invites to label the folders no rule covers, and starts a labelling of this scan', async () => {
    const { api } = setup({ labels: labelsOf({ candidates: 6 }) });

    expect(screen.getByText(/^6 folders no rule covers can be labelled/)).toBeTruthy();
    expect(screen.queryByText(/folders?$/, { selector: '.hint' })).toBeNull();

    await fireEvent.click(screen.getByRole('button', { name: 'Label folders' }));

    expect(api.labelFolders).toHaveBeenCalledWith(1);
  });

  it('shows the progress "N of M folders" with Cancel, in place of the button', async () => {
    const { api } = setup({
      status: statusOf('loaded', { labelling: { scan: 1, done: 3, total: 6 } }),
    });

    expect(screen.getByText('Labelling folders: 3 of 6 folders')).toBeTruthy();
    expect(
      screen.getByRole('progressbar', { name: 'Labelling progress' }).getAttribute('aria-valuenow'),
    ).toBe('50');
    expect(screen.queryByRole('button', { name: 'Label folders' })).toBeNull();

    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(api.cancelLabelling).toHaveBeenCalledTimes(1);
  });

  it('shows no progress for a labelling of another scan', () => {
    setup({ status: statusOf('loaded', { labelling: { scan: 2, done: 3, total: 6 } }) });

    expect(screen.queryByText(/^Labelling folders/)).toBeNull();
  });

  it('lists what needs a look, largest first as given, with path, size and the reason as text', () => {
    setup({
      labels: labelsOf({
        labels: [
          label(1, 23, { path: '/home/demo/old-backups', reason: 'Named like a backup <b>old</b>.' }),
          label(2, 31, { verdict: 'keep' }),
          label(3, 11),
        ],
      }),
    });

    const items = [...document.querySelectorAll('.item')];

    expect(items.map((item) => item.querySelector('.path')?.textContent)).toEqual([
      '/home/demo/old-backups',
      '/home/demo/folder-3',
    ]);
    expect(items[0]?.textContent).toContain('23 GB');
    // The reason is text: a tag in it is not markup.
    expect(items[0]?.querySelector('small')?.textContent).toBe('Named like a backup <b>old</b>.');
    expect(items[0]?.querySelector('b')?.textContent).toBe('23 GB');
    expect(document.querySelector('.hint')?.textContent).toBe('2 folders');
  });

  it('offers Always keep and Safe to clean on each folder that needs a look, and nowhere else', () => {
    setup({ labels: labelsOf({ labels: [label(1, 23), label(2, 31, { verdict: 'keep' }), label(3, 11)] }) });

    expect(screen.getAllByRole('button', { name: /^Always keep / })).toHaveLength(2);
    expect(screen.getAllByRole('button', { name: /^Safe to clean / })).toHaveLength(2);
    expect(screen.queryByRole('button', { name: /folder-2/ })).toBeNull();
  });

  it('keeps a folder with one click: the rule is made for that label, and the row shows the decision', async () => {
    const { api, rulesChanged } = setup({ labels: labelsOf({ labels: [label(1, 23), label(3, 11)] }) });

    await fireEvent.click(screen.getByRole('button', { name: 'Always keep /home/demo/folder-1' }));

    expect(api.addUserRule).toHaveBeenCalledWith(1, 1, 'keep');
    await vi.waitFor(() => expect(rulesChanged).toHaveBeenCalledTimes(1));

    const [first, second] = [...document.querySelectorAll('.item')];

    expect(first?.querySelector('button')).toBeNull();
    expect(first?.querySelector('.tag')?.textContent).toBe('keep');
    expect(first?.textContent).toContain('Your rule: always keep.');
    expect(second?.querySelectorAll('button')).toHaveLength(2);
    expect(document.querySelector('.hint')?.textContent).toBe('1 folder');
  });

  it('marks a folder safe to clean with one click', async () => {
    const { api } = setup({ labels: labelsOf({ labels: [label(4, 9)] }) });

    await fireEvent.click(screen.getByRole('button', { name: 'Safe to clean /home/demo/folder-4' }));

    expect(api.addUserRule).toHaveBeenCalledWith(1, 4, 'safe');
    await vi.waitFor(() => expect(document.querySelector('.item .tag')?.textContent).toBe('safe'));
    expect(screen.getByText('Your rule: safe to clean.')).toBeTruthy();
    expect(document.querySelector('.hint')).toBeNull();
  });

  it('shows the refusal on that row only, as text, and leaves its buttons for another try', async () => {
    const refusal =
      'this is, holds or lies inside a protected place. Platter never cleans it, so it cannot be marked safe';
    const alert = vi.spyOn(window, 'alert').mockImplementation(() => undefined);
    const confirm = vi.spyOn(window, 'confirm').mockImplementation(() => true);

    setup({
      labels: labelsOf({ labels: [label(1, 23), label(3, 11)] }),
      addUserRule: vi.fn().mockRejectedValue(new ApiRequestError('invalid', refusal)),
    });

    await fireEvent.click(screen.getByRole('button', { name: 'Safe to clean /home/demo/folder-1' }));

    await screen.findByRole('alert');

    const [first, second] = [...document.querySelectorAll('.item')];

    expect(first?.querySelector('[role="alert"]')?.textContent).toBe(refusal);
    expect(second?.querySelector('[role="alert"]')).toBeNull();
    expect(first?.querySelectorAll('button')).toHaveLength(2);
    expect(first?.querySelector('.tag')).toBeNull();
    expect(document.querySelector('.hint')?.textContent).toBe('2 folders');
    expect(alert).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();

    alert.mockRestore();
    confirm.mockRestore();
  });

  it('clears the refusal when the next try works', async () => {
    const addUserRule = vi
      .fn()
      .mockRejectedValueOnce(new ApiRequestError('invalid', 'No.'))
      .mockResolvedValueOnce({ rules: [] });

    setup({ labels: labelsOf({ labels: [label(1, 23)] }), addUserRule });

    await fireEvent.click(screen.getByRole('button', { name: 'Safe to clean /home/demo/folder-1' }));
    expect((await screen.findByRole('alert')).textContent).toBe('No.');

    await fireEvent.click(screen.getByRole('button', { name: 'Always keep /home/demo/folder-1' }));
    await vi.waitFor(() => expect(document.querySelector('.item .tag')?.textContent).toBe('keep'));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows a decision the user made earlier from the start, without buttons', () => {
    setup({
      labels: labelsOf({
        labels: [label(1, 23, { decided: 'keep' }), label(3, 11, { decided: 'safe' }), label(5, 2)],
      }),
    });

    const items = [...document.querySelectorAll('.item')];

    expect(items.map((item) => item.querySelector('.tag')?.textContent ?? null)).toEqual([
      'keep',
      'safe',
      null,
    ]);
    expect(items.map((item) => item.querySelectorAll('button').length)).toEqual([0, 0, 2]);
    expect(document.querySelector('.hint')?.textContent).toBe('1 folder');
  });

  it('brings the buttons back when the labels say the rule is gone', async () => {
    const { rerender, props } = setup({ labels: labelsOf({ labels: [label(1, 23, { decided: 'keep' })] }) });

    expect(screen.queryByRole('button', { name: /^Always keep/ })).toBeNull();

    await rerender({ ...props, labels: labelsOf({ labels: [label(1, 23)] }) });

    expect(screen.getByRole('button', { name: /^Always keep/ })).toBeTruthy();
  });

  it('says when the model looked at folders and found nothing', () => {
    setup({ labels: labelsOf({ labels: [label(1, 5, { verdict: 'keep' })] }) });

    expect(
      screen.getByText('The model looked at 1 folder and found nothing that needs your input.'),
    ).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Label again' })).toBeTruthy();
  });

  it('says a run stopped early, and offers to label again beside the list', async () => {
    const { api } = setup({
      labels: labelsOf({ labels: [label(1, 23)], stopped: 'timeBudget' }),
    });

    expect(screen.getByText('Stopped at the time limit, so some folders were not labelled.')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Label again' }));

    expect(api.labelFolders).toHaveBeenCalledWith(1);
  });

  it('keeps the button off and says why when the model cannot run', () => {
    setup({ status: statusOf('notDownloaded') });

    expect((screen.getByRole('button', { name: 'Label folders' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('Download the model first.')).toBeTruthy();
  });

  it('keeps the button off for an outdated scan, and when nothing is left to label', () => {
    setup({ scan: { id: 1, outdated: true } });
    expect((screen.getByRole('button', { name: 'Label folders' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('Labelling is paused until you rescan.')).toBeTruthy();
  });

  it('says when no folder is left for the model', () => {
    setup({ labels: labelsOf({ candidates: 0 }) });

    expect(screen.getByText('Nothing waiting. Folders the model is unsure about show up here.')).toBeTruthy();
    expect(screen.getByText('No folder is left for the model: the rules cover the rest.')).toBeTruthy();
  });

  it('offers a retry when the labels could not be read', async () => {
    const { onretry } = setup({ labels: undefined, loading: false });

    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    expect(onretry).toHaveBeenCalledTimes(1);
  });

  it('shows the refusal of a labelling as text', async () => {
    const { store } = setup();

    store.labelFailure = 'The model is busy with another job.';

    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(screen.getByRole('alert').textContent).toBe('The model is busy with another job.');
  });
});
